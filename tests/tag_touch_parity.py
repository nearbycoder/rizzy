#!/usr/bin/env python3
"""Compare stored card and board touch effects for tag toggles with Fizzy."""

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
                          {"board": {"name": "Tag touch"}}, 201)
        card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                         {"card": {"title": "Tag me"}}, 201)
        database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")

        def snapshot():
            with sqlite3.connect(database) as db:
                return db.execute(
                    "SELECT c.updated_at,c.last_active_at,b.updated_at "
                    "FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.number=?",
                    (card["number"],),
                ).fetchone()

        before = snapshot()
        changes = []
        for expected_tags in (["parity-tag"], []):
            time.sleep(.04)
            expect(target, f"/cards/{card['number']}/taggings", "POST",
                   {"tag_title": "parity-tag"}, 204)
            current = snapshot()
            changes.append(tuple(a != b for a, b in zip(current, before)))
            shown, _ = expect(target, f"/cards/{card['number']}")
            assert shown["tags"] == expected_tags, (name, shown["tags"])
            before = current
        return changes
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-tag-touch-") as temporary:
        base = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = base / name
            directory.mkdir()
            results[name] = check(name, directory)
        assert results == {"fizzy": [(True, False, True)] * 2,
                           "rizzy": [(True, False, True)] * 2}, results
    print("Tag on/off card and board touch effects match pinned Fizzy")


if __name__ == "__main__":
    main()
