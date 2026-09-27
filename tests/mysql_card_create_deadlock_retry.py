#!/usr/bin/env python3
"""Force a MySQL lock cycle and verify JSON card creation retries atomically."""

import concurrent.futures
import os
import select
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from card_workflow import expect
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


def main():
    database = "rztest_" + uuid.uuid4().hex[:12]
    create_databases(CLIENT, SOCKET, [database])
    server = locker = None
    with tempfile.TemporaryDirectory(prefix="rizzy-card-deadlock-") as temp:
        directory = Path(temp)
        try:
            env, credentials = prepare_rust(
                directory, rust_env(directory, "127.0.0.1", 33080, database))
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

            cli_env = {**os.environ, "LD_LIBRARY_PATH": "/tmp/rizzy-mysql8/compat"}
            locker = subprocess.Popen(
                [CLIENT, "--no-defaults", f"--socket={SOCKET}", "-uroot",
                 f"--database={database}", "--batch", "--unbuffered",
                 "--skip-column-names"], stdin=subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                bufsize=1, env=cli_env)
            locker.stdin.write(
                "BEGIN;\n"
                f"UPDATE accounts SET cards_count=cards_count+100 WHERE id='{account_id}';\n"
                "SELECT 'account_locked';\n")
            locker.stdin.flush()
            assert read_line(locker) == "account_locked"

            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                create = pool.submit(
                    expect, target, f"/boards/{board_id}/cards", "POST",
                    {"card": {"title": "Retried card", "description": "Body"}}, 201)
                for _ in range(200):
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
                card, _ = create.result(timeout=10)

            counts = mysql_query(
                CLIENT, SOCKET, database,
                "SELECT (SELECT cards_count FROM accounts WHERE id='" + account_id + "'), "
                "(SELECT COUNT(*) FROM cards WHERE board_id='" + board_id + "'), "
                "(SELECT COUNT(*) FROM events WHERE board_id='" + board_id + "')")
            assert counts == "1\t1\t1", counts
            assert card["title"] == "Retried card" and card["number"] == 1, card
            log = (directory / "rizzy.log").read_text()
            assert "1213" in log, log[-2000:]
            print("PASS: card creation survived forced MySQL 1213; one card and event")
        finally:
            if locker is not None:
                if locker.poll() is None:
                    locker.stdin.close()
                    try:
                        locker.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        locker.kill()
                        locker.wait()
            if server is not None:
                stop_server(server)
            drop_databases(CLIENT, SOCKET, [database])


if __name__ == "__main__":
    main()
