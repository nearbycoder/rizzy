#!/usr/bin/env python3
"""Backfill record IDs for existing inline card, comment, and board text."""
import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST", {
            "board": {"name": "Rich text migration", "public_description": "<p>Board</p>"}}, 201)
        card = api(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": "Text", "description": "<p>Card</p>"}}, 201)
        comment = api(target, f"/cards/{card['number']}/comments", "POST", {
            "comment": {"body": "<p>Comment</p>"}}, 201)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            db.execute("DROP TABLE rich_text_records")
            expected = {
                "Board": db.execute("SELECT created_at,updated_at FROM boards WHERE id=?",
                                    (board["id"],)).fetchone(),
                "Card": db.execute("SELECT created_at,updated_at FROM cards WHERE id=?",
                                   (card["id"],)).fetchone(),
                "Comment": db.execute("SELECT created_at,updated_at FROM comments WHERE id=?",
                                      (comment["id"],)).fetchone(),
            }

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        for attempt in range(2):
            _, restarted = start(env, binary=BINARY)
            try:
                with sqlite3.connect(target["db"]) as db:
                    rows = {kind: (record_id, created, updated) for kind, record_id, created, updated
                            in db.execute("SELECT record_type,id,created_at,updated_at FROM rich_text_records")}
                    assert set(rows) == {"Board", "Card", "Comment"}
                    for kind, (record_id, created, updated) in rows.items():
                        assert record_id and (created, updated) == expected[kind]
                    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
                    if attempt:
                        assert {kind: row[0] for kind, row in rows.items()} == stable_ids
                    else:
                        stable_ids = {kind: row[0] for kind, row in rows.items()}
            finally:
                stop(restarted)
        print("Rich text migration passed: three record types and stable restarts")


if __name__ == "__main__":
    main()
