#!/usr/bin/env python3
"""Compare the browser assignment picker and self-assignment with Fizzy."""
import http.cookiejar
import json
import os
import re
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
    status, _, raw = request(target, f"/{account}/users", bearer=True)
    assert status == 200
    owner_id = next(user["id"] for user in json.loads(raw) if user["role"] == "owner")
    boards = []
    try:
        status, _, raw = request(target, f"/{account}/boards", "POST",
                                 {"board":{"name":f"Assignment parity {time.time_ns()}"}}, bearer=True)
        assert status == 201, (target["name"], "board", status)
        board_id = json.loads(raw)["id"]
        boards.append(board_id)
        status, _, raw = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                 {"card":{"title":"Assignment picker"}}, bearer=True)
        assert status == 201, (target["name"], "card", status)
        number = json.loads(raw)["number"]
        path = f"/{account}/cards/{number}/assignments"
        status, headers, body = request(target, path + "/new", accept="text/html")
        assert status == 200 and headers.get("ETag"), (target["name"], "picker", status)
        for marker in (b"turbo-frame", b"Assign this to", b"Filter", b"Me", b"Maximum 100 assignees"):
            assert marker in body, (target["name"], marker)
        assert b'aria-checked="false"' in body
        status, _, _ = request({**target,"extra_headers":{"If-None-Match":headers["ETag"]}},
                               path + "/new", accept="text/html")
        assert status == 304, (target["name"], "cache", status)
        assert request(target, path + "/new.json", bearer=True)[0] == 406
        status, _, raw = request(target, path, "POST", {"assignee_id":owner_id}, bearer=True)
        assert status == 204, (target["name"], "assign", status, raw[:160])
        status, _, body = request(target, path + "/new", accept="text/html")
        assert status == 200 and re.search(rb'aria-checked="true"', body)
        if target["name"] == "rizzy":
            status, headers, _ = request(target, path + f"?assignee_id={owner_id}", "POST",
                                         b"", accept="text/html")
            assert status == 302 and headers["Location"] == f"/{account}/cards/{number}"
        else:
            status, _, _ = request(target, path, "POST", {"assignee_id":owner_id}, bearer=True)
            assert status == 204
        status, _, raw = request(target, f"/{account}/cards/{number}", bearer=True)
        assert status == 200 and not json.loads(raw)["assignees"], (target["name"], "unassign", status)
        return {"picker":200,"cache":304,"json_picker":406,"toggle":204}
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
    print("Assignment picker parity matched: choices, cache, JSON toggle, selected state")


if __name__ == "__main__":
    main()
