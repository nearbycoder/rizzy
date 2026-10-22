#!/usr/bin/env python3
"""Force a card/board MySQL lock cycle through a JSON card update."""

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

from card_workflow import expect  # noqa: E402
from sustained_mixed import prepare_rust, start_server, stop_server  # noqa: E402
from sustained_mixed_mysql import (create_databases, drop_databases, mysql_query,
                                   rust_env)  # noqa: E402

CLIENT = os.environ.get("RIZZY_TEST_MYSQL_CLIENT", "/tmp/rizzy-mysql8/bin/mysql")
SOCKET = os.environ.get("RIZZY_TEST_MYSQL_SOCKET", "/tmp/rizzy-mysql8/mysql.sock")
PORT = int(os.environ.get("MYSQL_PORT", "33080"))


def read_line(process, timeout=10):
    buffered = getattr(process, "_line_buffer", b"")
    deadline = time.monotonic() + timeout
    while b"\n" not in buffered:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([process.stdout], [], [], remaining)[0]:
            raise AssertionError("MySQL locker did not respond")
        chunk = os.read(process.stdout.fileno(), 4096)
        if not chunk:
            raise AssertionError(f"MySQL locker exited: {process.stderr.read()}")
        buffered += chunk
    line, buffered = buffered.split(b"\n", 1)
    process._line_buffer = buffered
    return line.decode().strip()


def force_deadlock(database, board_id, card_ids, make_request):
    cli_env = {**os.environ, "LD_LIBRARY_PATH": str(Path(CLIENT).resolve().parents[1] / "lib")}
    locker = subprocess.Popen(
        [CLIENT, "--no-defaults", f"--socket={SOCKET}", "-uroot",
         f"--database={database}", "--batch", "--unbuffered", "--skip-column-names"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, bufsize=1, env=cli_env)
    try:
        ballast = ",".join(f"('deadlock_update_{index}','1')" for index in range(20))
        locker.stdin.write(
            "BEGIN;\n"
            f"INSERT INTO app_secrets(name,value) VALUES {ballast};\n"
            f"SELECT id FROM cards WHERE board_id='{board_id}' ORDER BY id FOR UPDATE;\n"
            "SELECT 'cards_locked';\n")
        locker.stdin.flush()
        observed = [read_line(locker) for _ in card_ids]
        assert observed == sorted(card_ids), observed
        assert read_line(locker) == "cards_locked"

        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            update = pool.submit(make_request)
            for _ in range(200):
                if update.done():
                    raise AssertionError(f"card update finished before lock cycle: {update.result()}")
                waiting = int(mysql_query(
                    CLIENT, SOCKET, database,
                    "SELECT COUNT(*) FROM performance_schema.data_lock_waits w "
                    "JOIN performance_schema.data_locks l ON "
                    "w.REQUESTING_ENGINE_LOCK_ID=l.ENGINE_LOCK_ID "
                    f"WHERE l.OBJECT_SCHEMA='{database}' AND l.OBJECT_NAME='cards'"))
                if waiting:
                    break
                time.sleep(.025)
            else:
                raise AssertionError("card update never waited for the card row")

            locker.stdin.write(
                f"SELECT id FROM boards WHERE id='{board_id}' FOR UPDATE;\n"
                "DELETE FROM app_secrets WHERE name LIKE 'deadlock_update_%';\n"
                "COMMIT;\nSELECT 'committed';\n")
            locker.stdin.flush()
            assert read_line(locker) == board_id
            assert read_line(locker) == "committed"
            return update.result(timeout=10)
    finally:
        if locker.poll() is None:
            locker.stdin.close()
            try:
                locker.wait(timeout=2)
            except subprocess.TimeoutExpired:
                locker.kill()
                locker.wait()


def main():
    if not Path(SOCKET).is_socket():
        raise SystemExit("Start the disposable MySQL server first")
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    database = "rzupdate_" + uuid.uuid4().hex[:12]
    create_databases(CLIENT, SOCKET, [database])
    server = None
    with tempfile.TemporaryDirectory(prefix="rizzy-update-deadlock-") as temp:
        directory = Path(temp)
        try:
            env, credentials = prepare_rust(
                directory, rust_env(directory, "127.0.0.1", PORT, database))
            server, url = start_server("rizzy", directory, env,
                                       sorted(os.sched_getaffinity(0))[:4])
            target = {"url": url, "account": str(credentials["account"]),
                      "token": credentials["token"]}
            board, _ = expect(target, "/boards", "POST",
                              {"board": {"name": "Update deadlock retry"}}, 201)
            board_id = board["id"]
            cards = []
            for index in range(8):
                card, _ = expect(target, f"/boards/{board_id}/cards", "POST",
                                 {"card": {"title": f"Card {index}"}}, 201)
                cards.append(card)
            query = lambda statement: mysql_query(CLIENT, SOCKET, database, statement)
            card_ids = query(f"SELECT id FROM cards WHERE board_id='{board_id}' ORDER BY id").splitlines()
            assert len(card_ids) == 8, card_ids
            number = cards[0]["number"]
            force_deadlock(
                database, board_id, card_ids,
                lambda: expect(target, f"/cards/{number}", "PUT",
                               {"card": {"title": "Updated after deadlock"}}, 200))
            card_id = query(f"SELECT id FROM cards WHERE account_id=(SELECT id FROM accounts "
                            f"WHERE external_account_id={target['account']}) AND number={number}")
            title = query(f"SELECT title FROM cards WHERE id='{card_id}'")
            assert title == "Updated after deadlock", title
            event_count = query("SELECT COUNT(*) FROM events WHERE card_id='" + card_id +
                                "' AND action='card_title_changed'")
            assert event_count == "1", event_count
            comment_count = query(f"SELECT COUNT(*) FROM comments WHERE card_id='{card_id}'")
            assert comment_count == "1", comment_count
            log = (directory / "rizzy.log").read_text()
            assert "1213" in log, log[-2000:]
            print("PASS: card update survived forced MySQL 1213 with one title event and comment")
        finally:
            if server is not None:
                stop_server(server)
            drop_databases(CLIENT, SOCKET, [database])


if __name__ == "__main__":
    main()
