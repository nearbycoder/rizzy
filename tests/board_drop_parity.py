#!/usr/bin/env python3
"""Compare the four board drag-and-drop transitions with the pinned Fizzy app."""
import http.cookiejar
import json
import os
import re
import sqlite3
import time
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request


def state(target, number):
    status, _, body = request(target, f"/{target['account']}/cards/{number}.json", bearer=True)
    assert status == 200, (status, body[:200])
    card = json.loads(body)
    return card["closed"], card["postponed"], card.get("column", {}).get("id")


def run(target):
    login(target)
    account = target["account"]
    status, _, raw = request(target, f"/{account}/boards", "POST", {"board": {"name": f"Drop parity {time.time_ns()}"}}, bearer=True)
    assert status == 201, (status, raw[:200])
    board = json.loads(raw)
    path = f"/{account}/boards/{board['id']}"
    try:
        status, _, raw = request(target, path + "/columns", "POST", {"column": {"name": "Doing"}}, bearer=True)
        assert status == 201, (status, raw[:200])
        column = json.loads(raw)
        status, _, raw = request(target, path + "/cards", "POST", {"card": {"title": "Drag me"}}, bearer=True)
        assert status == 201, (status, raw[:200])
        number = json.loads(raw)["number"]
        status, _, html = request(target, path, accept="text/html")
        assert status == 200, status
        token = re.search(rb'<meta name="csrf-token" content="([^"]+)"', html)
        csrf = token.group(1).decode() if token else None
        assert state(target, number) == (False, False, None)
        transitions = [
            ("column", f"?column_id={urllib.parse.quote(column['id'])}", (False, False, column["id"]), "column_" + column["id"]),
            ("not_now", "", (False, True, None), "not-now"),
            ("stream", "", (False, False, None), "maybe"),
            ("closure", "", (True, False, None), "closed-cards"),
        ]
        results = []
        for destination, query, expected, target_id in transitions:
            drop_path = f"/{account}/columns/cards/{number}/drops/{destination}{query}"
            status, headers, body = request(target, drop_path, "POST", b"", accept="text/vnd.turbo-stream.html", csrf=csrf)
            assert status == 200, (destination, status, body[:300])
            assert headers.get_content_type() == "text/vnd.turbo-stream.html", (destination, headers.get("Content-Type"))
            assert f'target="{target_id}"'.encode() in body, (destination, body[:300])
            actual = state(target, number)
            assert actual == expected, (destination, actual, expected)
            results.append((destination, status, (actual[0], actual[1], actual[2] == column["id"])))
        return results
    finally:
        status, _, _ = request(target, path, "DELETE", bearer=True)
        assert status == 204, status


def main():
    fizzy_db = os.environ.get("FIZZY_DB_PATH", "upstream/storage/production.sqlite3")
    with sqlite3.connect(fizzy_db) as db:
        email = db.execute(
            "SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
            (int(os.environ["FIZZY_ACCOUNT"]),),
        ).fetchone()[0]
    targets = []
    for name in ("FIZZY", "RIZZY"):
        target = {"name": name.lower(), "url": os.environ[f"{name}_URL"],
                  "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
                  "email": email if name == "FIZZY" else os.environ.get("RIZZY_EMAIL", "owner@example.com"),
                  "db": fizzy_db}
        target["opener"] = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
        targets.append(target)
    source, rust = (run(target) for target in targets)
    assert source == rust, (source, rust)
    print("Board drop parity matched: column, Not Now, Maybe, and Done")


if __name__ == "__main__":
    main()
