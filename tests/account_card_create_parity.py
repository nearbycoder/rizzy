#!/usr/bin/env python3
"""Compare Fizzy's account-level card creation route with Rizzy."""
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
        status, _, raw = request(target, f"/{account}/boards", "POST",
                                 {"board":{"name":f"Account card parity {time.time_ns()}"}}, bearer=True)
        assert status == 201, (target["name"], "board", status)
        board_id = json.loads(raw)["id"]
        boards.append(board_id)
        status, _, raw = request(target, f"/{account}/cards", "POST",
                                 {"board_id":board_id,"card":{"title":"Account card route"}}, bearer=True)
        assert status == 201, (target["name"], "JSON create", status, raw[:120])
        card = json.loads(raw)
        assert card["title"] == "Account card route" and card["board"]["id"] == board_id
        status, _, html = request(target, f"/{account}/boards/{board_id}", accept="text/html")
        assert status == 200
        token = re.search(rb'<meta name="csrf-token" content="([^"]+)"', html)
        csrf = token.group(1).decode() if token else None
        status, headers, _ = request(target, f"/{account}/cards", "POST",
                                     urllib.parse.urlencode({"board_id":board_id}).encode(),
                                     accept="text/html", csrf=csrf)
        assert status == 302, (target["name"], "browser create", status)
        redirect = urllib.parse.urlparse(headers["Location"]).path
        assert re.fullmatch(rf"/{account}/cards/\d+/draft", redirect), (target["name"], redirect)
        assert request(target, redirect, accept="text/html")[0] == 200
        return {"json":201,"browser":302,"draft":200}
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
    print("Account card creation parity matched: JSON create and browser draft redirect")


if __name__ == "__main__":
    main()
