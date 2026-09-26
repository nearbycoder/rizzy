#!/usr/bin/env python3
"""Upgrade old card-touch triggers and verify draft and delete board times."""
import os
import sqlite3
import tempfile
import time

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def times(db, card_id):
    return db.execute("SELECT c.updated_at,b.updated_at FROM cards c JOIN boards b ON b.id=c.board_id "
                      "WHERE c.id=?", (card_id,)).fetchone()


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST", {"board": {"name": "Touch migration"}}, 201)
        card = api(target, f"/boards/{board['id']}/cards", "POST",
                   {"card": {"title": "Touch probe"}}, 201)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            for name in ("cards_touch_board_insert", "cards_touch_board_update", "cards_touch_board_delete"):
                db.execute(f"DROP TRIGGER IF EXISTS {name}")
            for action, suffix in (("INSERT", "NEW"), ("UPDATE", "NEW"), ("DELETE", "OLD")):
                db.execute(f"CREATE TRIGGER cards_touch_board_{action.lower()} AFTER {action} ON cards BEGIN "
                           f"UPDATE boards SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') "
                           f"WHERE id={suffix}.board_id; END")

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        port, restarted = start(env, binary=BINARY)
        active = {**target, "url": f"http://127.0.0.1:{port}"}
        try:
            with sqlite3.connect(target["db"]) as db:
                db.execute("PRAGMA foreign_keys=ON")
                for action in ("insert", "update"):
                    definition = db.execute("SELECT sql FROM sqlite_master WHERE name=?",
                                            (f"cards_touch_board_{action}",)).fetchone()[0]
                    assert "WHEN NEW.status='published'" in definition
                assert db.execute("SELECT 1 FROM sqlite_master WHERE name='cards_touch_board_delete'").fetchone() is None
                before = times(db, card["id"])
                time.sleep(.02)
                db.execute("UPDATE cards SET status='draft' WHERE id=?", (card["id"],))
                assert times(db, card["id"])[1] == before[1]
                draft = times(db, card["id"])

            time.sleep(.02)
            step = api(active, f"/cards/{card['number']}/steps", "POST",
                       {"step": {"content": "Draft step"}}, 201)
            with sqlite3.connect(target["db"]) as db:
                created = times(db, card["id"])
                assert created[0] > draft[0] and created[1] == draft[1]
            time.sleep(.02)
            api(active, f"/cards/{card['number']}/steps/{step['id']}", "PUT",
                {"step": {"content": "Edited draft step"}})
            with sqlite3.connect(target["db"]) as db:
                edited = times(db, card["id"])
                assert edited[0] > created[0] and edited[1] == created[1]
            time.sleep(.02)
            api(active, f"/cards/{card['number']}/steps/{step['id']}", "DELETE", expected=204)
            with sqlite3.connect(target["db"]) as db:
                db.execute("PRAGMA foreign_keys=ON")
                deleted = times(db, card["id"])
                assert deleted[0] > edited[0] and deleted[1] == edited[1]
                time.sleep(.02)
                db.execute("UPDATE cards SET status='published' WHERE id=?", (card["id"],))
                published = times(db, card["id"])
                assert published[1] > deleted[1]
            time.sleep(.02)
            published_step = api(active, f"/cards/{card['number']}/steps", "POST",
                                 {"step": {"content": "Published step"}}, 201)
            with sqlite3.connect(target["db"]) as db:
                after_published_create = times(db, card["id"])
                assert after_published_create[1] > published[1]
            time.sleep(.02)
            api(active, f"/cards/{card['number']}/steps/{published_step['id']}", "DELETE", expected=204)
            with sqlite3.connect(target["db"]) as db:
                after_published_delete = times(db, card["id"])
                assert after_published_delete[0] > after_published_create[0]
                assert after_published_delete[1] > after_published_create[1]
            time.sleep(.02)
            with sqlite3.connect(target["db"]) as db:
                db.execute("PRAGMA foreign_keys=ON")
                db.execute("DELETE FROM cards WHERE id=?", (card["id"],))
                assert db.execute("SELECT updated_at FROM boards WHERE id=?", (board["id"],)).fetchone()[0] == after_published_delete[1]
                violations = db.execute("PRAGMA foreign_key_check").fetchall()
                assert violations == [], violations
        finally:
            stop(restarted)
    print("Card board-touch migration passed: draft steps leave board unchanged; publication touches; delete does not")


if __name__ == "__main__":
    main()
