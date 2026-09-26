#!/usr/bin/env python3
"""Upgrade old closure tables and backfill closed cards without a record."""
import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST", {"board": {"name": "Closure migration"}}, 201)
        existing = api(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": "Old closure row"}}, 201)
        missing = api(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": "Flag only"}}, 201)
        api(target, f"/cards/{existing['number']}/closure", "POST", expected=204)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            old = db.execute("SELECT card_id,user_id,created_at FROM closures WHERE card_id=?",
                             (existing["id"],)).fetchone()
            fallback = db.execute("SELECT account_id,creator_id,updated_at FROM cards WHERE id=?",
                                  (missing["id"],)).fetchone()
            assert old and fallback
            db.execute("DROP TABLE closures")
            db.execute("CREATE TABLE closures (card_id TEXT PRIMARY KEY REFERENCES cards(id) ON DELETE CASCADE, "
                       "user_id TEXT NOT NULL REFERENCES users(id), created_at TEXT NOT NULL)")
            db.execute("INSERT INTO closures(card_id,user_id,created_at) VALUES(?,?,?)", old)
            db.execute("UPDATE cards SET closed=1 WHERE id=?", (missing["id"],))

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        saved_ids = None
        for _ in range(2):
            _, restarted = start(env, binary=BINARY)
            try:
                with sqlite3.connect(target["db"]) as db:
                    rows = {row[2]: row for row in db.execute(
                        "SELECT id,account_id,card_id,user_id,created_at,updated_at FROM closures")}
                    assert len(rows) == 2
                    assert rows[existing["id"]][2:] == (existing["id"], old[1], old[2], old[2])
                    assert rows[missing["id"]][1:] == (
                        fallback[0], missing["id"], fallback[1], fallback[2], fallback[2])
                    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
                    ids = {card_id: row[0] for card_id, row in rows.items()}
                    if saved_ids is not None:
                        assert ids == saved_ids
                    saved_ids = ids
            finally:
                stop(restarted)
        print("Closure migration passed: old table and flag-only card keep stable records")


if __name__ == "__main__":
    main()
