#!/usr/bin/env python3
"""Backfill legacy account times and keep name-change touches stable."""

import os
import sqlite3
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from sustained_mixed import rust_env, start_server, stop_server  # noqa: E402


def account_time(database):
    with sqlite3.connect(database) as db:
        return db.execute("SELECT updated_at FROM accounts WHERE id='old-account'").fetchone()[0]


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-account-time-migration-") as temporary:
        root = Path(temporary)
        database = root / "rizzy.sqlite3"
        old_time = "2020-01-01T00:00:00Z"
        with sqlite3.connect(database) as db:
            db.execute("CREATE TABLE accounts (id TEXT PRIMARY KEY, external_account_id INTEGER NOT NULL UNIQUE, name TEXT NOT NULL, cards_count INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL)")
            db.execute("INSERT INTO accounts(id,external_account_id,name,created_at) VALUES('old-account',7,'Old name',?)", (old_time,))
        environment = rust_env(root)
        cpus = sorted(os.sched_getaffinity(0))[:2]
        for restart in range(2):
            process, _ = start_server("rizzy", root, environment, cpus)
            try:
                if restart == 0:
                    assert account_time(database) == old_time
                    with sqlite3.connect(database) as db:
                        db.execute("UPDATE accounts SET name='New name' WHERE id='old-account'")
                    changed = account_time(database)
                    assert changed > old_time
                    with sqlite3.connect(database) as db:
                        db.execute("UPDATE accounts SET name='New name' WHERE id='old-account'")
                        db.execute("UPDATE accounts SET cards_count=cards_count+1 WHERE id='old-account'")
                        db.execute("UPDATE accounts SET auto_postpone_period_in_days=7 WHERE id='old-account'")
                    assert account_time(database) == changed
                else:
                    assert account_time(database) == changed
                    with sqlite3.connect(database) as db:
                        columns = {row[1] for row in db.execute("PRAGMA table_info(accounts)")}
                        assert {"created_at", "updated_at", "auto_postpone_period_in_days"} <= columns
                        db.execute("INSERT INTO accounts(id,external_account_id,name,created_at) VALUES('new-account',8,'Fresh',?)", (old_time,))
                        assert db.execute("SELECT updated_at FROM accounts WHERE id='new-account'").fetchone()[0] == old_time
            finally:
                stop_server(process)
    print("Account timestamp backfill, name touches, no-ops, and restart passed")


if __name__ == "__main__":
    main()
