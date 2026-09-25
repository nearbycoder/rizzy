#!/usr/bin/env python3
"""Verify full-text search indexing across cards and comments."""
import os
import re
import sqlite3
import subprocess
import tempfile
from pathlib import Path
from urllib.parse import quote

from auth import BINARY, ROOT, start, stop
from join import call


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{Path(temp) / 'search.sqlite3'}"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded.stdout).group(1)
        port, server = start(env)
        try:
            board, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Search"}}, bearer=token, status=201)
            board_id = board["id"]

            def search(term):
                result, _ = call(port, f"/{account}/search?q={quote(term)}", bearer=token)
                return result

            first, _ = call(port, f"/{account}/boards/{board_id}/cards", "POST", {"card": {"title": "Layout is broken", "description": "Look at this mess"}}, bearer=token, status=201)
            second, _ = call(port, f"/{account}/boards/{board_id}/cards", "POST", {"card": {"title": "Just haggis", "description": "More haggis"}}, bearer=token, status=201)
            comment, _ = call(port, f"/{account}/cards/{second['number']}/comments", "POST", {"comment": {"body": "I love haggis and overflowing text"}}, bearer=token, status=201)
            assert [card["id"] for card in search("broken")] == [first["id"]]
            assert [card["id"] for card in search("overflowing")] == [second["id"]]
            assert [card["id"] for card in search("haggis")] == [second["id"]]
            assert [card["id"] for card in search('"Layout is broken"')] == [first["id"]]
            assert [card["id"] for card in search(str(first["number"]))] == [first["id"]]
            call(port, f"/{account}/cards/{first['number']}", "PUT", {"card": {"title": "Repaired layout"}}, bearer=token)
            with sqlite3.connect(Path(temp) / "search.sqlite3") as db:
                assert db.execute("SELECT title FROM card_search WHERE rowid=(SELECT rowid FROM cards WHERE id=?)", (first["id"],)).fetchone()[0] == "Repaired layout"
            assert [card["id"] for card in search("repaired")] == [first["id"]]
            call(port, f"/{account}/cards/{second['number']}/comments/{comment['id']}", "DELETE", bearer=token, status=204)
            assert search("overflowing") == []
            call(port, f"/{account}/cards/{second['number']}", "DELETE", bearer=token, status=204)
            assert search("haggis") == []
            print("Search passed: FTS title, phrase, comment, deduplication, update, and delete")
        finally:
            stop(server)


if __name__ == "__main__":
    main()
