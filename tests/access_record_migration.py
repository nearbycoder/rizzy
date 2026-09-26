#!/usr/bin/env python3
"""Backfill stable metadata for access rows created by older Rizzy builds."""
import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST", {"board": {"name": "Access migration"}}, 201)
        owner = board["creator"]["id"]
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            old = db.execute("SELECT b.account_id,b.created_at,x.accessed_at FROM accesses x "
                             "JOIN boards b ON b.id=x.board_id WHERE x.board_id=? AND x.user_id=?",
                             (board["id"], owner)).fetchone()
            assert old and old[2] is None
            db.execute("DROP TRIGGER access_records_insert")
            db.execute("DROP TRIGGER access_records_update")
            db.execute("DROP TABLE access_records")

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        stable_id = None
        for _ in range(2):
            _, restarted = start(env, binary=BINARY)
            try:
                with sqlite3.connect(target["db"]) as db:
                    row = db.execute("SELECT id,account_id,board_id,user_id,created_at,updated_at "
                                     "FROM access_records WHERE board_id=? AND user_id=?",
                                     (board["id"], owner)).fetchone()
                    assert row and row[1:] == (old[0], board["id"], owner, old[1], old[1])
                    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
                    if stable_id is not None:
                        assert row[0] == stable_id
                    stable_id = row[0]
            finally:
                stop(restarted)
        print("Access migration passed: old rows gain stable metadata")


if __name__ == "__main__":
    main()
