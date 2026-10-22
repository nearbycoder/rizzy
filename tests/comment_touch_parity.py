#!/usr/bin/env python3
"""Compare comment write timestamp effects with pinned Fizzy."""

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
                          {"board": {"name": "Comment touch"}}, 201)
        card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                         {"card": {"title": "Comment on me"}}, 201)
        database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")

        def snapshot():
            with sqlite3.connect(database) as db:
                card_times = db.execute(
                    "SELECT c.updated_at,c.last_active_at,b.updated_at "
                    "FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.number=?",
                    (card["number"],),
                ).fetchone()
                comment_time = db.execute(
                    "SELECT updated_at FROM comments WHERE card_id="
                    "(SELECT id FROM cards WHERE number=?) ORDER BY created_at DESC LIMIT 1",
                    (card["number"],),
                ).fetchone()
                return (*card_times, comment_time[0] if comment_time else None)

        before = snapshot()
        changes = {}
        time.sleep(.05)
        comment, _ = expect(target, f"/cards/{card['number']}/comments", "POST",
                            {"comment": {"body": "Initial"}}, 201)
        path = f"/cards/{card['number']}/comments/{comment['id']}"
        for action, method, payload, status in (
            ("create", None, None, None),
            ("edit", "PATCH", {"comment": {"body": "Edited"}}, 200),
            ("repeat", "PATCH", {"comment": {"body": "Edited"}}, 200),
            ("empty", "PATCH", {"comment": {"body": ""}}, 200),
            ("null", "PATCH", {"comment": {"body": None}}, 200),
            ("repeat null", "PATCH", {"comment": {"body": None}}, 200),
            ("delete", "DELETE", None, 204),
        ):
            if method:
                time.sleep(.05)
                expect(target, path, method, payload, status)
            current = snapshot()
            changes[action] = tuple(a != b for a, b in zip(current, before))
            before = current
        return changes
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-comment-touch-") as temporary:
        base = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = base / name
            directory.mkdir()
            results[name] = check(name, directory)
        expected = {
            "create": (True, True, True, True),
            "edit": (True, False, True, True),
            "repeat": (False, False, False, False),
            "empty": (True, False, True, True),
            "null": (True, False, True, True),
            "repeat null": (False, False, False, False),
            "delete": (True, False, True, True),
        }
        assert results == {"fizzy": expected, "rizzy": expected}, results
    print("Comment timestamp effects match pinned Fizzy")


if __name__ == "__main__":
    main()
