#!/usr/bin/env python3
"""Force a board/card InnoDB cycle and verify a whole board-move retry."""

import concurrent.futures
import os
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from card_workflow import expect  # noqa: E402
from mysql_card_update_deadlock_retry import read_line  # noqa: E402
from sustained_mixed import prepare_rust, start_server, stop_server  # noqa: E402
from sustained_mixed_mysql import (  # noqa: E402
    create_databases, drop_databases, mysql_admin, mysql_query, rust_env,
)

CLIENT = os.environ.get("RIZZY_TEST_MYSQL_CLIENT", "/tmp/rizzy-mysql8/bin/mysql")
SOCKET = os.environ.get("RIZZY_TEST_MYSQL_SOCKET", "/tmp/rizzy-mysql8/mysql.sock")
PORT = int(os.environ.get("MYSQL_PORT", "33080"))


def force_deadlock(database, card_id, board_id, make_request):
    client_env = {**os.environ,
                  "LD_LIBRARY_PATH": str(Path(CLIENT).resolve().parents[1] / "lib")}
    locker = subprocess.Popen(
        [CLIENT, "--no-defaults", f"--socket={SOCKET}", "-uroot",
         f"--database={database}", "--batch", "--unbuffered", "--skip-column-names"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, bufsize=1, env=client_env)
    try:
        ballast = ",".join(f"('deadlock_move_{index}','1')" for index in range(30))
        locker.stdin.write(
            "BEGIN;\n"
            f"INSERT INTO app_secrets(name,value) VALUES {ballast};\n"
            f"SELECT id FROM cards WHERE id='{card_id}' FOR UPDATE;\n"
            "SELECT 'card_locked';\n")
        locker.stdin.flush()
        assert read_line(locker) == card_id
        assert read_line(locker) == "card_locked"

        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            move = pool.submit(make_request)
            for _ in range(300):
                if move.done():
                    raise AssertionError(f"move finished before lock cycle: {move.result()}")
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
                raise AssertionError("move never waited for the card row")

            locker.stdin.write(
                f"SELECT id FROM boards WHERE id='{board_id}' FOR UPDATE;\n"
                "DELETE FROM app_secrets WHERE name LIKE 'deadlock_move_%';\n"
                "COMMIT;\nSELECT 'committed';\n")
            locker.stdin.flush()
            assert read_line(locker) == board_id
            assert read_line(locker) == "committed"
            return move.result(timeout=15)
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
    old_trust = mysql_admin(CLIENT, SOCKET, "SELECT @@GLOBAL.log_bin_trust_function_creators").strip()
    mysql_admin(CLIENT, SOCKET, "SET GLOBAL log_bin_trust_function_creators=1")
    database = "rzmoveretry_" + uuid.uuid4().hex[:12]
    created = False
    server = None
    try:
        create_databases(CLIENT, SOCKET, [database])
        created = True
        with tempfile.TemporaryDirectory(prefix="rizzy-move-retry-") as temp:
            directory = Path(temp)
            env, credentials = prepare_rust(
                directory, rust_env(directory, "127.0.0.1", PORT, database))
            server, url = start_server("rizzy", directory, env,
                                       sorted(os.sched_getaffinity(0))[:4])
            target = {"url": url, "account": str(credentials["account"]),
                      "token": credentials["token"]}
            source, _ = expect(target, "/boards", "POST",
                               {"board": {"name": "Move retry source"}}, 201)
            destination, _ = expect(target, "/boards", "POST",
                                    {"board": {"name": "Move retry destination"}}, 201)
            card, _ = expect(target, f"/boards/{source['id']}/cards", "POST",
                             {"card": {"title": "Move after deadlock"}}, 201)
            query = lambda statement: mysql_query(CLIENT, SOCKET, database, statement)
            number, card_id = card["number"], card["id"]
            moved, _ = force_deadlock(
                database, card_id, min(source["id"], destination["id"]),
                lambda: expect(target, f"/cards/{number}/board", "PATCH",
                               {"board_id": destination["id"]}, 200))
            assert moved["board"]["id"] == destination["id"], moved
            state = query(
                "SELECT (SELECT board_id FROM cards WHERE id='" + card_id + "'), "
                "(SELECT COUNT(*) FROM events WHERE card_id='" + card_id +
                "' AND action='card_board_changed'), "
                "(SELECT COUNT(*) FROM comments WHERE card_id='" + card_id + "')")
            assert state == f"{destination['id']}\t1\t1", state
            assert query("SELECT COUNT(*) FROM app_secrets WHERE name LIKE 'deadlock_move_%'") == "0"
            log = (directory / "rizzy.log").read_text()
            assert "1213" in log, log[-2000:]
    finally:
        if server is not None:
            stop_server(server)
        if created:
            drop_databases(CLIENT, SOCKET, [database])
        mysql_admin(CLIENT, SOCKET,
                    f"SET GLOBAL log_bin_trust_function_creators={old_trust}")
    print("PASS MySQL board move retries a forced 1213 with one event and comment")


if __name__ == "__main__":
    main()
