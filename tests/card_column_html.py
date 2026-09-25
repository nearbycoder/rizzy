#!/usr/bin/env python3
"""Check the browser card column chooser and its state controls."""
import os
import re
import subprocess
import tempfile
from pathlib import Path

from auth import BINARY, ROOT, call as auth_call, cookie, request_code, start, stop
from join import call


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{Path(temp) / 'columns.sqlite3'}"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT,
                                env={**env, "RIZZY_SEED_EMAIL": "column-owner@example.test"},
                                capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded.stdout).group(1)
        port, server = start(env)
        try:
            board, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Column chooser"}}, bearer=token, status=201)
            board_id = board["id"]
            column, _ = call(port, f"/{account}/boards/{board_id}/columns", "POST",
                             {"column": {"name": "Working on", "color": "#25a864"}}, bearer=token, status=201)
            card, _ = call(port, f"/{account}/boards/{board_id}/cards", "POST",
                           {"card": {"title": "Move me"}}, bearer=token, status=201)
            number = card["number"]
            pending, code = request_code(port, "column-owner@example.test")
            _, headers = auth_call(port, "/session/magic_link", "POST", {"code": code}, [pending])
            session = cookie(headers, "session_token")
            path = f"/{account}/cards/{number}/column/edit"

            def chooser():
                html, _ = call(port, path, cookies=[session], accept="text/html")
                assert f'id="card_{card["id"]}_columns"' in html
                assert 'role="radiogroup"' in html
                return html

            html = chooser()
            for label in ("Not now", "Maybe?", "Working on", "Done"):
                assert label in html
            assert 'data-stage="maybe" disabled' in html
            assert f'data-column-id="{column["id"]}"' in html
            card_page, _ = call(port, f"/{account}/cards/{number}", cookies=[session], accept="text/html")
            assert 'id="choose-column"' in card_page and 'id="column-picker"' in card_page
            call(port, f"/{account}/cards/{number}/triage", "POST", {"column_id": column["id"]}, bearer=token, status=204)
            assert f'data-column-id="{column["id"]}" disabled' in chooser()
            call(port, f"/{account}/cards/{number}/not_now", "POST", bearer=token, status=204)
            assert 'data-stage="not_now" disabled' in chooser()
            call(port, f"/{account}/cards/{number}/closure", "POST", bearer=token, status=204)
            assert 'data-stage="done" disabled' in chooser()
            call(port, path + ".json", cookies=[session], status=406)
            call(port, f"/9999/cards/{number}/column/edit", cookies=[session], accept="text/html", status=401)
        finally:
            stop(server)
    print("Card column chooser passed: options, current stage, HTML entry, and access")


if __name__ == "__main__":
    main()
