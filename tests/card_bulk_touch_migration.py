#!/usr/bin/env python3
"""Restore the bulk-touch guard on a pre-migration SQLite database."""

import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-bulk-touch-migration-") as temporary:
        target, process = rust_target(temporary)
        board = api(target, "/boards", "POST", {"board": {"name": "Bulk touch migration"}}, 201)
        column = api(target, f"/boards/{board['id']}/columns", "POST",
                     {"column": {"name": "Delete"}}, 201)
        card = api(target, f"/boards/{board['id']}/cards", "POST",
                   {"card": {"title": "Preserved card"}}, 201)
        api(target, f"/cards/{card['number']}/triage", "POST",
            {"column_id": column["id"]}, 204)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            db.execute("DROP TRIGGER cards_touch_board_update")
            db.execute("DROP TRIGGER cards_touch_column_update")
            db.execute("DROP TABLE card_bulk_touches")
            db.execute("CREATE TRIGGER cards_touch_board_update AFTER UPDATE ON cards "
                       "WHEN NEW.status='published' BEGIN "
                       "UPDATE boards SET updated_at=NEW.updated_at WHERE id=NEW.board_id; END")
            db.execute("CREATE TRIGGER cards_touch_column_update AFTER UPDATE ON cards BEGIN "
                       "UPDATE columns SET updated_at=NEW.updated_at WHERE id=NEW.column_id; END")

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        port, restarted = start(env, binary=BINARY)
        active = {**target, "url": f"http://127.0.0.1:{port}"}
        try:
            with sqlite3.connect(target["db"]) as db:
                assert db.execute("SELECT COUNT(*) FROM card_bulk_touches").fetchone()[0] == 0
                for name in ("cards_touch_board_update", "cards_touch_column_update"):
                    definition = db.execute(
                        "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?",
                        (name,)).fetchone()[0]
                    assert "card_bulk_touches" in definition, (name, definition)
            api(active, f"/boards/{board['id']}/columns/{column['id']}", "DELETE",
                expected=204)
            with sqlite3.connect(target["db"]) as db:
                assert db.execute("SELECT COUNT(*) FROM card_bulk_touches").fetchone()[0] == 0
                row = db.execute("SELECT column_id FROM cards WHERE id=?", (card["id"],)).fetchone()
                assert row == (None,), row
        finally:
            stop(restarted)
    print("SQLite bulk-touch guard migration preserved cards and installed guarded triggers")


if __name__ == "__main__":
    main()
