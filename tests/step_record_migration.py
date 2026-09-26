#!/usr/bin/env python3
"""Check step update-time backfill on an existing Rizzy database."""
import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST", {"board": {"name": "Step migration"}}, 201)
        card = api(target, f"/boards/{board['id']}/cards", "POST",
                   {"card": {"title": "Legacy steps"}}, 201)
        step = api(target, f"/cards/{card['number']}/steps", "POST",
                   {"step": {"content": "Before migration"}}, 201)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            old = db.execute("SELECT id,card_id,content,completed,created_at FROM steps WHERE id=?",
                             (step["id"],)).fetchone()
            db.execute("DROP TRIGGER steps_updated_at_insert")
            db.execute("DROP TABLE steps")
            db.execute("CREATE TABLE steps (id TEXT PRIMARY KEY, card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE, "
                       "content TEXT NOT NULL, completed INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL)")
            db.execute("INSERT INTO steps(id,card_id,content,completed,created_at) VALUES(?,?,?,?,?)", old)

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        for attempt in range(2):
            _, restarted = start(env, binary=BINARY)
            try:
                with sqlite3.connect(target["db"]) as db:
                    row = db.execute("SELECT id,card_id,content,completed,created_at,updated_at "
                                     "FROM steps WHERE id=?", (step["id"],)).fetchone()
                    assert row == (*old, old[-1])
                    if attempt == 0:
                        db.execute("INSERT INTO steps(id,card_id,content,created_at) VALUES('legacy-writer',?, 'Later step', ?)",
                                   (card["id"], old[-1]))
                    assert db.execute("SELECT updated_at FROM steps WHERE id='legacy-writer'").fetchone()[0] == old[-1]
                    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
            finally:
                stop(restarted)
    print("Step record migration passed: update-time backfill and legacy insert")


if __name__ == "__main__":
    main()
