#!/usr/bin/env python3
"""Compare stored watch touch effects with pinned Fizzy."""

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
                          {"board": {"name": "Watch touch"}}, 201)
        card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                         {"card": {"title": "Watch me"}}, 201)
        database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")

        def snapshot():
            with sqlite3.connect(database) as db:
                return db.execute(
                    "SELECT c.updated_at,c.last_active_at,b.updated_at,w.updated_at,w.watching "
                    "FROM cards c JOIN boards b ON b.id=c.board_id "
                    "JOIN watches w ON w.card_id=c.id WHERE c.number=?",
                    (card["number"],),
                ).fetchone()

        before = snapshot()
        changes = []
        path = f"/cards/{card['number']}/watch"
        for method, watching in (("POST", 1), ("DELETE", 0),
                                  ("DELETE", 0), ("POST", 1)):
            time.sleep(.05)
            expect(target, path, method, None, 204)
            current = snapshot()
            changes.append((tuple(a != b for a, b in zip(current[:4], before[:4])),
                            current[4]))
            assert current[4] == watching
            before = current
        return changes
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-watch-touch-") as temporary:
        base = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = base / name
            directory.mkdir()
            results[name] = check(name, directory)
        expected = [
            ((False, False, False, False), 1),
            ((True, False, True, True), 0),
            ((False, False, False, False), 0),
            ((True, False, True, True), 1),
        ]
        assert results == {"fizzy": expected, "rizzy": expected}, results
    print("Watch timestamp effects match pinned Fizzy")


if __name__ == "__main__":
    main()
