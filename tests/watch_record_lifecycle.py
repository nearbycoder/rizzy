#!/usr/bin/env python3
"""Check source-style watch identity and timestamp transitions in Rizzy."""
import sqlite3
import tempfile

from auth import stop
from search_html_parity import api, rust_target


def record(target, card_id, user_id):
    with sqlite3.connect(target["db"]) as db:
        return db.execute("SELECT id,watching,created_at,updated_at FROM watches "
                          "WHERE card_id=? AND user_id=?", (card_id, user_id)).fetchone()


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        try:
            board = api(target, "/boards", "POST", {"board": {"name": "Watch record lifecycle"}}, 201)
            card = api(target, f"/boards/{board['id']}/cards", "POST", {
                "card": {"title": "Watched card"}}, 201)
            owner = next(user for user in api(target, "/users") if user["role"] == "owner")
            path = f"/cards/{card['number']}/watch"
            initial_activity = api(target, f"/cards/{card['number']}")["last_active_at"]
            first = record(target, card["id"], owner["id"])
            assert first and first[0] and first[1] == 1 and first[2] == first[3]
            api(target, path, "POST", expected=204)
            assert record(target, card["id"], owner["id"]) == first
            api(target, path, "DELETE", expected=204)
            off = record(target, card["id"], owner["id"])
            assert off[0] == first[0] and off[1] == 0 and off[2] == first[2]
            api(target, path, "DELETE", expected=204)
            assert record(target, card["id"], owner["id"]) == off
            api(target, path, "POST", expected=204)
            on = record(target, card["id"], owner["id"])
            assert on[0] == first[0] and on[1] == 1 and on[2] == first[2]
            assert api(target, f"/cards/{card['number']}")["last_active_at"] == initial_activity
            api(target, f"/boards/{board['id']}", "DELETE", expected=204)
            print("Watch record lifecycle passed: stable ID, repeat no-ops, state changes")
        finally:
            stop(process)


if __name__ == "__main__":
    main()
