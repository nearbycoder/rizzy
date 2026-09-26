#!/usr/bin/env python3
"""Backfill stable watch record IDs and timestamps in older Rizzy databases."""
import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST", {"board": {"name": "Watch migration"}}, 201)
        card = api(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": "Legacy watcher"}}, 201)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            old = db.execute("SELECT card_id,user_id,watching FROM watches WHERE card_id=?",
                             (card["id"],)).fetchone()
            card_created = db.execute("SELECT created_at FROM cards WHERE id=?",
                                      (card["id"],)).fetchone()[0]
            assert old and old[2] == 1
            db.execute("DROP TABLE watches")
            db.execute("CREATE TABLE watches (card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE, "
                       "user_id TEXT NOT NULL REFERENCES users(id), watching INTEGER NOT NULL DEFAULT 1, "
                       "PRIMARY KEY(card_id,user_id))")
            db.execute("INSERT INTO watches(card_id,user_id,watching) VALUES(?,?,?)", old)

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        for attempt in range(2):
            _, restarted = start(env, binary=BINARY)
            try:
                with sqlite3.connect(target["db"]) as db:
                    row = db.execute("SELECT id,card_id,user_id,watching,created_at,updated_at "
                                     "FROM watches WHERE card_id=?", (card["id"],)).fetchone()
                    assert row and row[0] and row[1:4] == old
                    assert row[4:] == (card_created, card_created)
                    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
                    if attempt:
                        assert row[0] == stable_id
                    else:
                        stable_id = row[0]
            finally:
                stop(restarted)
        print("Watch record migration passed: stable ID and card-time fallback")


if __name__ == "__main__":
    main()
