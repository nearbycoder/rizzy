#!/usr/bin/env python3
"""Compare the account navigation menu and conditional caching with Fizzy."""
import json
import os
import re
import sqlite3
import time
import urllib.parse

from draft_workflow_parity import login, request, NoRedirect
import http.cookiejar
import urllib.request


def target(name):
    opener = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect()
    )
    return {"name": name.lower(), "url": os.environ[f"{name}_URL"],
            "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
            "email": os.environ[f"{name}_EMAIL"], "db": os.environ[f"{name}_DB"],
            "opener": opener}


def check(target):
    login(target)
    account = target["account"]
    path = f"/{account}/my/menu"
    status, headers, body = request(target, path, accept="text/html")
    assert status == 200 and headers.get("ETag"), (target["name"], "initial menu", status)
    for label in (b"my_menu", b"Boards", b"Tags", b"People", b"Settings", b"Shortcuts",
                  b"Home", b"Assigned to me", b"Added by me", b"My Profile"):
        assert label in body, (target["name"], label)
    etag = headers["ETag"]
    status, not_modified_headers, _ = request({**target,"extra_headers":{"If-None-Match":etag}}, path, accept="text/html")
    assert status == 304, (target["name"], "unchanged menu", status)
    if target["name"] == "rizzy":
        nonce = re.search(rb'<script nonce="([^"]+)"', body)
        assert nonce and f"'nonce-{nonce.group(1).decode()}'" in headers["Content-Security-Policy"], (
            "menu script CSP", headers)
        assert "Content-Security-Policy" not in not_modified_headers, (
            "cached menu CSP was replaced", not_modified_headers)

    board_name = f"Menu parity {time.time_ns()}"
    status, _, raw = request(target, f"/{account}/boards", "POST", {"board":{"name":board_name}}, bearer=True)
    assert status == 201, (target["name"], "create board", status)
    board = json.loads(raw)
    try:
        status, changed_headers, changed_body = request({**target,"extra_headers":{"If-None-Match":etag}}, path, accept="text/html")
        assert status == 200 and changed_headers["ETag"] != etag, (target["name"], "updated menu", status)
        assert board_name.encode() in changed_body, (target["name"], "board in menu")
        assert f"/{account}/boards/{board['id']}".encode() in changed_body, (target["name"], "board link")
        status, _, raw = request(target, f"/{account}/boards/{board['id']}/cards", "POST",
                                 {"card":{"title":"Saved menu card"}}, bearer=True)
        assert status == 201
        number = json.loads(raw)["number"]
        status, _, _ = request(target, f"/{account}/cards/{number}/taggings", "POST",
                               {"tag_title":"menufilter"}, bearer=True)
        assert status == 204
        status, _, raw = request(target, f"/{account}/tags", bearer=True)
        assert status == 200
        tag_id = next(tag["id"] for tag in json.loads(raw) if tag["title"] == "menufilter")
        status, _, board_html = request(target, f"/{account}/boards/{board['id']}", accept="text/html")
        assert status == 200
        status, menu_before_save, _ = request(target, path, accept="text/html")
        assert status == 200
        with sqlite3.connect(target["db"]) as db:
            filters_before = db.execute("SELECT COUNT(*) FROM filters").fetchone()[0]
        csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', board_html)
        csrf = csrf_match.group(1).decode() if csrf_match else None
        fields = urllib.parse.urlencode({"board_ids[]":board["id"], "tag_ids[]":tag_id}).encode()
        status, _, stream = request(target, f"/{account}/filters", "POST", fields,
                                    accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200 and b"Delete custom view" in stream, (target["name"], "save view", status)
        match = re.search(rb'/filters/([a-z0-9]+)', stream)
        assert match, (target["name"], "saved view ID", stream[:250])
        filter_id = match.group(1).decode()
        status, filter_headers, filtered_menu = request(target, path, accept="text/html")
        assert status == 200 and board_name.encode() in filtered_menu and b"#menufilter" in filtered_menu
        with sqlite3.connect(target["db"]) as db:
            filter_count = db.execute("SELECT COUNT(*) FROM filters").fetchone()[0]
        assert filter_count == filters_before + 1 and filter_headers["ETag"] != menu_before_save["ETag"]
        assert f"/cards?filter_id={filter_id}".encode() not in filtered_menu
        assert b"Custom views" not in filtered_menu
        status, _, _ = request(target, f"/{account}/filters/{filter_id}", "DELETE", b"",
                               accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200, (target["name"], "delete view", status)
        status, cleared_headers, cleared_menu = request(target, path, accept="text/html")
        assert status == 200 and cleared_headers["ETag"] != filter_headers["ETag"]
        assert f"/cards?filter_id={filter_id}".encode() not in cleared_menu
        with sqlite3.connect(target["db"]) as db:
            assert db.execute("SELECT COUNT(*) FROM filters").fetchone()[0] == filters_before
    finally:
        status, _, _ = request(target, f"/{account}/boards/{board['id']}", "DELETE", bearer=True)
        assert status == 204, (target["name"], "delete board", status)
    status, _, _ = request(target, path, accept="application/json")
    assert status == 406, (target["name"], "JSON menu", status)
    return {"show":200,"unchanged":304,"board_changed":200,"saved_view":200,"json":406}


def main():
    db = os.environ.get("FIZZY_DB", "upstream/storage/production.sqlite3")
    with sqlite3.connect(db) as connection:
        email = connection.execute("SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1", (int(os.environ["FIZZY_ACCOUNT"]),)).fetchone()[0]
    os.environ["FIZZY_EMAIL"] = email
    source, rust = check(target("FIZZY")), check(target("RIZZY"))
    assert source == rust
    print("My menu parity matched: content, ETag reuse, board invalidation, and JSON rejection")


if __name__ == "__main__":
    main()
