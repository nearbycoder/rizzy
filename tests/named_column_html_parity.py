#!/usr/bin/env python3
"""Compare the authenticated named-column page and its card pagination."""

import http.cookiejar
import os
import re
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, start_server,
                             stop_server)  # noqa: E402
from board_special_column_etag_parity import html_request  # noqa: E402
from draft_workflow_parity import NoRedirect, login  # noqa: E402


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    if name == "rizzy":
        env["RIZZY_DEV_MAGIC_LINKS"] = "1"
    process, url = start_server(name, directory, env,
                                sorted(os.sched_getaffinity(0))[:2])
    try:
        target = {"name": name, "url": url, "account": str(credentials["account"]),
                  "token": credentials["token"], "email": "owner@example.com",
                  "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                  "opener": urllib.request.build_opener(
                      urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
        login(target)
        board, _ = expect(target, "/boards", "POST",
                          {"board": {"name": "Browser column"}}, 201)
        column, _ = expect(target, f"/boards/{board['id']}/columns", "POST",
                           {"column": {"name": "In progress"}}, 201)
        path = f"/boards/{board['id']}/columns/{column['id']}"
        frame = f"cards_column_{column['id']}"
        status, _, empty = html_request(target, path)
        assert status == 200 and b"No cards here" in empty, (name, status)
        assert f'<turbo-frame id="{frame}">'.encode() in empty, name
        assert b'data-drag-drop-item-container' in empty, name
        for index in range(16):
            card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                             {"card": {"title": f"Browser column card {index:02d}"}}, 201)
            expect(target, f"/cards/{card['number']}/triage", "POST",
                   {"column_id": column["id"]}, 204)
        status, first_headers, first = html_request(target, path)
        assert status == 200 and first.count(b'class="card__title') == 15, (name, status)
        assert f'<turbo-frame id="{frame}">'.encode() in first, name
        assert f'id="{frame}-pagination-list"'.encode() in first, name
        assert f'id="{frame}-pagination-contents-1"'.encode() in first, name
        assert f'id="{frame}-pagination-link-2"'.encode() in first, name
        assert re.search(rb'href="[^"]+\?page=2"', first), name
        assert b'data-drag-drop-item-container' in first, name
        assert first_headers.get("X-Total-Count") is None, name
        etag = first_headers.get("ETag")
        assert etag, name
        cached_status, cached_headers, cached_body = html_request(target, path, etag)
        assert cached_status == 304 and not cached_body, (name, cached_status)
        assert cached_headers.get("X-Total-Count") is None, name
        status, second_headers, second = html_request(target, path + "?page=2")
        assert status == 200 and second.count(b'class="card__title') == 1, (name, status)
        assert f'id="{frame}-pagination-contents-2"'.encode() in second, name
        assert f'id="{frame}-pagination-link-3"'.encode() not in second, name
        assert second_headers.get("X-Total-Count") is None, name
        return {"empty": True, "first": 15, "second": 1,
                "frame": bool(re.search(rb'<turbo-frame id="cards_column_', first))}
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-named-column-html-") as temporary:
        root = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = root / name
            directory.mkdir()
            results[name] = check(name, directory)
        assert results["fizzy"] == results["rizzy"], results
    print("Named-column HTML structure and two-page previews matched pinned Fizzy")


if __name__ == "__main__":
    main()
