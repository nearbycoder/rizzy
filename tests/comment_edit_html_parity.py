#!/usr/bin/env python3
"""Compare comment edit pages and JSON updates against a running Fizzy."""
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
                                 {"board":{"name":f"Comment edit parity {time.time_ns()}"}}, bearer=True)
        assert status == 201, (target["name"], "board", status)
        board = json.loads(raw)
        boards.append(board["id"])
        status, _, raw = request(target, f"/{account}/boards/{board['id']}/cards", "POST",
                                 {"card":{"title":"Edit my comment"}}, bearer=True)
        assert status == 201, (target["name"], "card", status)
        number = json.loads(raw)["number"]
        status, _, raw = request(target, f"/{account}/cards/{number}/comments", "POST",
                                 {"comment":{"body":"Original comment"}}, bearer=True)
        assert status == 201, (target["name"], "comment", status)
        comment_id = json.loads(raw)["id"]
        path = f"/{account}/cards/{number}/comments/{comment_id}"
        status, _, body = request(target, path + "/edit", accept="text/html")
        assert status == 200, (target["name"], "edit", status)
        for marker in (b"turbo-frame", b"Original comment", b"comment[body]", b"Save", b"Cancel", b"Delete", b"lexxy-editor"):
            assert marker in body, (target["name"], marker)
        status, _, _ = request(target, path + "/edit.json", bearer=True)
        assert status == 406, (target["name"], "JSON edit", status)
        status, _, raw = request(target, path, "PATCH",
                                 {"comment":{"body":"Updated comment"}}, bearer=True)
        assert status == 200 and "Updated comment" in json.loads(raw)["body"]["plain_text"], (target["name"], "update", status)
        status, _, raw = request(target, path, bearer=True)
        assert status == 200 and "Updated comment" in json.loads(raw)["body"]["plain_text"]
        status, _, body = request(target, f"/{account}/cards/{number}", accept="text/html")
        assert status == 200 and b"Updated comment" in body
        if target["name"] == "rizzy":
            assert (path + "/edit").encode() in body
            status, headers, _ = request(target, path, "POST",
                                         b"_method=patch&comment%5Bbody%5D=Saved+by+form", accept="text/html")
            assert status == 302 and headers["Location"] == f"/{account}/cards/{number}"
            status, _, raw = request(target, path, bearer=True)
            assert status == 200 and "Saved by form" in json.loads(raw)["body"]["plain_text"]
            status, headers, _ = request(target, path, "POST", b"_method=delete", accept="text/html")
            assert status == 302 and headers["Location"] == f"/{account}/cards/{number}"
            assert request(target, path, bearer=True)[0] == 404
        return {"edit":200,"json_edit":406,"json_update":200,"browser_card":200}
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
    print("Comment edit parity matched: edit page, JSON rejection, update, reload")


if __name__ == "__main__":
    main()
