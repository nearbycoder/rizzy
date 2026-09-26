#!/usr/bin/env python3
"""Keep legacy spike timestamps while adding stable source-style record IDs."""
import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST", {"board": {"name": "Spike ID migration"}}, 201)
        card = api(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": "Old spike"}}, 201)
        owner = next(user for user in api(target, "/users") if user["role"] == "owner")
        api(target, f"/cards/{card['number']}/assignments", "POST", {
            "assignee_id": owner["id"]}, 204)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            old = db.execute("SELECT card_id,account_id,created_at,updated_at "
                             "FROM card_activity_spikes WHERE card_id=?", (card["id"],)).fetchone()
            assert old
            db.execute("DROP TABLE card_activity_spikes")
            db.execute("CREATE TABLE card_activity_spikes ("
                       "card_id TEXT PRIMARY KEY REFERENCES cards(id) ON DELETE CASCADE, "
                       "account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE, "
                       "created_at TEXT NOT NULL, updated_at TEXT NOT NULL)")
            db.execute("INSERT INTO card_activity_spikes(card_id,account_id,created_at,updated_at) "
                       "VALUES(?,?,?,?)", old)

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        for previous_id in (None, "stable"):
            _, restarted = start(env, binary=BINARY)
            try:
                with sqlite3.connect(target["db"]) as db:
                    record = db.execute("SELECT id,card_id,account_id,created_at,updated_at "
                                        "FROM card_activity_spikes WHERE card_id=?",
                                        (card["id"],)).fetchone()
                    assert record and record[0] and record[1:] == old
                    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
                    assert not any(row[2] == "cards" for row in db.execute(
                        "PRAGMA foreign_key_list(card_activity_spikes)"))
                    if previous_id == "stable":
                        assert record[0] == stable_id
                    else:
                        stable_id = record[0]
            finally:
                stop(restarted)
        print("Activity-spike ID migration passed: timestamps and ID stable across restarts")


if __name__ == "__main__":
    main()
