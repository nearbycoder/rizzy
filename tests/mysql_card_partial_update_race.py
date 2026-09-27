#!/usr/bin/env python3
"""Compare a queued partial edit after another transaction changes the title."""

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
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402
from sustained_mixed_mysql import (create_databases, drop_databases, mysql_query,  # noqa: E402
                                   rust_env, source_env)

CLIENT = os.environ.get("RIZZY_TEST_MYSQL_CLIENT", "/tmp/rizzy-mysql8/bin/mysql")
SOCKET = os.environ.get("RIZZY_TEST_MYSQL_SOCKET", "/tmp/rizzy-mysql8/mysql.sock")
PORT = int(os.environ.get("MYSQL_PORT", "33080"))


def hold_and_edit(name, target, database):
    board, _ = expect(target, "/boards", "POST", {"board": {"name": "Partial edit race"}}, 201)
    card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                     {"card": {"title": "Original title", "description": "Original body"}}, 201)
    if name == "fizzy":
        card_id = mysql_query(CLIENT, SOCKET, database,
                              f"SELECT HEX(id) FROM cards WHERE number={int(card['number'])}")
        board_id = mysql_query(CLIENT, SOCKET, database,
                               f"SELECT HEX(board_id) FROM cards WHERE id=UNHEX('{card_id}')")
        card_key, board_key = f"UNHEX('{card_id}')", f"UNHEX('{board_id}')"
        lock_column = "HEX(id)"
    else:
        card_id = mysql_query(CLIENT, SOCKET, database,
                              f"SELECT id FROM cards WHERE number={int(card['number'])}")
        board_id = mysql_query(CLIENT, SOCKET, database,
                               f"SELECT board_id FROM cards WHERE id='{card_id}'")
        card_key, board_key = f"'{card_id}'", f"'{board_id}'"
        lock_column = "id"
    lock_table = "boards" if name == "rizzy" else "cards"
    lock_id = board_id if name == "rizzy" else card_id
    lock_key = board_key if name == "rizzy" else card_key
    cli_env = {**os.environ, "LD_LIBRARY_PATH": str(Path(CLIENT).resolve().parents[1] / "lib")}
    locker = subprocess.Popen(
        [CLIENT, "--no-defaults", f"--socket={SOCKET}", "-uroot",
         f"--database={database}", "--batch", "--unbuffered", "--skip-column-names"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, bufsize=1, env=cli_env)
    try:
        locker.stdin.write(
            "BEGIN;\n"
            f"SELECT {lock_column} FROM {lock_table} WHERE id={lock_key} FOR UPDATE;\n"
            f"UPDATE cards SET title='Concurrent title' WHERE id={card_key};\n"
            f"SELECT title FROM cards WHERE id={card_key};\n"
            "SELECT 'locked';\n")
        locker.stdin.flush()
        locked = read_line(locker)
        assert locked == lock_id, (name, locked, lock_id, locker.stderr.read() if not locked else "")
        assert read_line(locker) == "Concurrent title"
        assert read_line(locker) == "locked"
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            edit = pool.submit(expect, target, f"/cards/{card['number']}", "PATCH",
                               {"card": {"description": "New body"}}, 200)
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                if edit.done():
                    raise AssertionError(f"{name} edit finished before row unlock: {edit.result()}")
                waiting = int(mysql_query(
                    CLIENT, SOCKET, database,
                    "SELECT COUNT(*) FROM performance_schema.data_lock_waits w "
                    "JOIN performance_schema.data_locks l ON "
                    "w.REQUESTING_ENGINE_LOCK_ID=l.ENGINE_LOCK_ID "
                    f"WHERE l.OBJECT_SCHEMA='{database}' AND l.OBJECT_NAME='{lock_table}'"))
                if waiting:
                    break
                time.sleep(.025)
            else:
                raise AssertionError(f"{name} edit did not wait for {lock_table} row")
            locker.stdin.write("COMMIT;\nSELECT 'committed';\n")
            locker.stdin.flush()
            assert read_line(locker) == "committed"
            edited, _ = edit.result(timeout=15)
        expected_title = "Concurrent title" if name == "rizzy" else "Original title"
        assert edited["title"] == expected_title, (name, edited)
        assert edited["description"] == "New body", (name, edited)
        final, _ = expect(target, f"/cards/{card['number']}")
        assert final["title"] == expected_title, (name, final)
        assert final["description"] == "New body", (name, final)
        print(f"PASS: {name} final title after queued partial edit: {expected_title}")
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
    prefix = "rzpartial_" + uuid.uuid4().hex[:10]
    source_dbs = {role: prefix + "_f" + suffix for role, suffix in
                  (("", ""), ("CABLE_", "_c"), ("CACHE_", "_k"), ("QUEUE_", "_q"))}
    rust_db = prefix + "_r"
    databases = [*source_dbs.values(), rust_db]
    processes = []
    create_databases(CLIENT, SOCKET, databases)
    try:
        with tempfile.TemporaryDirectory(prefix="rizzy-partial-edit-race-") as temp:
            root = Path(temp)
            source_dir, rust_dir = root / "fizzy", root / "rizzy"
            source_dir.mkdir()
            rust_dir.mkdir()
            source_settings = source_env(source_dir, 1, "127.0.0.1", PORT, source_dbs)
            source_env_ready, source_credentials = prepare_source(source_dir, 1, source_settings)
            rust_settings = rust_env(rust_dir, "127.0.0.1", PORT, rust_db)
            rust_settings["RIZZY_SEED_EMAIL"] = "mysql-pair-owner@example.test"
            rust_settings["RIZZY_DEV_MAGIC_LINKS"] = "1"
            rust_env_ready, rust_credentials = prepare_rust(rust_dir, rust_settings)
            cpus = sorted(os.sched_getaffinity(0))[:2]
            for name, directory, env, credentials, database in (
                ("fizzy", source_dir, source_env_ready, source_credentials, source_dbs[""]),
                ("rizzy", rust_dir, rust_env_ready, rust_credentials, rust_db),
            ):
                process, url = start_server(name, directory, env, cpus)
                processes.append(process)
                target = {"url": url, "account": str(credentials["account"]),
                          "token": credentials["token"]}
                hold_and_edit(name, target, database)
    finally:
        for process in processes:
            stop_server(process)
        drop_databases(CLIENT, SOCKET, databases)


if __name__ == "__main__":
    main()
