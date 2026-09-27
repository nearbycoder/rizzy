#!/usr/bin/env python3
"""Force MySQL lock cycles and verify JSON card and browser draft retries."""

import concurrent.futures
import http.cookiejar
import os
import re
import select
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from card_workflow import expect
from draft_workflow_parity import NoRedirect, login, request as browser_request
from sustained_mixed import prepare_rust, start_server, stop_server
from sustained_mixed_mysql import (create_databases, drop_databases, mysql_query,
                                   rust_env)

CLIENT = "/tmp/rizzy-mysql8/bin/mysql"
SOCKET = "/tmp/rizzy-mysql8/mysql.sock"


def read_line(process, timeout=10):
    if not select.select([process.stdout], [], [], timeout)[0]:
        raise AssertionError("MySQL locker did not respond")
    line = process.stdout.readline().strip()
    if not line:
        raise AssertionError(f"MySQL locker exited: {process.stderr.read()}")
    return line


def create_draft(target, board_id):
    status, headers, _ = browser_request(
        target, f"/{target['account']}/boards/{board_id}/cards", "POST", b"",
        accept="text/html", csrf=target["csrf"])
    return status, headers.get("Location")


def force_deadlock(database, account_id, board_id, make_request):
    cli_env = {**os.environ, "LD_LIBRARY_PATH": "/tmp/rizzy-mysql8/compat"}
    locker = subprocess.Popen(
        [CLIENT, "--no-defaults", f"--socket={SOCKET}", "-uroot",
         f"--database={database}", "--batch", "--unbuffered",
         "--skip-column-names"], stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        bufsize=1, env=cli_env)
    try:
        locker.stdin.write(
            "BEGIN;\n"
            f"UPDATE accounts SET cards_count=cards_count+100 WHERE id='{account_id}';\n"
            "SELECT 'account_locked';\n")
        locker.stdin.flush()
        assert read_line(locker) == "account_locked"

        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            create = pool.submit(make_request)
            for _ in range(200):
                if create.done():
                    raise AssertionError(f"card request finished before account lock: {create.result()}")
                waiting = int(mysql_query(
                    CLIENT, SOCKET, database,
                    "SELECT COUNT(*) FROM performance_schema.data_lock_waits w "
                    "JOIN performance_schema.data_locks l ON "
                    "w.REQUESTING_ENGINE_LOCK_ID=l.ENGINE_LOCK_ID "
                    f"WHERE l.OBJECT_SCHEMA='{database}' AND l.OBJECT_NAME='accounts'"))
                if waiting:
                    break
                time.sleep(.025)
            else:
                raise AssertionError("card create never waited for the account row")

            locker.stdin.write(
                f"SELECT id FROM boards WHERE id='{board_id}' FOR UPDATE;\n"
                f"UPDATE accounts SET cards_count=cards_count-100 WHERE id='{account_id}';\n"
                "COMMIT;\nSELECT 'committed';\n")
            locker.stdin.flush()
            assert read_line(locker) == board_id
            assert read_line(locker) == "committed"
            return create.result(timeout=10)
    finally:
        if locker.poll() is None:
            locker.stdin.close()
            try:
                locker.wait(timeout=2)
            except subprocess.TimeoutExpired:
                locker.kill()
                locker.wait()


def main():
    database = "rztest_" + uuid.uuid4().hex[:12]
    create_databases(CLIENT, SOCKET, [database])
    server = None
    with tempfile.TemporaryDirectory(prefix="rizzy-card-deadlock-") as temp:
        directory = Path(temp)
        try:
            local_env = rust_env(directory, "127.0.0.1", 33080, database)
            local_env["RIZZY_DEV_MAGIC_LINKS"] = "1"
            env, credentials = prepare_rust(directory, local_env)
            server, url = start_server("rizzy", directory, env,
                                       sorted(os.sched_getaffinity(0))[:4])
            target = {"url": url, "account": str(credentials["account"]),
                      "token": credentials["token"]}
            board, _ = expect(target, "/boards", "POST",
                              {"board": {"name": "Deadlock retry"}}, 201)
            board_id = board["id"]
            account_id = mysql_query(
                CLIENT, SOCKET, database,
                f"SELECT id FROM accounts WHERE external_account_id={target['account']}")

            card, _ = force_deadlock(
                database, account_id, board_id,
                lambda: expect(target, f"/boards/{board_id}/cards", "POST",
                               {"card": {"title": "Retried card", "description": "Body"}}, 201))

            counts = mysql_query(
                CLIENT, SOCKET, database,
                "SELECT (SELECT cards_count FROM accounts WHERE id='" + account_id + "'), "
                "(SELECT COUNT(*) FROM cards WHERE board_id='" + board_id + "'), "
                "(SELECT COUNT(*) FROM events WHERE board_id='" + board_id + "')")
            assert counts == "1\t1\t1", counts
            assert card["title"] == "Retried card" and card["number"] == 1, card

            target.update(name="rizzy", email="owner@example.com",
                          opener=urllib.request.build_opener(
                              urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                              NoRedirect()))
            login(target)
            page_status, _, page = browser_request(
                target, f"/{target['account']}/boards/{board_id}", accept="text/html")
            assert page_status == 200, page_status
            token = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
            target["csrf"] = token.group(1).decode() if token else None
            status, location = force_deadlock(
                database, account_id, board_id,
                lambda: create_draft(target, board_id))
            assert status == 302 and location.endswith("/cards/2/draft"), (status, location)
            counts = mysql_query(
                CLIENT, SOCKET, database,
                "SELECT (SELECT cards_count FROM accounts WHERE id='" + account_id + "'), "
                "(SELECT COUNT(*) FROM cards WHERE board_id='" + board_id + "'), "
                "(SELECT COUNT(*) FROM events WHERE board_id='" + board_id + "')")
            assert counts == "2\t2\t1", counts
            draft_state = mysql_query(
                CLIENT, SOCKET, database,
                "SELECT status, (SELECT COUNT(*) FROM watches WHERE card_id=cards.id) "
                f"FROM cards WHERE account_id='{account_id}' AND number=2")
            assert draft_state == "drafted\t1", draft_state
            log = (directory / "rizzy.log").read_text()
            assert log.count("1213") >= 2, log[-2000:]
            print("PASS: JSON card and browser draft survived forced MySQL 1213")
        finally:
            if server is not None:
                stop_server(server)
            drop_databases(CLIENT, SOCKET, [database])


if __name__ == "__main__":
    main()
