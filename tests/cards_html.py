#!/usr/bin/env python3
"""Exercise the all-cards browser grid without changing its JSON API."""
import os
import re
import subprocess
import tempfile
import urllib.parse
from pathlib import Path

from auth import BINARY, ROOT, call as auth_call, cookie, request_code, start, stop
from join import call


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{Path(temp) / 'cards-html.sqlite3'}"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT,
                                env={**env, "RIZZY_SEED_EMAIL": "cards-html@example.test"},
                                capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded.stdout).group(1)
        port, server = start(env)
        try:
            board, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "HTML card board"}}, bearer=token, status=201)
            board_id = board["id"]
            for index in range(17):
                card, _ = call(port, f"/{account}/boards/{board_id}/cards", "POST",
                               {"card": {"title": f"Grid item {index:02d}"}}, bearer=token, status=201)
                if index == 16:
                    newest = card
            column, _ = call(port, f"/{account}/boards/{board_id}/columns", "POST",
                             {"column": {"name": "In progress", "color": "var(--color-card-4)"}}, bearer=token, status=201)
            call(port, f"/{account}/cards/{newest['number']}/triage", "POST",
                 {"column_id": column["id"]}, bearer=token, status=204)
            closed, _ = call(port, f"/{account}/boards/{board_id}/cards", "POST",
                             {"card": {"title": "Completed item"}}, bearer=token, status=201)
            call(port, f"/{account}/cards/{closed['number']}/closure", "POST", bearer=token, status=204)
            second_board, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Another HTML board"}}, bearer=token, status=201)
            call(port, f"/{account}/boards/{second_board['id']}/cards", "POST",
                 {"card": {"title": "On another board"}}, bearer=token, status=201)
            pending, code = request_code(port, "cards-html@example.test")
            _, headers = auth_call(port, "/session/magic_link", "POST", {"code": code}, [pending])
            session = cookie(headers, "session_token")
            base = f"/{account}/cards?" + urllib.parse.urlencode({"board_ids[]": board_id})

            def html(path):
                value, _ = call(port, path, cookies=[session], accept="text/html")
                return value

            first = html(base)
            assert '<turbo-frame id="cards_container">' in first
            assert 'id="cards_paginated_container"' in first
            assert 'HTML card board' in first and '17 cards' in first
            assert 'class="card__board"' in first and 'class="card__id"' in first
            assert 'class="card__footer"' in first and 'class="card__column-name">In progress</span>' in first
            assert 'card--color-4' in first
            assert all(f"Grid item {index:02d}" in first for index in range(2, 17))
            assert "Grid item 00" not in first and "Completed item" not in first
            assert 'data-preview-url=' in first and 'load-page-2' in first
            assert 'name="indexed_by"' in first and 'name="sorted_by"' in first
            assert f'name="board_ids[]" value="{board_id}" checked' in first
            both = html(f"/{account}/cards?board_ids%5B%5D={board_id}&board_ids%5B%5D={second_board['id']}")
            assert "18 cards" in both and "On another board" in both
            assert f'name="board_ids[]" value="{board_id}" checked' in both
            assert f'name="board_ids[]" value="{second_board["id"]}" checked' in both
            second = html(base + "&page=2")
            assert "Grid item 00" in second and "Grid item 01" in second
            assert "Grid item 16" not in second
            preview = html(f"/{account}/cards/previews.turbo_stream?target=cards_paginated_container&board_ids%5B%5D={board_id}&page=2")
            assert 'action="append" target="cards_paginated_container"' in preview
            assert "Grid item 00" in preview and "Grid item 01" in preview
            assert "load-page-3" not in preview
            done = html(base + "&indexed_by=closed")
            assert "Completed item" in done and "Grid item 16" not in done
            search = html(base + "&terms%5B%5D=Grid+item+05")
            assert "Grid item 05" in search and "Grid item 06" not in search
            browser_form_query = (
                base + "&terms%5B%5D=Grid+item+05&indexed_by=all&sorted_by=latest"
                "&tag_ids%5B%5D=&assignee_ids%5B%5D=&creator_ids%5B%5D="
                "&assignment_status=&creation=&closure="
            )
            browser_result = html(browser_form_query)
            assert "Grid item 05" in browser_result and "Grid item 06" not in browser_result
            rows, _ = call(port, base, bearer=token)
            assert isinstance(rows, list) and len(rows) == 15
            board_page = html(f"/{account}/boards/{board_id}")
            assert f'href="/{account}/cards">All cards</a>' in board_page
        finally:
            stop(server)
    print("All-cards HTML passed: filters, pagination, Turbo append, navigation, and JSON regression")


if __name__ == "__main__":
    main()
