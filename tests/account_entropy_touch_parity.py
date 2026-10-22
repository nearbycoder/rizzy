#!/usr/bin/env python3
"""Compare account entropy bulk touches across inherited and override boards."""

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


def state(name, directory, target, fixtures):
    database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
    with sqlite3.connect(database) as db:
        account_id, account_stamp = db.execute(
            "SELECT id,updated_at FROM accounts WHERE external_account_id=?",
            (int(target["account"]),),
        ).fetchone()
        if name == "fizzy":
            entropy_stamp = db.execute(
                "SELECT updated_at FROM entropies WHERE container_type='Account' AND container_id=?",
                (account_id,),
            ).fetchone()[0]
        else:
            entropy_stamp = db.execute(
                "SELECT updated_at FROM entropy_records WHERE container_type='Account' AND container_id=?",
                (account_id,),
            ).fetchone()[0]
        boards = {}
        for label, board, column, card in fixtures:
            board_id = database_id(target, board["id"])
            column_id = database_id(target, column["id"])
            card_id = database_id(target, card["id"])
            boards[label] = (
                db.execute("SELECT updated_at FROM boards WHERE id=?", (board_id,)).fetchone()[0],
                db.execute("SELECT updated_at FROM columns WHERE id=?", (column_id,)).fetchone()[0],
                *db.execute("SELECT updated_at,last_active_at FROM cards WHERE id=?",
                            (card_id,)).fetchone(),
            )
    return account_stamp, entropy_stamp, boards


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    process, url = start_server(name, directory, env,
                                sorted(os.sched_getaffinity(0))[:2])
    try:
        target = {"name": name, "url": url,
                  "account": str(credentials["account"]),
                  "token": credentials["token"]}
        fixtures = []
        for label in ("inherited", "override"):
            board, _ = expect(target, "/boards", "POST",
                              {"board": {"name": "Account entropy " + label}}, 201)
            path = f"/boards/{board['id']}"
            column, _ = expect(target, path + "/columns", "POST",
                               {"column": {"name": "Selected"}}, 201)
            card, _ = expect(target, path + "/cards", "POST",
                             {"card": {"title": "Entropy card"}}, 201)
            expect(target, f"/cards/{card['number']}/triage", "POST",
                   {"column_id": column["id"]}, 204)
            if label == "override":
                expect(target, path + "/entropy", "PUT",
                       {"board": {"auto_postpone_period_in_days": 90}})
            fixtures.append((label, board, column, card))
        before = state(name, directory, target, fixtures)
        result = []
        for days in (7, 7, 3):
            time.sleep(.05)
            response, _ = expect(target, "/account/entropy", "PUT",
                                 {"entropy": {"auto_postpone_period_in_days": days}})
            after = state(name, directory, target, fixtures)
            board_changes = {label: tuple(old != new for old, new in
                                          zip(before[2][label], after[2][label]))
                             for label in ("inherited", "override")}
            result.append((days, response["auto_postpone_period_in_days"],
                           before[0] != after[0], before[1] != after[1], board_changes))
            before = after
        return result
    finally:
        stop_server(process)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-account-entropy-touch-") as temp:
        root = Path(temp)
        source = root / "fizzy"
        rust = root / "rizzy"
        source.mkdir()
        rust.mkdir()
        left, right = check("fizzy", source), check("rizzy", rust)
        assert left == right, (left, right)
        print("Account entropy timestamp effects matched pinned Fizzy:", left)


if __name__ == "__main__":
    main()
