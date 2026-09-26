#!/usr/bin/env python3
"""Backfill stable publication records for boards with a legacy key."""
import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST", {"board": {"name": "Publication migration"}}, 201)
        api(target, f"/boards/{board['id']}/publication", "POST", expected=201)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            old = db.execute("SELECT account_id,publication_key,updated_at FROM boards WHERE id=?",
                             (board["id"],)).fetchone()
            assert old and old[1]
            db.execute("DELETE FROM board_publications WHERE board_id=?", (board["id"],))

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        stable_id = None
        for _ in range(2):
            _, restarted = start(env, binary=BINARY)
            try:
                with sqlite3.connect(target["db"]) as db:
                    row = db.execute("SELECT id,account_id,board_id,`key`,created_at,updated_at "
                                     "FROM board_publications WHERE board_id=?", (board["id"],)).fetchone()
                    assert row and row[1:] == (old[0], board["id"], old[1], old[2], old[2])
                    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
                    if stable_id is not None:
                        assert row[0] == stable_id
                    stable_id = row[0]
            finally:
                stop(restarted)
        print("Publication migration passed: legacy key keeps a stable record")


if __name__ == "__main__":
    main()
