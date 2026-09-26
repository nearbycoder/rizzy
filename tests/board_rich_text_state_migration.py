#!/usr/bin/env python3
"""Upgrade a pre-null-state database without losing explicit empty board text."""
import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST",
                    {"board": {"name": "Rich state migration", "public_description": ""}}, 201)
        api(target, f"/boards/{board['id']}/publication", "POST", expected=201)
        stop(process)
        with sqlite3.connect(target["db"]) as db:
            row = db.execute("SELECT body_is_null FROM rich_text_records WHERE record_type='Board' "
                             "AND record_id=? AND name='description'", (board["id"],)).fetchone()
            assert row == (0,)
            db.execute("ALTER TABLE rich_text_records DROP COLUMN body_is_null")

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        port, restarted = start(env, binary=BINARY)
        target["url"] = f"http://127.0.0.1:{port}"
        try:
            with sqlite3.connect(target["db"]) as db:
                assert "body_is_null" in {row[1] for row in db.execute(
                    "PRAGMA table_info(rich_text_records)")}
            current = api(target, f"/boards/{board['id']}")
            assert current["public_description_html"].startswith('<div class="action-text-content">')
            cleared = api(target, f"/boards/{board['id']}", "PATCH",
                          {"board": {"public_description": None}})
            assert cleared["public_description_html"] == ""
            empty = api(target, f"/boards/{board['id']}", "PATCH",
                        {"board": {"public_description": ""}})
            assert empty["public_description_html"].startswith('<div class="action-text-content">')
            with sqlite3.connect(target["db"]) as db:
                assert db.execute("SELECT body_is_null FROM rich_text_records WHERE record_type='Board' "
                                  "AND record_id=? AND name='description'", (board["id"],)).fetchone() == (0,)
                assert db.execute("PRAGMA foreign_key_check").fetchall() == []
        finally:
            stop(restarted)
    print("Board rich-text null-state migration preserved existing records")


if __name__ == "__main__":
    main()
