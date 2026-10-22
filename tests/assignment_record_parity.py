#!/usr/bin/env python3
"""Compare assignment record and card side effects with pinned Fizzy."""

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


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    process, url = start_server(name, directory, env,
                                sorted(os.sched_getaffinity(0))[:2])
    try:
        target = {"url": url, "account": str(credentials["account"]),
                  "token": credentials["token"]}
        board, _ = expect(target, "/boards", "POST",
                          {"board": {"name": "Assignment record"}}, 201)
        card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                         {"card": {"title": "Assign me"}}, 201)
        users, _ = expect(target, "/users")
        owner = next(user for user in users if user["role"] == "owner")
        database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")

        def snapshot():
            with sqlite3.connect(database) as db:
                row = db.execute(
                    "SELECT c.updated_at,c.last_active_at,b.updated_at,"
                    "(SELECT COUNT(*) FROM assignments a WHERE a.card_id=c.id),"
                    "(SELECT COUNT(*) FROM comments m WHERE m.card_id=c.id),"
                    "(SELECT COUNT(*) FROM events e WHERE e.eventable_type='Card' "
                    "AND e.eventable_id=c.id) "
                    "FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.number=?",
                    (card["number"],),
                ).fetchone()
                assignment = db.execute(
                    "SELECT created_at,updated_at FROM assignments WHERE card_id="
                    "(SELECT id FROM cards WHERE number=?)", (card["number"],),
                ).fetchone()
                return row, assignment

        before, initial_assignment = snapshot()
        assert initial_assignment is None
        changes = []
        for expected_count in (1, 0):
            time.sleep(.05)
            expect(target, f"/cards/{card['number']}/assignments", "POST",
                   {"assignee_id": owner["id"]}, 204)
            current, assignment = snapshot()
            changes.append((tuple(a != b for a, b in zip(current[:3], before[:3])),
                            tuple(a - b for a, b in zip(current[3:], before[3:])),
                            assignment is not None))
            assert current[3] == expected_count
            if assignment:
                assert assignment[0] == assignment[1], (name, assignment)
            before = current
        return changes
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-assignment-record-") as temporary:
        base = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = base / name
            directory.mkdir()
            results[name] = check(name, directory)
        expected = [
            ((True, True, True), (1, 1, 1), True),
            ((True, True, True), (-1, 1, 1), False),
        ]
        assert results == {"fizzy": expected, "rizzy": expected}, results
    print("Assignment record and card effects match pinned Fizzy")


if __name__ == "__main__":
    main()
