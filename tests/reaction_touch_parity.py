#!/usr/bin/env python3
"""Compare stored reaction timestamp side effects with pinned Fizzy."""

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
                          {"board": {"name": "Reaction touch"}}, 201)
        card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                         {"card": {"title": "React to me"}}, 201)
        comment, _ = expect(target, f"/cards/{card['number']}/comments", "POST",
                            {"comment": {"body": "React here too"}}, 201)
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
                ).fetchone()[0]
                return (*card_times, comment_time)

        before = snapshot()
        changes = {}
        for kind, path in (
            ("card", f"/cards/{card['number']}/reactions"),
            ("comment", f"/cards/{card['number']}/comments/{comment['id']}/reactions"),
        ):
            time.sleep(.05)
            reaction, _ = expect(target, path, "POST",
                                 {"reaction": {"content": "🎉"}}, 201)
            current = snapshot()
            changes[kind + " create"] = tuple(a != b for a, b in zip(current, before))
            before = current
            time.sleep(.05)
            expect(target, f"{path}/{reaction['id']}", "DELETE", None, 204)
            current = snapshot()
            changes[kind + " delete"] = tuple(a != b for a, b in zip(current, before))
            before = current
        return changes
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-reaction-touch-") as temporary:
        base = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = base / name
            directory.mkdir()
            results[name] = check(name, directory)
        expected = {
            "card create": (True, True, True, False),
            "card delete": (True, False, True, False),
            "comment create": (True, True, True, True),
            "comment delete": (True, False, True, True),
        }
        assert results == {"fizzy": expected, "rizzy": expected}, results
    print("Reaction timestamp effects match pinned Fizzy")


if __name__ == "__main__":
    main()
