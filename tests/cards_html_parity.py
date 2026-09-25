#!/usr/bin/env python3
"""Compare all-cards browser filtering and page boundaries with Fizzy."""
import http.cookiejar
import json
import os
import sqlite3
import time
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request


def check(target):
    login(target)
    account = target["account"]
    marker = f"cardshtml{time.time_ns()}"
    status, _, raw = request(target, f"/{account}/boards", "POST", {"board": {"name": marker}}, bearer=True)
    assert status == 201, (target["name"], status, raw[:160])
    board_id = json.loads(raw)["id"]
    try:
        for index in range(17):
            status, _, raw = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                     {"card": {"title": f"{marker} card {index:02d}"}}, bearer=True)
            assert status == 201, (target["name"], index, status, raw[:160])
        status, _, raw = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                 {"card": {"title": f"{marker} completed"}}, bearer=True)
        assert status == 201, (target["name"], status)
        number = json.loads(raw)["number"]
        status, _, raw = request(target, f"/{account}/cards/{number}/closure", "POST", bearer=True)
        assert status == 204, (target["name"], status, raw[:160])
        base = f"/{account}/cards?" + urllib.parse.urlencode({"board_ids[]": board_id})

        def html(suffix=""):
            status, headers, raw = request(target, base + suffix, accept="text/html")
            assert status == 200 and headers.get_content_type() == "text/html", (target["name"], suffix, status, raw[:160])
            body = raw.decode()
            assert marker in body and "cards--grid" in body and "cards_container" in body
            return body

        def found(body):
            return [index for index in range(17) if f"{marker} card {index:02d}" in body]

        first = found(html())
        second = found(html("&page=2"))
        closed = html("&indexed_by=closed")
        term = found(html("&terms%5B%5D=" + urllib.parse.quote(marker + " card 05")))
        oldest = found(html("&sorted_by=oldest"))
        assert first == list(range(2, 17)), (target["name"], first)
        assert second == [0, 1], (target["name"], second)
        assert f"{marker} completed" in closed and not found(closed)
        assert term == [5], (target["name"], term)
        assert oldest == list(range(15)), (target["name"], oldest)
        json_path = f"/{account}/cards.json?board_ids%5B%5D={urllib.parse.quote(board_id)}"
        status, _, raw = request(target, json_path, bearer=True)
        assert status == 200 and len(json.loads(raw)) == 15, (target["name"], status)
        return first, second, term, oldest
    finally:
        status, _, _ = request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)
        assert status == 204, (target["name"], status)


def main():
    source_db = os.environ.get("FIZZY_DB_PATH", "upstream/storage/production.sqlite3")
    with sqlite3.connect(source_db) as db:
        email = db.execute(
            "SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
            (int(os.environ["FIZZY_ACCOUNT"]),),
        ).fetchone()[0]
    targets = []
    for name in ("FIZZY", "RIZZY"):
        target = {"name": name.lower(), "url": os.environ[f"{name}_URL"],
                  "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
                  "email": email if name == "FIZZY" else os.environ["RIZZY_EMAIL"],
                  "db": source_db,
                  "opener": urllib.request.build_opener(
                      urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
        if name == "FIZZY":
            target["extra_headers"] = {"X-Forwarded-For": f"198.51.100.{30 + time.time_ns() % 200}"}
        targets.append(target)
    left, right = (check(target) for target in targets)
    assert left == right, (left, right)
    print("All-cards HTML parity passed: first and second pages, status, term, sort, and JSON")


if __name__ == "__main__":
    main()
