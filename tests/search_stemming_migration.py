#!/usr/bin/env python3
"""Upgrade an existing Unicode FTS index to Porter without losing search rows."""

import os
import re
import sqlite3
import subprocess
import tempfile
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from join import call


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "search.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True).stdout
        account = re.search(r"Account: (\d+)", seeded).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded).group(1)
        port, server = start(env)
        try:
            board, _ = call(port, f"/{account}/boards", "POST",
                            {"board": {"name": "Search migration"}}, bearer=token, status=201)
            card, _ = call(port, f"/{account}/boards/{board['id']}/cards", "POST",
                           {"card": {"title": "Running"}}, bearer=token, status=201)
            call(port, f"/{account}/cards/{card['number']}/comments", "POST",
                 {"comment": {"body": "Walking"}}, bearer=token, status=201)
        finally:
            stop(server)

        with sqlite3.connect(database) as db:
            db.executescript("""
                DROP TRIGGER cards_search_insert;
                DROP TRIGGER cards_search_update;
                DROP TRIGGER cards_search_delete;
                DROP TRIGGER comments_search_insert;
                DROP TRIGGER comments_search_update;
                DROP TRIGGER comments_search_delete;
                DROP TABLE card_search;
                DROP TABLE comment_search;
                CREATE VIRTUAL TABLE card_search USING fts5(title,description);
                CREATE VIRTUAL TABLE comment_search USING fts5(body);
                INSERT INTO card_search(rowid,title,description)
                  SELECT rowid,title,search_text FROM cards WHERE status='published';
                INSERT INTO comment_search(rowid,body)
                  SELECT m.rowid,m.search_text FROM comments m JOIN cards c ON c.id=m.card_id
                  WHERE c.status='published';
            """)
            before = (db.execute("SELECT COUNT(*) FROM card_search").fetchone()[0],
                      db.execute("SELECT COUNT(*) FROM comment_search").fetchone()[0])
            assert db.execute("SELECT COUNT(*) FROM card_search WHERE card_search MATCH 'run'").fetchone()[0] == 0
            assert db.execute("SELECT COUNT(*) FROM comment_search WHERE comment_search MATCH 'walk'").fetchone()[0] == 0

        port, server = start(env)
        try:
            for term in ("run", "walk"):
                matches, _ = call(port, f"/{account}/search?q={term}", bearer=token)
                assert [item["id"] for item in matches] == [card["id"]], term
            with sqlite3.connect(database) as db:
                after = (db.execute("SELECT COUNT(*) FROM card_search").fetchone()[0],
                         db.execute("SELECT COUNT(*) FROM comment_search").fetchone()[0])
                assert before == after, (before, after)
                assert "porter unicode61" in db.execute(
                    "SELECT sql FROM sqlite_master WHERE name='card_search'").fetchone()[0]
        finally:
            stop(server)
        port, server = start(env)
        try:
            matches, _ = call(port, f"/{account}/search?q=run", bearer=token)
            assert [item["id"] for item in matches] == [card["id"]]
        finally:
            stop(server)
    print("Search Porter migration retained card and comment rows and enabled stemmed queries")


if __name__ == "__main__":
    main()
