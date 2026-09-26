#!/usr/bin/env python3
"""Check durable rich-text identities through edits and deletion."""
import sqlite3
import tempfile

from auth import stop
from search_html_parity import api, rust_target


def record(target, kind, record_id):
    with sqlite3.connect(target["db"]) as db:
        return db.execute("SELECT id,created_at,updated_at FROM rich_text_records "
                          "WHERE record_type=? AND record_id=?", (kind, record_id)).fetchone()


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        try:
            board = api(target, "/boards", "POST", {
                "board": {"name": "Rich text record", "public_description": "<p>Board body</p>"}}, 201)
            card = api(target, f"/boards/{board['id']}/cards", "POST", {
                "card": {"title": "Rich card", "description": "<p>Card body</p>"}}, 201)
            comment = api(target, f"/cards/{card['number']}/comments", "POST", {
                "comment": {"body": "<p>Comment body</p>"}}, 201)
            ids = {kind: record(target, kind, item["id"])[0] for kind, item in
                   (("Board", board), ("Card", card), ("Comment", comment))}
            assert len(set(ids.values())) == 3

            api(target, f"/boards/{board['id']}", "PATCH", {
                "board": {"public_description": "<p>Board revised</p>"}})
            api(target, f"/cards/{card['number']}", "PATCH", {
                "card": {"description": "<p>Card revised</p>"}})
            api(target, f"/cards/{card['number']}/comments/{comment['id']}", "PATCH", {
                "comment": {"body": "<p>Comment revised</p>"}})
            for kind, item in (("Board", board), ("Card", card), ("Comment", comment)):
                assert record(target, kind, item["id"])[0] == ids[kind]

            api(target, f"/cards/{card['number']}/comments/{comment['id']}", "DELETE", expected=204)
            assert record(target, "Comment", comment["id"]) is None
            cascade_comment = api(target, f"/cards/{card['number']}/comments", "POST", {
                "comment": {"body": "<p>Card cascade</p>"}}, 201)
            api(target, f"/cards/{card['number']}", "DELETE", expected=204)
            assert record(target, "Card", card["id"]) is None
            assert record(target, "Comment", cascade_comment["id"]) is None
            cascade_card = api(target, f"/boards/{board['id']}/cards", "POST", {
                "card": {"title": "Board cascade", "description": "<p>Remaining</p>"}}, 201)
            api(target, f"/boards/{board['id']}", "DELETE", expected=204)
            assert record(target, "Board", board["id"]) is None
            assert record(target, "Card", cascade_card["id"]) is None
            with sqlite3.connect(target["db"]) as db:
                assert db.execute("PRAGMA foreign_key_check").fetchall() == []
            print("Rich text lifecycle passed: stable IDs on edits and cleanup on deletion")
        finally:
            stop(process)


if __name__ == "__main__":
    main()
