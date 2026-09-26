#!/usr/bin/env python3
"""Backfill stable entropy metadata for older account and board settings."""
import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST", {
            "board": {"name": "Entropy migration", "auto_postpone_period_in_days": 90}}, 201)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            account_id, account_created = db.execute(
                "SELECT id,created_at FROM accounts WHERE external_account_id=?",
                (target["account"],)).fetchone()
            board_updated = db.execute("SELECT updated_at FROM boards WHERE id=?", (board["id"],)).fetchone()[0]
            for trigger in ("entropy_records_account_insert", "entropy_records_account_update",
                            "entropy_records_board_insert", "entropy_records_board_update",
                            "entropy_records_board_delete"):
                db.execute(f"DROP TRIGGER {trigger}")
            db.execute("DROP TABLE entropy_records")

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        stable_ids = None
        for _ in range(2):
            _, restarted = start(env, binary=BINARY)
            try:
                with sqlite3.connect(target["db"]) as db:
                    rows = {row[2]: row for row in db.execute(
                        "SELECT id,account_id,container_id,container_type,created_at,updated_at "
                        "FROM entropy_records WHERE account_id=?", (account_id,))}
                    assert set(rows) == {account_id, board["id"]}
                    assert rows[account_id][1:] == (
                        account_id, account_id, "Account", account_created, account_created)
                    assert rows[board["id"]][1:] == (
                        account_id, board["id"], "Board", board_updated, board_updated)
                    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
                    ids = {container: row[0] for container, row in rows.items()}
                    if stable_ids is not None:
                        assert ids == stable_ids
                    stable_ids = ids
            finally:
                stop(restarted)
        print("Entropy migration passed: account and board metadata stable across restarts")


if __name__ == "__main__":
    main()
