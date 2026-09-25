#!/usr/bin/env python3
"""Compare card and comment reaction forms with Fizzy."""
import http.cookiejar
import json
import os
import sqlite3
import time
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
        status, _, raw = request(target, f"/{account}/boards", "POST",
                                 {"board":{"name":f"Reaction parity {time.time_ns()}"}}, bearer=True)
        assert status == 201, (target["name"], "board", status)
        board_id = json.loads(raw)["id"]
        boards.append(board_id)
        status, _, raw = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                 {"card":{"title":"Reaction picker"}}, bearer=True)
        assert status == 201, (target["name"], "card", status)
        number = json.loads(raw)["number"]
        status, _, raw = request(target, f"/{account}/cards/{number}/comments", "POST",
                                 {"comment":{"body":"React to this"}}, bearer=True)
        assert status == 201, (target["name"], "comment", status)
        comment_id = json.loads(raw)["id"]
        paths = [f"/{account}/cards/{number}/reactions",
                 f"/{account}/cards/{number}/comments/{comment_id}/reactions"]
        for path in paths:
            status, _, body = request(target, path + "/new", accept="text/html")
            assert status == 200, (target["name"], "new", path, status)
            for marker in (b"turbo-frame", b"reaction[content]", b"New reaction", b"Add a reaction", b"Submit", b"Cancel", "👍".encode()):
                assert marker in body, (target["name"], path, marker)
            if target["name"] == "rizzy":
                assert request(target, path + "/new.json", bearer=True)[0] == 406
            status, _, raw = request(target, path, "POST", {"reaction":{"content":"👍"}}, bearer=True)
            assert status == 201 and json.loads(raw)["content"] == "👍", (target["name"], "create", path, status)
            status, _, raw = request(target, path, bearer=True)
            assert status == 200 and any(item["content"] == "👍" for item in json.loads(raw))
        if target["name"] == "rizzy":
            status, _, body = request(target, f"/{account}/cards/{number}", accept="text/html")
            assert status == 200 and all((path + "/new").encode() in body for path in paths)
        return {"card_form":200,"comment_form":200,"create":201}
    finally:
        for board_id in boards:
            status, _, _ = request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)
            assert status == 204, (target["name"], "cleanup", status)


def main():
    with sqlite3.connect(os.environ.get("FIZZY_DB", "upstream/storage/production.sqlite3")) as db:
        email = db.execute("SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
                           "JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? "
                           "AND u.role='owner' LIMIT 1", (int(os.environ["FIZZY_ACCOUNT"]),)).fetchone()[0]
    os.environ["FIZZY_EMAIL"] = email
    source, rust = check(target("FIZZY")), check(target("RIZZY"))
    assert source == rust
    print("Reaction form parity matched: card and comment forms, JSON create and listing")


if __name__ == "__main__":
    main()
