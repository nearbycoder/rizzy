#!/usr/bin/env python3
"""Compare MySQL card date scalar errors and transaction outcomes with Fizzy."""

import os
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
sys.path.insert(0, str(ROOT / "tests"))

from card_timestamp_invalid_parity import request  # noqa: E402
from card_workflow import expect  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402
from sustained_mixed_mysql import (create_databases, drop_databases, mysql_query,  # noqa: E402
                                   rust_env, source_env)

CLIENT = os.environ.get("RIZZY_TEST_MYSQL_CLIENT", "/tmp/rizzy-mysql8/bin/mysql")
SOCKET = os.environ.get("RIZZY_TEST_MYSQL_SOCKET", "/tmp/rizzy-mysql8/mysql.sock")
HOST = os.environ.get("RIZZY_TEST_MYSQL_HOST", "127.0.0.1")
PORT = int(os.environ.get("RIZZY_TEST_MYSQL_PORT", "33080"))
CASES = (("one", 1), ("true", True), ("false", False), ("decimal", 1.5))
ERROR = (500, {"status": 500, "error": "Internal Server Error"})


def row(database, title):
    return mysql_query(CLIENT, SOCKET, database,
                       "SELECT number,created_at,last_active_at,updated_at "
                       f"FROM cards WHERE title='{title}'")


def count(database):
    return int(mysql_query(CLIENT, SOCKET, database,
                           "SELECT cards_count FROM accounts LIMIT 1"))


def run(name, directory, source_dbs, rust_db):
    if name == "fizzy":
        env, credentials = prepare_source(
            directory, 2, source_env(directory, 2, HOST, PORT, source_dbs))
        database = source_dbs[""]
    else:
        env, credentials = prepare_rust(directory, rust_env(directory, HOST, PORT, rust_db))
        database = rust_db
    process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
    target = {"url": url, "account": str(credentials["account"]),
              "token": credentials["token"]}
    try:
        board, _ = expect(target, "/boards", "POST", {"board": {"name": "MySQL scalar dates"}}, 201)
        outcomes = []
        for key in ("created_at", "last_active_at"):
            for label, value in CASES:
                title = f"{key}-{label}-create"
                before = count(database)
                status, body = request(target, f"/boards/{board['id']}/cards", "POST",
                                       {"card": {"title": title, key: value}})
                if label == "false":
                    assert status == 201, (name, key, label, status, body)
                    assert body["created_at"] == body["last_active_at"]
                    assert row(database, title) and count(database) == before + 1
                    outcomes.append((key, label, "create", 201, True))
                else:
                    assert (status, body) == ERROR, (name, key, label, status, body)
                    assert not row(database, title) and count(database) == before
                    outcomes.append((key, label, "create", 500, False))
                original, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                                     {"card": {"title": f"{key}-{label}-update"}}, 201)
                old = row(database, original["title"])
                before = count(database)
                result = request(target, f"/cards/{original['number']}", "PATCH",
                                 {"card": {key: value}})
                assert result == ERROR, (name, key, label, result)
                assert row(database, original["title"]) == old and count(database) == before
                outcomes.append((key, label, "update", 500, True))
        return outcomes
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    prefix = "rzscalar_" + uuid.uuid4().hex[:10]
    source_dbs = {role: prefix + suffix for role, suffix in
                  (("", "_f"), ("CABLE_", "_c"), ("CACHE_", "_k"), ("QUEUE_", "_q"))}
    rust_db = prefix + "_r"
    names = [*source_dbs.values(), rust_db]
    create_databases(CLIENT, SOCKET, names)
    try:
        with tempfile.TemporaryDirectory(prefix="rizzy-mysql-card-scalar-dates-") as temporary:
            runs = {}
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                runs[name] = run(name, directory, source_dbs, rust_db)
        assert runs["fizzy"] == runs["rizzy"], runs
    finally:
        drop_databases(CLIENT, SOCKET, names)
    print("Matched Fizzy's MySQL card date scalar responses and rollback in 16 cases")


if __name__ == "__main__":
    main()
