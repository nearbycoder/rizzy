#!/usr/bin/env python3
"""Compare board entropy bulk card touches with pinned Fizzy."""

import os
import sqlite3
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402
from activity_event_board_parity import database_id  # noqa: E402


def state(name, directory, target, board, column, card):
    database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
    with sqlite3.connect(database) as db:
        board_id = database_id(target, board["id"])
        column_id = database_id(target, column["id"])
        card_id = database_id(target, card["id"])
        board_stamp = db.execute("SELECT updated_at FROM boards WHERE id=?", (board_id,)).fetchone()[0]
        column_stamp = db.execute("SELECT updated_at FROM columns WHERE id=?", (column_id,)).fetchone()[0]
        card_stamp, active_stamp = db.execute(
            "SELECT updated_at,last_active_at FROM cards WHERE id=?", (card_id,)
        ).fetchone()
        if name == "fizzy":
            entropy = db.execute(
                "SELECT auto_postpone_period/86400,updated_at FROM entropies "
                "WHERE container_type='Board' AND container_id=?", (board_id,)
            ).fetchone()
        else:
            entropy = db.execute(
                "SELECT b.auto_postpone_period_in_days,e.updated_at FROM boards b "
                "JOIN entropy_records e ON e.container_type='Board' AND e.container_id=b.id "
                "WHERE b.id=? AND b.entropy_override=1", (board_id,),
            ).fetchone()
    return (board_stamp, column_stamp, card_stamp, active_stamp,
            None if entropy is None else int(entropy[0]),
            None if entropy is None else entropy[1])


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    process, url = start_server(name, directory, env,
                                sorted(os.sched_getaffinity(0))[:2])
    try:
        target = {"name": name, "url": url,
                  "account": str(credentials["account"]),
                  "token": credentials["token"]}
        board, _ = expect(target, "/boards", "POST", {"board": {"name": "Entropy touch"}}, 201)
        path = f"/boards/{board['id']}"
        column, _ = expect(target, path + "/columns", "POST",
                           {"column": {"name": "Selected"}}, 201)
        card, _ = expect(target, path + "/cards", "POST",
                         {"card": {"title": "Entropy card"}}, 201)
        expect(target, f"/cards/{card['number']}/triage", "POST",
               {"column_id": column["id"]}, 204)
        before = state(name, directory, target, board, column, card)
        result = []
        for days in (90, 90, 7):
            time.sleep(.05)
            response, _ = expect(target, path + "/entropy", "PUT",
                                 {"board": {"auto_postpone_period_in_days": days}})
            after = state(name, directory, target, board, column, card)
            result.append((days, tuple(old != new for old, new in zip(before[:4], after[:4])),
                           after[4], before[5] != after[5],
                           response["auto_postpone_period_in_days"]))
            before = after
        return result
    finally:
        stop_server(process)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-board-entropy-touch-") as temp:
        root = Path(temp)
        source = root / "fizzy"
        rust = root / "rizzy"
        source.mkdir()
        rust.mkdir()
        left, right = check("fizzy", source), check("rizzy", rust)
        # Fizzy's production JSON fragment can return a cached pre-update
        # period; stored timestamps and entropy values are compared here.
        structural = lambda runs: [(days, touches, stored, entropy_touch)
                                   for days, touches, stored, entropy_touch, _ in runs]
        assert structural(left) == structural(right), (left, right)
        print("Board entropy timestamp effects matched pinned Fizzy:", structural(left))


if __name__ == "__main__":
    main()
