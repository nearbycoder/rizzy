#!/usr/bin/env python3
"""Backfill legacy postponed flags when the Not Now record table is absent."""
import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST", {"board": {"name": "Not Now migration"}}, 201)
        active = api(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": "Event backed"}}, 201)
        fallback = api(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": "Card backed"}}, 201)
        api(target, f"/cards/{active['number']}/not_now", "POST", expected=204)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            event = db.execute("SELECT creator_id,created_at FROM events WHERE card_id=? "
                               "AND action='card_postponed' ORDER BY created_at DESC LIMIT 1",
                               (active["id"],)).fetchone()
            fallback_row = db.execute("SELECT creator_id,updated_at FROM cards WHERE id=?",
                                      (fallback["id"],)).fetchone()
            assert event and fallback_row
            db.execute("DROP TABLE card_not_nows")
            db.execute("UPDATE cards SET postponed=1 WHERE id=?", (fallback["id"],))

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        _, restarted = start(env, binary=BINARY)
        try:
            with sqlite3.connect(target["db"]) as db:
                rows = dict(db.execute(
                    "SELECT card_id,(user_id || ':' || created_at) FROM card_not_nows"
                ).fetchall())
                assert len(rows) == 2
                assert rows[active["id"]] == event[0] + ":" + event[1]
                assert rows[fallback["id"]] == fallback_row[0] + ":" + fallback_row[1]
                ids = dict(db.execute("SELECT card_id,id FROM card_not_nows").fetchall())
                assert db.execute("PRAGMA foreign_key_check").fetchall() == []
        finally:
            stop(restarted)

        _, restarted = start(env, binary=BINARY)
        try:
            with sqlite3.connect(target["db"]) as db:
                assert dict(db.execute("SELECT card_id,id FROM card_not_nows").fetchall()) == ids
        finally:
            stop(restarted)
        print("Not Now legacy migration passed: event and card fallback, stable restart")


if __name__ == "__main__":
    main()
