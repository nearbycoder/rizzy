#!/usr/bin/env python3
"""Check MySQL pin and notification SSE revisions across Rizzy processes."""

import json
import os
import subprocess
import sys
import tempfile
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from card_workflow import expect  # noqa: E402
from draft_workflow_parity import login  # noqa: E402
from events_html_parity import target as browser_target  # noqa: E402
from sustained_mixed import prepare_rust, start_server, stop_server  # noqa: E402
from sustained_mixed_mysql import (create_databases, drop_databases, mysql_query,  # noqa: E402
                                   rust_env)

MYSQL = "/tmp/rizzy-mysql8/bin/mysql"
SOCKET = "/tmp/rizzy-mysql8/mysql.sock"


def next_revision(stream):
    for _ in range(10):
        line = stream.readline()
        if not line:
            raise AssertionError("Live stream closed")
        if line.startswith(b"data: "):
            return json.loads(line[6:])
    raise AssertionError("No live revision event")


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    database = "rzlive_" + uuid.uuid4().hex[:12]
    create_databases(MYSQL, SOCKET, [database])
    processes = []
    try:
        with tempfile.TemporaryDirectory(prefix="rizzy-mysql-user-live-") as temporary:
            directory = Path(temporary)
            env = rust_env(directory, "127.0.0.1", 33080, database)
            env["RIZZY_DEV_MAGIC_LINKS"] = "1"
            env, credentials = prepare_rust(directory, env)
            urls = []
            for index in range(2):
                worker = directory / f"process-{index}"
                worker.mkdir()
                process, url = start_server("rizzy", worker, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                urls.append(url)
            target = {"url": urls[1], "account": str(credentials["account"]),
                      "token": credentials["token"]}
            board, _ = expect(target, "/boards", "POST", {"board": {"name": "MySQL live"}}, 201)
            card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                             {"card": {"title": "Live card"}}, 201)
            newer_card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                                   {"card": {"title": "Second live card"}}, 201)
            browser = browser_target("rizzy", urls[0], target["account"], target["token"],
                                     "owner@example.com", "")
            login(browser)
            request = urllib.request.Request(
                urls[0] + f"/{target['account']}/my/live",
                headers={"Accept": "text/event-stream"})
            with browser["opener"].open(request, timeout=12) as stream:
                initial = next_revision(stream)
                expect(target, f"/cards/{card['number']}/pin", "POST", status=204)
                pinned = next_revision(stream)
                assert pinned["pins"] != initial["pins"]
                expect(target, f"/cards/{newer_card['number']}/pin", "POST", status=204)
                pinned_second = next_revision(stream)
                assert pinned_second["pins"] != pinned["pins"]
                expect(target, f"/cards/{card['number']}", "PUT",
                       {"card": {"title": "Live card updated"}}, 200)
                edited = next_revision(stream)
                assert edited["pins"] != pinned_second["pins"]
                account_id, user_id = mysql_query(
                    MYSQL, SOCKET, database,
                    "SELECT a.id,u.id FROM accounts a JOIN users u ON u.account_id=a.id "
                    "WHERE u.role='owner' LIMIT 1").split("\t")
                event_id = mysql_query(
                    MYSQL, SOCKET, database,
                    f"SELECT id FROM events WHERE card_id='{card['id']}' LIMIT 1")
                notification_id = uuid.uuid4().hex
                mysql_query(
                    MYSQL, SOCKET, database,
                    "INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,unread_count,created_at,updated_at) "
                    f"VALUES('{notification_id}','{account_id}','{user_id}','{user_id}','{card['id']}','{event_id}','Event',1,NOW(6),NOW(6))")
                notified = next_revision(stream)
                assert notified["notifications"] != edited["notifications"]
            print("PASS MySQL cross-process pin, card-preview, and notification live revisions")
    finally:
        for process in reversed(processes):
            stop_server(process)
        drop_databases(MYSQL, SOCKET, [database])


if __name__ == "__main__":
    main()
