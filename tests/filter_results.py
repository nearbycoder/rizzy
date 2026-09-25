#!/usr/bin/env python3
"""Exercise saved and ad hoc card filters, FTS terms, and closure windows."""
import os
import re
import sqlite3
import subprocess
import tempfile
import urllib.parse
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from filters import turbo
from join import call


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "filter-results.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        seed = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seed.stdout).group(1)
        token = re.search(r"Bearer token: (\w+)", seed.stdout).group(1)
        port, server = start(env)
        try:
            board, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Filter results"}}, bearer=token, status=201)
            cards = []
            for title in ("haggis neeps", "haggis", "neeps"):
                card, _ = call(port, f"/{account}/boards/{board['id']}/cards", "POST", {"card": {"title": title}}, bearer=token, status=201)
                cards.append(card)
            path = f"/{account}/cards"

            def titles(params):
                query = urllib.parse.urlencode(params, doseq=True)
                data, _ = call(port, path + "?" + query, bearer=token)
                return {card["title"] for card in data}

            assert titles({"board_ids[]": [board["id"]], "terms[]": ["haggis", "neeps"]}) == {"haggis neeps"}
            assert titles({"terms[]": ["haggis"]}) == {"haggis neeps", "haggis"}
            call(port, f"/{account}/cards/{cards[1]['number']}/comments", "POST", {"comment": {"body": "neeps in comment"}}, bearer=token, status=201)
            assert titles({"terms[]": ["haggis", "neeps"]}) == {"haggis neeps"}
            assert titles({"creation": "this week", "board_ids[]": [board["id"]]}) == {"haggis neeps", "haggis", "neeps"}
            with sqlite3.connect(database) as db:
                db.execute("UPDATE cards SET created_at='2024-01-01T00:00:00.000Z' WHERE id=?", (cards[2]["id"],))
                owner_id = db.execute("SELECT id FROM users WHERE role='owner'").fetchone()[0]
            assert titles({"creation": "this week", "board_ids[]": [board["id"]]}) == {"haggis neeps", "haggis"}
            call(port, f"/{account}/cards/{cards[0]['number']}/closure", "POST", bearer=token, status=204)
            assert titles({"closure": "today", "board_ids[]": [board["id"]]}) == {"haggis neeps"}
            assert titles({"closer_ids[]": [owner_id]}) == {"haggis neeps"}
            assert titles({"indexed_by": "closed"}) == {"haggis neeps"}
            assert titles({"indexed_by": "draft"}) == set()
            turbo(port, f"/{account}/filters", token, params={"indexed_by": "closed", "board_ids[]": [board["id"]], "terms[]": ["haggis"]})
            with sqlite3.connect(database) as db:
                filter_id = db.execute("SELECT id FROM filters").fetchone()[0]
            assert titles({"filter_id": filter_id}) == {"haggis neeps"}
            assert titles({"filter_id": filter_id, "indexed_by": "all"}) == {"haggis neeps"}
            call(port, path + "?filter_id=missing", bearer=token, status=404)
            with sqlite3.connect(database) as db:
                db.execute("DELETE FROM closures WHERE card_id=?", (cards[0]["id"],))
            stop(server)
            port, server = start(env)
            assert titles({"closer_ids[]": [owner_id]}) == {"haggis neeps"}
            call(port, f"/{account}/cards/{cards[0]['number']}/closure", "DELETE", bearer=token, status=204)
            assert titles({"closure": "today"}) == set()
        finally:
            stop(server)
    print("card filter result checks passed")


if __name__ == "__main__":
    main()
