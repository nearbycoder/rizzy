#!/usr/bin/env python3
"""Compare card and comment reaction forms with Fizzy."""
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
        card_id = json.loads(raw)["id"]
        status, _, _ = request(target, f"/{account}/cards/{number}/publish", "POST", b"", bearer=True)
        assert status == 201, (target["name"], "publish", status)
        status, _, raw = request(target, f"/{account}/cards/{number}/comments", "POST",
                                 {"comment":{"body":"React to this"}}, bearer=True)
        assert status == 201, (target["name"], "comment", status)
        comment_id = json.loads(raw)["id"]
        status, _, card_html = request(target, f"/{account}/cards/{number}", accept="text/html")
        assert status == 200 and f"reacting_card_{card_id}".encode() in card_html
        assert f"reacting_comment_{comment_id}".encode() in card_html
        csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', card_html)
        csrf = csrf_match.group(1).decode() if csrf_match else None
        paths = [f"/{account}/cards/{number}/reactions",
                 f"/{account}/cards/{number}/comments/{comment_id}/reactions"]
        for path, kind, target_id in zip(paths, ("card", "comment"), (card_id, comment_id)):
            status, _, html = request(target, path, accept="text/html")
            assert status == 200 and f"reacting_{kind}_{target_id}".encode() in html
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
            status, headers, stream = request(target, path, "POST",
                                               b"reaction%5Bcontent%5D=Great+work%21",
                                               accept="text/vnd.turbo-stream.html", csrf=csrf)
            assert status == 200 and headers.get_content_type() == "text/vnd.turbo-stream.html", (target["name"], path, status, stream[:180])
            assert b'action="replace"' in stream and f"reacting_{kind}_{target_id}".encode() in stream
            assert b"Great work!" in stream
            status, _, raw = request(target, path, bearer=True)
            reaction_id = next(item["id"] for item in json.loads(raw) if item["content"] == "Great work!")
            status, headers, stream = request(target, path + "/" + reaction_id, "DELETE",
                                               accept="text/vnd.turbo-stream.html", csrf=csrf)
            assert status == 200 and headers.get_content_type() == "text/vnd.turbo-stream.html"
            assert b'action="remove"' in stream and f"reaction_{reaction_id}".encode() in stream
            assert all(item["id"] != reaction_id for item in json.loads(request(target, path, bearer=True)[2]))
            status, _, _ = request(target, path, "POST", b"reaction%5Bcontent%5D=Plain+HTML",
                                    accept="text/html", csrf=csrf)
            assert status == 406, (target["name"], path, "HTML create", status)
            status, _, raw = request(target, path, bearer=True)
            html_reaction = next(item["id"] for item in json.loads(raw) if item["content"] == "Plain HTML")
            assert request(target, path + "/" + html_reaction, "DELETE", bearer=True)[0] == 204
        missing_path = f"/{account}/cards/{number}/comments/nonexistent/reactions"
        assert request(target, missing_path, accept="text/html")[0] == 404
        assert request(target, missing_path, bearer=True)[0] == 404
        if target["name"] == "rizzy":
            status, _, body = request(target, f"/{account}/cards/{number}", accept="text/html")
            assert status == 200 and all((path + "/new").encode() in body for path in paths)
        return {"card_form":200,"comment_form":200,"create":201,"html_index":200,"turbo_create":200,"turbo_destroy":200}
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
    print("Reaction parity matched: card and comment HTML lists, forms, JSON and Turbo mutations, missing comments, and HTML create side effects")


if __name__ == "__main__":
    main()
