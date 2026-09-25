#!/usr/bin/env python3
"""Compare the account navigation menu and conditional caching with Fizzy."""
import json
import os
import sqlite3
import time

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
    status, _, _ = request({**target,"extra_headers":{"If-None-Match":etag}}, path, accept="text/html")
    assert status == 304, (target["name"], "unchanged menu", status)

    board_name = f"Menu parity {time.time_ns()}"
    status, _, raw = request(target, f"/{account}/boards", "POST", {"board":{"name":board_name}}, bearer=True)
    assert status == 201, (target["name"], "create board", status)
    board = json.loads(raw)
    try:
        status, changed_headers, changed_body = request({**target,"extra_headers":{"If-None-Match":etag}}, path, accept="text/html")
        assert status == 200 and changed_headers["ETag"] != etag, (target["name"], "updated menu", status)
        assert board_name.encode() in changed_body, (target["name"], "board in menu")
        assert f"/{account}/boards/{board['id']}".encode() in changed_body, (target["name"], "board link")
    finally:
        status, _, _ = request(target, f"/{account}/boards/{board['id']}", "DELETE", bearer=True)
        assert status == 204, (target["name"], "delete board", status)
    status, _, _ = request(target, path, accept="application/json")
    assert status == 406, (target["name"], "JSON menu", status)
    return {"show":200,"unchanged":304,"board_changed":200,"json":406}


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
