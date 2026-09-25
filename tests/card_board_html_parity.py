#!/usr/bin/env python3
"""Compare card-to-board picker and move behavior with Fizzy."""
import http.cookiejar
import json
import os
import re
import sqlite3
import time
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request


def target(name):
    return {"name":name.lower(), "url":os.environ[f"{name}_URL"],
            "account":os.environ[f"{name}_ACCOUNT"], "token":os.environ[f"{name}_TOKEN"],
            "email":os.environ[f"{name}_EMAIL"], "db":os.environ[f"{name}_DB"],
            "opener":urllib.request.build_opener(
                urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}


def check(target):
    login(target)
    account = target["account"]
    boards = []
    try:
        for label in ("First", "Second"):
            status, _, raw = request(target, f"/{account}/boards", "POST",
                                     {"board":{"name":f"{label} picker {time.time_ns()}"}}, bearer=True)
            assert status == 201, (target["name"], "board", status)
            boards.append(json.loads(raw))
        first, second = boards
        status, _, raw = request(target, f"/{account}/boards/{first['id']}/cards", "POST",
                                 {"card":{"title":"Board move parity"}}, bearer=True)
        assert status == 201, (target["name"], "card", status)
        number = json.loads(raw)["number"]
        path = f"/{account}/cards/{number}/board"
        status, headers, body = request(target, path + "/edit", accept="text/html")
        assert status == 200 and headers.get("ETag"), (target["name"], "picker", status)
        for marker in (b'id="board_picker"', b"Move this card to", first["name"].encode(), second["name"].encode()):
            assert marker in body, (target["name"], marker)
        assert f'board_id={first["id"]}'.encode() in body and f'board_id={second["id"]}'.encode() in body
        token = re.search(rb'<meta name="csrf-token" content="([^"]+)"', body)
        csrf = token.group(1).decode() if token else None
        status, _, _ = request({**target,"extra_headers":{"If-None-Match":headers["ETag"]}}, path + "/edit", accept="text/html")
        assert status == 304, (target["name"], "cached picker", status)
        status, headers, _ = request(target, path + "?board_id=" + second["id"], "POST", b"_method=patch", accept="text/html", csrf=csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == f"/{account}/cards/{number}", (target["name"], "browser move", status)
        status, _, raw = request(target, f"/{account}/cards/{number}", bearer=True)
        assert status == 200 and json.loads(raw)["board"]["id"] == second["id"], (target["name"], "moved card", status)
        status, _, body = request(target, path + "/edit", accept="text/html")
        assert status == 200 and f'board_id={second["id"]}'.encode() in body
        status, _, raw = request(target, path, "PATCH", {"board_id":first["id"]}, bearer=True)
        assert status == 200 and json.loads(raw)["board"]["id"] == first["id"], (target["name"], "JSON move", status)
        status, _, _ = request(target, path + "/edit.json", bearer=True)
        assert status == 406, (target["name"], "JSON picker", status)
        return {"picker":200,"cache":304,"browser_move":302,"json_move":200,"json_picker":406}
    finally:
        for board in boards:
            status, _, _ = request(target, f"/{account}/boards/{board['id']}", "DELETE", bearer=True)
            assert status == 204, (target["name"], "cleanup", status)


def main():
    source_db = os.environ.get("FIZZY_DB", "upstream/storage/production.sqlite3")
    with sqlite3.connect(source_db) as db:
        email = db.execute("SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1", (int(os.environ["FIZZY_ACCOUNT"]),)).fetchone()[0]
    os.environ["FIZZY_EMAIL"] = email
    source, rust = check(target("FIZZY")), check(target("RIZZY"))
    assert source == rust
    print("Card board picker parity matched: choices, cache, browser move, JSON move")


if __name__ == "__main__":
    main()
