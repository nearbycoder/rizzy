#!/usr/bin/env python3
"""Compare card and named-column timestamp effects with pinned Fizzy."""

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
                          {"board": {"name": "Column touch"}}, 201)
        column, _ = expect(target, f"/boards/{board['id']}/columns", "POST",
                           {"column": {"name": "In progress"}}, 201)
        card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                         {"card": {"title": "Touch this"}}, 201)
        database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")

        def snapshot():
            with sqlite3.connect(database) as db:
                card_times = db.execute(
                    "SELECT c.updated_at,c.last_active_at,b.updated_at "
                    "FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.number=?",
                    (card["number"],)).fetchone()
                column_time = db.execute("SELECT updated_at FROM columns WHERE id=?",
                    (database_id(target, column["id"]),)).fetchone()[0]
                return (*card_times, column_time)

        before = snapshot()
        changes = {}
        actions = (
            ("triage", f"/cards/{card['number']}/triage", "POST",
             {"column_id": column["id"]}, 204),
            ("title", f"/cards/{card['number']}", "PATCH",
             {"card": {"title": "Edited title"}}, 200),
            ("comment", f"/cards/{card['number']}/comments", "POST",
             {"comment": {"body": "A note"}}, 201),
            ("rename column", f"/boards/{board['id']}/columns/{column['id']}", "PATCH",
             {"column": {"name": "Renamed"}}, 200),
            ("repeat rename", f"/boards/{board['id']}/columns/{column['id']}", "PATCH",
             {"column": {"name": "Renamed"}}, 200),
            ("untriage", f"/cards/{card['number']}/triage", "DELETE", None, 204),
        )
        for label, path, method, payload, status in actions:
            time.sleep(.05)
            expect(target, path, method, payload, status)
            current = snapshot()
            changes[label] = tuple(a != b for a, b in zip(before, current))
            before = current
        return changes
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-column-touch-") as temporary:
        root = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = root / name
            directory.mkdir()
            results[name] = check(name, directory)
        print(results)
        assert results["fizzy"] == results["rizzy"], results
    print("Column timestamp effects matched pinned Fizzy")


if __name__ == "__main__":
    main()
