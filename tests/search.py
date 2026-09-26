#!/usr/bin/env python3
"""Verify full-text search indexing across cards and comments."""
import os
import re
import sqlite3
import subprocess
import tempfile
import urllib.request
from pathlib import Path
from urllib.parse import quote

from auth import BINARY, ROOT, call as session_call, cookie, request_code, start, stop
from join import call


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{Path(temp) / 'search.sqlite3'}", "RIZZY_SEED_EMAIL": "search@example.com"}
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

            pending, code = request_code(port, "search@example.com")
            _, headers = session_call(port, "/session/magic_link", "POST", {"code": code}, [pending])
            session = cookie(headers, "session_token")

            def browser_search(term, page=None):
                path = f"/{account}/search?q={quote(term)}"
                if page is not None:
                    path += f"&page={page}"
                request = urllib.request.Request(
                    f"http://127.0.0.1:{port}{path}",
                    headers={"Accept": "text/html", "Cookie": session},
                )
                with urllib.request.urlopen(request, timeout=3) as response:
                    assert response.status == 200
                    return response.read().decode()

            assert 'header__title">Search</h1>' in browser_search("")
            title_html = browser_search("broken")
            assert "Layout is <mark" in title_html and "Look at this mess" in title_html
            comment_html = browser_search("overflowing")
            assert "search__excerpt--comment" in comment_html and "text" in comment_html
            haggis_html = browser_search("haggis")
            assert haggis_html.count('class="search__title"') == 2
            assert 'class="circled-text"><span></span>haggis</mark>' in haggis_html
            assert 'data-controller="auto-submit"' in browser_search(str(first["number"]))
            assert 'search__blank-slate' in browser_search("999999")
            call(port, f"/{account}/boards/{board_id}/cards", "POST", {"card": {"title": "<b>Bold</b> testing content"}}, bearer=token, status=201)
            escaped_html = browser_search("testing")
            assert "&lt;b&gt;Bold&lt;/b&gt;" in escaped_html
            assert '<mark class="circled-text"><span></span>testing</mark>' in escaped_html
            filler = " ".join(["word"] * 20)
            call(port, f"/{account}/boards/{board_id}/cards", "POST", {"card": {"title": "Long winded", "description": f"{filler} needle {filler}"}}, bearer=token, status=201)
            long_excerpt = re.search(r'<div class="search__excerpt">(.*?)</div>', browser_search("needle")).group(1)
            assert long_excerpt.startswith("... ") and long_excerpt.endswith(" ...")
            assert "needle" in long_excerpt
            for i in range(20):
                call(port, f"/{account}/boards/{board_id}/cards", "POST", {"card": {"title": f"paginated card {i}"}}, bearer=token, status=201)
            assert browser_search("paginated").count('class="search__title"') == 15
            assert 'id="filtered_search_results-pagination-link-2"' in browser_search("paginated")
            assert browser_search("paginated", 2).count('class="search__title"') == 5
            assert browser_search("paginated", "abc").count('class="search__title"') == 15

            call(port, f"/{account}/cards/{first['number']}", "PUT", {"card": {"title": "Repaired layout"}}, bearer=token)
            with sqlite3.connect(Path(temp) / "search.sqlite3") as db:
                assert db.execute("SELECT title FROM card_search WHERE rowid=(SELECT rowid FROM cards WHERE id=?)", (first["id"],)).fetchone()[0] == "Repaired layout"
            assert [card["id"] for card in search("repaired")] == [first["id"]]
            call(port, f"/{account}/cards/{second['number']}/comments/{comment['id']}", "DELETE", bearer=token, status=204)
            assert search("overflowing") == []
            call(port, f"/{account}/cards/{second['number']}", "DELETE", bearer=token, status=204)
            assert search("haggis") == []
            print("Search passed: JSON indexing and browser hits, highlights, redirect, empty state, and pagination")
        finally:
            stop(server)


if __name__ == "__main__":
    main()
