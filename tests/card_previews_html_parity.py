#!/usr/bin/env python3
"""Compare working source card preview pages and exercise Rust's next link."""
import http.cookiejar
import json
import os
import sqlite3
import time
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request


def run(target):
    login(target)
    account = target["account"]
    marker = "previewpage" + str(time.time_ns())
    status, _, raw = request(target, f"/{account}/boards", "POST", {"board": {"name": marker}}, bearer=True)
    assert status == 201, (target["name"], status, raw[:200])
    board_id = json.loads(raw)["id"]
    try:
        def create(index):
            status, _, raw = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                     {"card": {"title": f"{marker} card {index:02d}"}}, bearer=True)
            assert status == 201, (target["name"], index, status, raw[:200])

        for index in range(15):
            create(index)

        def page(number):
            params = urllib.parse.urlencode({"target": "preview-cards", "board_ids[]": board_id, "page": number})
            status, headers, body = request(
                target, f"/{account}/cards/previews.turbo_stream?{params}",
                accept="text/vnd.turbo-stream.html")
            assert status == 200, (target["name"], number, status, body[:200])
            assert headers.get_content_type() == "text/vnd.turbo-stream.html"
            html = body.decode()
            assert f'target="preview-cards-load-page-{number}"' in html
            assert 'action="append" target="preview-cards"' in html
            found = [index for index in range(17) if f"{marker} card {index:02d}" in html]
            return found, html

        first, _ = page(1)
        assert first == list(range(15)), (target["name"], first)
        for index in (15, 16):
            create(index)
        second, second_html = page(2)
        assert second == [0, 1], (target["name"], second)
        if target["name"] == "rizzy":
            full_first, full_html = page(1)
            assert full_first == list(range(2, 17)), full_first
            assert "preview-cards-load-page-2" in full_html
        assert "preview-cards-load-page-3" not in second_html
        return first, second
    finally:
        status, _, _ = request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)
        assert status == 204, (target["name"], status)


def main():
    names = ("RIZZY",) if os.environ.get("FIZZY_SKIP") else ("FIZZY", "RIZZY")
    fizzy_db = os.environ.get("FIZZY_DB_PATH", "upstream/storage/production.sqlite3")
    email = None
    if "FIZZY" in names:
        with sqlite3.connect(fizzy_db) as db:
            email = db.execute(
                "SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
                "JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
                (int(os.environ["FIZZY_ACCOUNT"]),),
            ).fetchone()[0]
    targets = []
    for name in names:
        target = {"name": name.lower(), "url": os.environ[f"{name}_URL"],
                  "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
                  "email": email if name == "FIZZY" else os.environ.get("RIZZY_EMAIL", "owner@example.com"),
                  "db": fizzy_db,
                  "opener": urllib.request.build_opener(
                      urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
        if name == "FIZZY":
            target["extra_headers"] = {"X-Forwarded-For": f"198.51.100.{time.time_ns() % 200 + 30}"}
        targets.append(target)
    results = [run(target) for target in targets]
    if len(results) == 2:
        assert results[0] == results[1], results
    print("Card preview Turbo pages passed: 15 cards, 2 more, and Rust next-page link")


if __name__ == "__main__":
    main()
