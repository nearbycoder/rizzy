#!/usr/bin/env python3
"""Backfill assignment update times on an older Rizzy SQLite database."""

import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST",
                    {"board": {"name": "Assignment migration"}}, 201)
        card = api(target, f"/boards/{board['id']}/cards", "POST",
                   {"card": {"title": "Legacy assignment"}}, 201)
        later_card = api(target, f"/boards/{board['id']}/cards", "POST",
                         {"card": {"title": "Legacy writer"}}, 201)
        owner = next(user for user in api(target, "/users") if user["role"] == "owner")
        api(target, f"/cards/{card['number']}/assignments", "POST",
            {"assignee_id": owner["id"]}, expected=204)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            old = db.execute(
                "SELECT id,account_id,card_id,assignee_id,assigner_id,created_at "
                "FROM assignments WHERE card_id=?", (card["id"],)).fetchone()
            assert old
            db.execute("DROP TRIGGER assignments_updated_at_insert")
            db.execute("DROP TABLE assignments")
            db.execute(
                "CREATE TABLE assignments (id TEXT PRIMARY KEY, account_id TEXT NOT NULL "
                "REFERENCES accounts(id), card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE, "
                "assignee_id TEXT NOT NULL REFERENCES users(id), assigner_id TEXT NOT NULL "
                "REFERENCES users(id), created_at TEXT NOT NULL, UNIQUE(card_id,assignee_id))")
            db.execute("INSERT INTO assignments(id,account_id,card_id,assignee_id,assigner_id,created_at) "
                       "VALUES(?,?,?,?,?,?)", old)

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        for attempt in range(2):
            _, restarted = start(env, binary=BINARY)
            try:
                with sqlite3.connect(target["db"]) as db:
                    row = db.execute(
                        "SELECT id,account_id,card_id,assignee_id,assigner_id,created_at,updated_at "
                        "FROM assignments WHERE id=?", (old[0],)).fetchone()
                    assert row == (*old, old[-1])
                    if attempt == 0:
                        db.execute(
                            "INSERT INTO assignments(id,account_id,card_id,assignee_id,assigner_id,created_at) "
                            "VALUES('legacy-writer',?,?,?,?,?)",
                            (old[1], later_card["id"], owner["id"], owner["id"], old[-1]))
                    assert db.execute(
                        "SELECT updated_at FROM assignments WHERE id='legacy-writer'").fetchone()[0] == old[-1]
                    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
            finally:
                stop(restarted)
            if attempt == 0:
                # Simulate an interrupted backfill after the column was added.
                with sqlite3.connect(target["db"]) as db:
                    db.execute("UPDATE assignments SET updated_at='' WHERE id=?", (old[0],))
                    db.execute("DELETE FROM app_secrets WHERE name='assignments_updated_at_v1'")
    print("Assignment update-time migration passed: backfill, interrupted recovery, and legacy insert")


if __name__ == "__main__":
    main()
