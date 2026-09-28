#!/usr/bin/env python3
"""Compare named-column create and delete side effects with pinned Fizzy."""

import os
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, start_server,
                             stop_server)  # noqa: E402
from activity_event_board_parity import database_id  # noqa: E402


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    process, url = start_server(name, directory, env,
                                sorted(os.sched_getaffinity(0))[:2])
    try:
        target = {"name": name, "url": url, "account": str(credentials["account"]),
                  "token": credentials["token"]}
        board, _ = expect(target, "/boards", "POST",
                          {"board": {"name": "Column lifecycle"}}, 201)
        board_path = f"/boards/{board['id']}"
        database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
        board_db_id = database_id(target, board["id"])

        def snapshot(cards):
            with sqlite3.connect(database) as db:
                board_time = db.execute("SELECT updated_at FROM boards WHERE id=?",
                                        (board_db_id,)).fetchone()[0]
                card_state = []
                for card in cards:
                    card_state.append(db.execute(
                        "SELECT updated_at,last_active_at,column_id FROM cards WHERE number=?",
                        (card["number"],)).fetchone())
                return board_time, card_state

        before_create = snapshot([])
        time.sleep(.05)
        first, _ = expect(target, board_path + "/columns", "POST",
                          {"column": {"name": "First"}}, 201)
        after_create = snapshot([])
        second, _ = expect(target, board_path + "/columns", "POST",
                           {"column": {"name": "Second"}}, 201)
        with sqlite3.connect(database) as db:
            positions = tuple(db.execute(
                "SELECT position FROM columns WHERE board_id=? ORDER BY position,id",
                (board_db_id,)).fetchall())
        cards = []
        for label in ("In first", "In second", "In Maybe"):
            card, _ = expect(target, board_path + "/cards", "POST",
                             {"card": {"title": label}}, 201)
            cards.append(card)
        for card, column in ((cards[0], first), (cards[1], second)):
            expect(target, f"/cards/{card['number']}/triage", "POST",
                   {"column_id": column["id"]}, 204)
        second_db_id = database_id(target, second["id"])
        def second_column_time():
            with sqlite3.connect(database) as db:
                return db.execute("SELECT updated_at FROM columns WHERE id=?",
                                  (second_db_id,)).fetchone()[0]

        before_delete = snapshot(cards)
        before_second_column = second_column_time()
        time.sleep(.05)
        expect(target, board_path + f"/columns/{first['id']}", "DELETE", None, 204)
        after_delete = snapshot(cards)
        assert after_delete[1][0][2] is None, (name, after_delete)
        assert after_delete[1][1][2] == before_delete[1][1][2], name
        assert after_delete[1][2][2] is None, name
        return {
            "create_board_touch": before_create[0] != after_create[0],
            "positions": positions,
            "delete_board_touch": before_delete[0] != after_delete[0],
            "delete_card_updates": tuple(before[0] != after[0] for before, after in
                                         zip(before_delete[1], after_delete[1])),
            "delete_card_activity": tuple(before[1] != after[1] for before, after in
                                          zip(before_delete[1], after_delete[1])),
            "delete_other_column_touch": before_second_column != second_column_time(),
            "deleted_column_card_untriaged": after_delete[1][0][2] is None,
        }
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-column-lifecycle-") as temporary:
        root = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = root / name
            directory.mkdir()
            results[name] = check(name, directory)
        print(results)
        assert results["fizzy"] == results["rizzy"], results
    print("Column create and delete timestamp effects matched pinned Fizzy")


if __name__ == "__main__":
    main()
