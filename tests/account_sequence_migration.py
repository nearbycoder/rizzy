#!/usr/bin/env python3
"""Check account-number migration and high-water retention on disposable databases."""

import argparse
import os
import sqlite3
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/release/rizzy"
sys.path.insert(0, str(ROOT / "bench"))
from sustained_mixed_mysql import create_databases, drop_databases, mysql_query  # noqa: E402


def run(env, command, email=None):
    local_env = {**env, "RIZZY_SEED_EMAIL": email} if email else env
    result = subprocess.run([BINARY, command], cwd=ROOT, env=local_env,
                            capture_output=True, text=True, check=True)
    if command == "seed":
        return int(result.stdout.split("Account: ", 1)[1].splitlines()[0])


def sqlite_check(directory):
    database = directory / "sequence.sqlite3"
    env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}",
           "RIZZY_STORAGE_DIR": str(directory / "storage")}
    assert run(env, "seed", "first@example.test") == 1
    with sqlite3.connect(database) as db:
        db.execute("UPDATE accounts SET external_account_id=42 WHERE external_account_id=1")
        db.execute("DROP TABLE account_external_id_sequences")
    run(env, "init-db")
    with sqlite3.connect(database) as db:
        assert db.execute("SELECT value FROM account_external_id_sequences").fetchone()[0] == 42
    assert run(env, "seed", "second@example.test") == 43
    with sqlite3.connect(database) as db:
        db.execute("UPDATE account_external_id_sequences SET value=100")
    run(env, "init-db")
    assert run(env, "seed", "third@example.test") == 101
    with sqlite3.connect(database) as db:
        numbers = [row[0] for row in db.execute(
            "SELECT external_account_id FROM accounts ORDER BY external_account_id")]
    assert numbers == [42, 43, 101], numbers


def mysql_check(directory):
    client = os.environ.get("RIZZY_TEST_MYSQL_CLIENT", "/tmp/rizzy-mysql8/bin/mysql")
    socket = os.environ.get("RIZZY_TEST_MYSQL_SOCKET", "/tmp/rizzy-mysql8/mysql.sock")
    database = "rzseq_" + uuid.uuid4().hex[:12]
    create_databases(client, socket, [database])
    env = {**os.environ,
           "RIZZY_DATABASE_URL": f"mysql://rizzy:rizzy@127.0.0.1:33080/{database}",
           "RIZZY_STORAGE_DIR": str(directory / "storage"),
           "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
           "RIZZY_DISABLE_PUSH_DELIVERY": "1"}

    def query(statement):
        return mysql_query(client, socket, database, statement)

    try:
        assert run(env, "seed", "first@example.test") == 1
        query("UPDATE accounts SET external_account_id=42 WHERE external_account_id=1")
        query("DROP TABLE account_external_id_sequences")
        run(env, "init-db")
        assert query("SELECT value FROM account_external_id_sequences") == "42"
        assert run(env, "seed", "second@example.test") == 43
        query("UPDATE account_external_id_sequences SET value=100")
        run(env, "init-db")
        assert run(env, "seed", "third@example.test") == 101
        assert query("SELECT external_account_id FROM accounts ORDER BY external_account_id").splitlines() == [
            "42", "43", "101"]
    finally:
        drop_databases(client, socket, [database])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mysql", action="store_true")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="rizzy-account-sequence-") as temp:
        directory = Path(temp)
        if args.mysql:
            mysql_check(directory)
        else:
            sqlite_check(directory)
    print("Account-number migration and high-water retention passed on " +
          ("MySQL" if args.mysql else "SQLite"))


if __name__ == "__main__":
    main()
