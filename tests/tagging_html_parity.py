#!/usr/bin/env python3
"""Compare the card tag picker and toggle with Fizzy."""
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
    title = f"tagparity{time.time_ns()}"
    try:
        status, _, raw = request(target, f"/{account}/boards", "POST",
                                 {"board":{"name":f"Tagging parity {time.time_ns()}"}}, bearer=True)
        assert status == 201, (target["name"], "board", status)
        board_id = json.loads(raw)["id"]
        boards.append(board_id)
        status, _, raw = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                 {"card":{"title":"Tag picker"}}, bearer=True)
        assert status == 201, (target["name"], "card", status)
        number = json.loads(raw)["number"]
        card_id = json.loads(raw)["id"]
        status, _, _ = request(target, f"/{account}/cards/{number}/publish", "POST", b"", bearer=True)
        assert status == 201, (target["name"], "publish", status)
        path = f"/{account}/cards/{number}/taggings"
        status, _, card_html = request(target, f"/{account}/cards/{number}", accept="text/html")
        assert status == 200 and f"tags_card_{card_id}".encode() in card_html
        csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', card_html)
        csrf = csrf_match.group(1).decode() if csrf_match else None
        status, headers, body = request(target, path + "/new", accept="text/html")
        assert status == 200 and headers.get("ETag"), (target["name"], "picker", status)
        for marker in (b"turbo-frame", b"Tag this", b"tag_title", b"Create tag"):
            assert marker in body, (target["name"], marker)
        status, _, _ = request({**target,"extra_headers":{"If-None-Match":headers["ETag"]}},
                               path + "/new", accept="text/html")
        assert status == 304, (target["name"], "cache", status)
        assert request(target, path + "/new.json", bearer=True)[0] == 406
        assert request(target, path + "/new", accept="text/vnd.turbo-stream.html")[0] == 406
        status, _, _ = request(target, path, "POST", {"tag_title":title}, bearer=True)
        assert status == 204, (target["name"], "add tag", status)
        status, _, body = request(target, path + "/new", accept="text/html")
        assert status == 200 and f"#{title}".encode() in body and b'aria-checked="true"' in body
        status, headers, stream = request(target, path + "?" + urllib.parse.urlencode({"tag_title":title}),
                                          "POST", b"", accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200 and headers.get_content_type() == "text/vnd.turbo-stream.html", (target["name"], status, stream[:180])
        assert b'action="replace"' in stream and f"tags_card_{card_id}".encode() in stream
        assert f"tagging_card_{card_id}".encode() in stream
        status, _, raw = request(target, f"/{account}/cards/{number}", bearer=True)
        assert status == 200 and not json.loads(raw)["tags"], (target["name"], "remove tag", status)
        status, headers, stream = request(target, path, "POST", b"tag_title=%23" + title.encode(),
                                          accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200 and f"tags_card_{card_id}".encode() in stream
        status, _, raw = request(target, f"/{account}/cards/{number}", bearer=True)
        assert status == 200 and title in json.loads(raw)["tags"]
        return {"picker":200,"cache":304,"json_picker":406,"toggle":204,"turbo_toggle":200}
    finally:
        for board_id in boards:
            status, _, _ = request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)
            assert status == 204, (target["name"], "cleanup", status)
        with sqlite3.connect(target["db"]) as db:
            db.execute("DELETE FROM tags WHERE title=? AND NOT EXISTS "
                       "(SELECT 1 FROM taggings WHERE taggings.tag_id=tags.id)", (title,))


def main():
    with sqlite3.connect(os.environ.get("FIZZY_DB", "upstream/storage/production.sqlite3")) as db:
        email = db.execute("SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
                           "JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? "
                           "AND u.role='owner' LIMIT 1", (int(os.environ["FIZZY_ACCOUNT"]),)).fetchone()[0]
    os.environ["FIZZY_EMAIL"] = email
    source, rust = check(target("FIZZY")), check(target("RIZZY"))
    assert source == rust
    print("Tag picker parity matched: frame, cache, JSON and Turbo toggles, in-place tag block")


if __name__ == "__main__":
    main()
