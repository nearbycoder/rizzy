#!/usr/bin/env python3
"""A failed title event or system comment must roll back the card edit."""

import http.cookiejar
import os
import subprocess
import sys
import tempfile
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from card_workflow import expect  # noqa: E402
from draft_workflow_parity import NoRedirect, request  # noqa: E402
from sustained_mixed import prepare_rust, start_server, stop_server  # noqa: E402
from sustained_mixed_mysql import (  # noqa: E402
    create_databases, drop_databases, mysql_query, rust_env,
)

CLIENT = os.environ.get("RIZZY_TEST_MYSQL_CLIENT", "/tmp/rizzy-mysql8/bin/mysql")
SOCKET = os.environ.get("RIZZY_TEST_MYSQL_SOCKET", "/tmp/rizzy-mysql8/mysql.sock")
PORT = int(os.environ.get("MYSQL_PORT", "33080"))


def main():
    if not Path(SOCKET).is_socket():
        raise SystemExit("Start the disposable MySQL server first")
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    database = "rztitle_" + uuid.uuid4().hex[:12]
    create_databases(CLIENT, SOCKET, [database])
    server = None
    with tempfile.TemporaryDirectory(prefix="rizzy-title-atomic-") as temp:
        directory = Path(temp)
        try:
            env, credentials = prepare_rust(
                directory, rust_env(directory, "127.0.0.1", PORT, database))
            server, url = start_server("rizzy", directory, env,
                                       sorted(os.sched_getaffinity(0))[:4])
            target = {"url": url, "account": str(credentials["account"]),
                      "token": credentials["token"],
                      "opener": urllib.request.build_opener(
                          urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                          NoRedirect())}
            board, _ = expect(target, "/boards", "POST",
                              {"board": {"name": "Atomic title changes"}}, 201)
            card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                             {"card": {"title": "Original title"}}, 201)
            card_id = card["id"]
            number = card["number"]
            path = f"/{target['account']}/cards/{number}"
            query = lambda statement: mysql_query(CLIENT, SOCKET, database, statement)

            def counts():
                return query(
                    "SELECT (SELECT title FROM cards WHERE id='" + card_id + "'), "
                    "(SELECT COUNT(*) FROM events WHERE card_id='" + card_id +
                    "' AND action='card_title_changed'), "
                    "(SELECT COUNT(*) FROM comments WHERE card_id='" + card_id + "')")

            assert counts() == "Original title\t0\t0", counts()
            for table, trigger in (("events", "fail_title_event"),
                                   ("comments", "fail_title_comment")):
                query(f"CREATE TRIGGER {trigger} BEFORE INSERT ON {table} FOR EACH ROW "
                      "SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='forced title write failure'")
                try:
                    status, _, _ = request(
                        target, path, "PATCH", {"card": {"title": "New title"}},
                        bearer=True)
                    assert status == 500, (table, status)
                    assert counts() == "Original title\t0\t0", (table, counts())
                finally:
                    query(f"DROP TRIGGER {trigger}")

            updated, _ = expect(target, f"/cards/{number}", "PATCH",
                                {"card": {"title": "New title"}}, 200)
            assert updated["title"] == "New title"
            assert counts() == "New title\t1\t1", counts()
            stamp_match = query(
                "SELECT e.created_at=m.created_at FROM events e JOIN comments m "
                f"ON m.card_id=e.card_id WHERE e.card_id='{card_id}' "
                "AND e.action='card_title_changed'")
            assert stamp_match == "1", stamp_match
            print("PASS: title, event, and system comment commit or roll back together")
        finally:
            if server is not None:
                stop_server(server)
            drop_databases(CLIENT, SOCKET, [database])


if __name__ == "__main__":
    main()
