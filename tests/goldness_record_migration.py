#!/usr/bin/env python3
"""Backfill Golden Ticket records from legacy card flags."""
import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST", {"board": {"name": "Goldness migration"}}, 201)
        first = api(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": "Existing gold"}}, 201)
        second = api(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": "Flag only gold"}}, 201)
        api(target, f"/cards/{first['number']}/goldness", "POST", expected=204)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            db.execute("DROP TABLE card_goldnesses")
            db.execute("UPDATE cards SET golden=1 WHERE id=?", (second["id"],))
            updated = dict(db.execute("SELECT id,updated_at FROM cards WHERE id IN (?,?)",
                                      (first["id"], second["id"])).fetchall())

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        _, restarted = start(env, binary=BINARY)
        try:
            with sqlite3.connect(target["db"]) as db:
                rows = {card_id: (record_id, created, modified)
                        for card_id, record_id, created, modified in db.execute(
                            "SELECT card_id,id,created_at,updated_at FROM card_goldnesses")}
                assert set(rows) == {first["id"], second["id"]}
                for card_id, (_, created, modified) in rows.items():
                    assert created == modified == updated[card_id]
                assert db.execute("PRAGMA foreign_key_check").fetchall() == []
        finally:
            stop(restarted)

        _, restarted = start(env, binary=BINARY)
        try:
            with sqlite3.connect(target["db"]) as db:
                assert {card_id: record_id for card_id, record_id in db.execute(
                    "SELECT card_id,id FROM card_goldnesses")} == {
                        card_id: value[0] for card_id, value in rows.items()}
        finally:
            stop(restarted)
        print("Goldness legacy migration passed: both flags restored with stable records")


if __name__ == "__main__":
    main()
