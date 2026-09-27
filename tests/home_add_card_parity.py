#!/usr/bin/env python3
"""Compare Home's add-card and board-filter behavior with Fizzy."""

import json
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import urllib.parse
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench"))
from draft_workflow_parity import login, request
from events_html_parity import target
from sustained_mixed import (ROOT, RUST, SOURCE, prepare_rust, prepare_source,
                             rust_env, source_env, start_server, stop_server)


def board(target_item, name):
    account = target_item["account"]
    status, _, raw = request(target_item, f"/{account}/boards", "POST",
                             {"board": {"name": name}}, bearer=True)
    assert status == 201, (target_item["name"], status, raw[:250])
    return json.loads(raw)["id"]


def add_card_action(body, account):
    forms = re.findall(rb'<form\b[^>]*\baction="([^"]+/boards/[^/]+/cards)"[^>]*>',
                       body)
    assert len(forms) == 1, (account, forms, body[:500])
    return urllib.parse.urlparse(forms[0].decode()).path


def check(target_item):
    login(target_item)
    account = target_item["account"]
    for path in (f"/{account}", f"/{account}/"):
        status, _, root = request(target_item, path, accept="text/html")
        assert status == 200 and b"Latest Activity" in root, (
            target_item["name"], "account Home path", path, status)
    status, headers, empty_home = request(target_item, f"/{account}/events",
                                          accept="text/html")
    assert status == 200 and b'Add a card' not in empty_home and b"all boards" in empty_home, (
        target_item["name"], "empty Home filter")
    empty_etag = headers["ETag"]
    first = board(target_item, "Home selection first")
    target_item["extra_headers"] = {"If-None-Match": empty_etag}
    status, _, refreshed = request(target_item, f"/{account}/events",
                                   accept="text/html")
    del target_item["extra_headers"]
    assert status == 200 and add_card_action(refreshed, account) == f"/{account}/boards/{first}/cards", (
        target_item["name"], "Home stayed stale after a board was created", status)
    assert b"Latest Activity" in refreshed and b"events__filter-select" not in refreshed, (
        target_item["name"], "single-board Home filter")
    second = board(target_item, "Home selection recent")
    with sqlite3.connect(target_item["db"]) as db:
        db.execute("UPDATE accesses SET accessed_at='2025-01-01T00:00:00.000Z'")
        db.execute("UPDATE accesses SET accessed_at='2026-01-01T00:00:00.000Z' "
                   "WHERE board_id=(SELECT id FROM boards WHERE name='Home selection first')")
        db.execute("UPDATE accesses SET accessed_at='2026-02-01T00:00:00.000Z' "
                   "WHERE board_id=(SELECT id FROM boards WHERE name='Home selection recent')")
    status, _, home = request(target_item, f"/{account}/events", accept="text/html")
    assert status == 200, (target_item["name"], status)
    assert b"Activity across" in home and b"all boards" in home and b"events__filter-select" in home, (
        target_item["name"], "multi-board Home filter")
    assert add_card_action(home, account) == f"/{account}/boards/{second}/cards", target_item["name"]
    selected = urllib.parse.urlencode({"board_ids[]": first})
    status, _, filtered = request(target_item, f"/{account}/events?{selected}",
                                  accept="text/html")
    assert status == 200, (target_item["name"], status)
    assert b"Activity in" in filtered and b"Home selection first" in filtered, (
        target_item["name"], "selected-board Home heading")
    action = add_card_action(filtered, account)
    assert action == f"/{account}/boards/{first}/cards", (target_item["name"], action)
    csrf = re.search(rb'<meta name="csrf-token" content="([^"]+)"', filtered)
    status, headers, body = request(target_item, action, "POST", b"",
                                    accept="text/html",
                                    csrf=csrf.group(1).decode() if csrf else None)
    path = urllib.parse.urlparse(headers.get("Location", "")).path
    assert status == 302 and re.fullmatch(rf"/{account}/cards/\d+/draft", path), (
        target_item["name"], status, path, body[:250])
    return {"recent_board": True, "selected_board": True, "draft_redirect": True}


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-home-add-card-") as temporary:
        processes = []
        checks = []
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
                checks.append(check(item))
            assert checks[0] == checks[1]
            print("PASS Home add-card, draft redirect, and board-filter headings")
        finally:
            for process in processes:
                stop_server(process)


if __name__ == "__main__":
    main()
