#!/usr/bin/env python3
"""Compare rendered Home event sentences for Fizzy's timeline action set."""

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import urllib.parse
import uuid
from datetime import datetime, timezone
from html.parser import HTMLParser
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench"))
from activity_particulars_parity import fixture, stored_id
from draft_workflow_parity import login, request
from events_html_parity import target
from sustained_mixed import (ROOT, RUST, prepare_rust, prepare_source,
                             rust_env, source_env, start_server, stop_server)


class Descriptions(HTMLParser):
    def __init__(self):
        super().__init__()
        self.sentences = []
        self.depth = 0
        self.hidden = 0
        self.parts = []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "strong" and "event__title" in attrs.get("class", "").split():
            self.depth = 1
            self.parts = []
            return
        if self.depth:
            self.depth += 1
            if self.hidden:
                self.hidden += 1
            elif "data-only-visible-to-others" in attrs:
                self.hidden = 1

    def handle_endtag(self, tag):
        if self.depth:
            if self.hidden:
                self.hidden -= 1
            self.depth -= 1
            if not self.depth:
                self.sentences.append(" ".join("".join(self.parts).split()))

    def handle_data(self, data):
        if self.depth and not self.hidden:
            self.parts.append(data)


def descriptions(body):
    parsed = Descriptions()
    parsed.feed(body.decode())
    return parsed.sentences


def check(item):
    board, _ = fixture(item)
    with sqlite3.connect(item["db"]) as db:
        db.row_factory = sqlite3.Row
        published = db.execute("SELECT * FROM events WHERE action='card_published' "
                               "ORDER BY created_at DESC LIMIT 1").fetchone()
        cloned = dict(published)
        cloned["id"] = stored_id(item, uuid.uuid4().int)
        cloned["action"] = "card_collection_changed"
        cloned["particulars"] = json.dumps({"particulars": {"new_collection": "Review <Now>"}})
        cloned["created_at"] = "2024-01-01T00:00:12.000Z"
        cloned["updated_at"] = cloned["created_at"]
        columns = list(cloned)
        db.execute(f"INSERT INTO events ({','.join(columns)}) "
                   f"VALUES ({','.join('?' for _ in columns)})",
                   [cloned[column] for column in columns])
        if item["name"] == "fizzy":
            # The API fixture uses ISO strings for synthetic rows; Active
            # Record's SQLite day query compares its native datetime format.
            db.execute("UPDATE events SET created_at=replace(replace(created_at,'T',' '),'Z','') "
                       "WHERE created_at LIKE '2024-01-01T%'")
    login(item)
    account = item["account"]
    filter_query = urllib.parse.urlencode({"board_ids[]": board["id"]})
    result = {}
    for day, expected in (("2024-01-01", 12),
                          (datetime.now(timezone.utc).date().isoformat(), 2)):
        status, _, body = request(item, f"/{account}/events?day={day}&{filter_query}",
                                  accept="text/html")
        assert status == 200, (item["name"], day, status)
        result[day] = descriptions(body)
        assert len(result[day]) == expected, (item["name"], day, result[day], body[:1200])
    return result


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-timeline-descriptions-") as temporary:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env, credentials = prepare_source(directory, 2,
                                                      source_env(directory, 2))
                    database = directory / "db.sqlite3"
                else:
                    environment = rust_env(directory)
                    environment["RIZZY_DEV_MAGIC_LINKS"] = "1"
                    env, credentials = prepare_rust(directory, environment)
                    database = directory / "rizzy.sqlite3"
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                with sqlite3.connect(database) as db:
                    email = db.execute(
                        "SELECT i.email_address FROM accounts a JOIN users u "
                        "ON u.account_id=a.id JOIN identities i ON i.id=u.identity_id "
                        "WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
                        (credentials["account"],)).fetchone()[0]
                item = target(name, url, str(credentials["account"]),
                              credentials["token"], email, str(database))
                results[name] = check(item)
            assert results["fizzy"] == results["rizzy"], {
                day: (results["fizzy"][day], results["rizzy"][day])
                for day in results["fizzy"] if results["fizzy"][day] != results["rizzy"][day]
            }
            print("PASS 14 rendered Home event descriptions match Fizzy")
        finally:
            for process in processes:
                stop_server(process)


if __name__ == "__main__":
    main()
