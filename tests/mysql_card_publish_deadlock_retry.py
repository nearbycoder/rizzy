#!/usr/bin/env python3
"""Force a MySQL publication deadlock and verify an atomic retry."""

import http.cookiejar
import os
import re
import subprocess
import sys
import tempfile
import urllib.parse
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from card_workflow import expect  # noqa: E402
from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from mysql_card_update_deadlock_retry import (  # noqa: E402
    CLIENT, PORT, SOCKET, force_deadlock,
)
from sustained_mixed import prepare_rust, start_server, stop_server  # noqa: E402
from sustained_mixed_mysql import (  # noqa: E402
    create_databases, drop_databases, mysql_query, rust_env,
)


def main():
    if not Path(SOCKET).is_socket():
        raise SystemExit("Start the disposable MySQL server first")
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    database = "rzpublish_" + uuid.uuid4().hex[:12]
    create_databases(CLIENT, SOCKET, [database])
    server = None
    with tempfile.TemporaryDirectory(prefix="rizzy-publish-deadlock-") as temp:
        directory = Path(temp)
        try:
            local_env = rust_env(directory, "127.0.0.1", PORT, database)
            local_env["RIZZY_DEV_MAGIC_LINKS"] = "1"
            try:
                env, credentials = prepare_rust(directory, local_env)
            except subprocess.CalledProcessError as error:
                raise AssertionError(error.stderr or error.stdout) from error
            server, url = start_server("rizzy", directory, env,
                                       sorted(os.sched_getaffinity(0))[:4])
            target = {"name": "rizzy", "url": url,
                      "account": str(credentials["account"]),
                      "token": credentials["token"], "email": "owner@example.com",
                      "opener": urllib.request.build_opener(
                          urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                          NoRedirect())}
            login(target)
            board, _ = expect(target, "/boards", "POST",
                              {"board": {"name": "Publish deadlock retry"}}, 201)
            board_id = board["id"]
            account = target["account"]
            status, _, html = request(target, f"/{account}/boards/{board_id}",
                                      accept="text/html")
            assert status == 200
            csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', html)
            csrf = csrf_match.group(1).decode() if csrf_match else None
            status, headers, _ = request(target, f"/{account}/boards/{board_id}/cards",
                                         "POST", b"", accept="text/html", csrf=csrf)
            assert status == 302, status
            draft_path = urllib.parse.urlparse(headers["Location"]).path
            number = re.fullmatch(rf"/{account}/cards/(\d+)/draft", draft_path).group(1)

            query = lambda statement: mysql_query(CLIENT, SOCKET, database, statement)
            card_id = query(f"SELECT id FROM cards WHERE board_id='{board_id}' AND number={number}")
            assert card_id and query(f"SELECT status FROM cards WHERE id='{card_id}'") == "drafted"
            assert query(f"SELECT COUNT(*) FROM watches WHERE card_id='{card_id}'") == "1"
            force_deadlock(
                database, board_id, [card_id],
                lambda: expect(target, f"/cards/{number}/publish", "POST", {}, 201))

            assert query(f"SELECT status FROM cards WHERE id='{card_id}'") == "published"
            assert query(f"SELECT COUNT(*) FROM events WHERE card_id='{card_id}' "
                         "AND action='card_published'") == "1"
            assert query(f"SELECT COUNT(*) FROM watches WHERE card_id='{card_id}'") == "1"
            assert query("SELECT COUNT(*) FROM app_secrets "
                         "WHERE name LIKE 'deadlock_update_%'") == "0"
            log = (directory / "rizzy.log").read_text()
            assert "1213" in log, log[-2000:]
            print("PASS: draft publication survived forced MySQL 1213 with one event")
        finally:
            if server is not None:
                stop_server(server)
            drop_databases(CLIENT, SOCKET, [database])


if __name__ == "__main__":
    main()
