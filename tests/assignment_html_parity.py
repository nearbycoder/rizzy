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
        card_id = json.loads(raw)["id"]
        status, _, _ = request(target, f"/{account}/cards/{number}/publish", "POST", b"", bearer=True)
        assert status == 201, (target["name"], "publish", status)
        path = f"/{account}/cards/{number}/assignments"
        status, _, card_html = request(target, f"/{account}/cards/{number}", accept="text/html")
        assert status == 200 and f"meta_card_{card_id}".encode() in card_html, (target["name"], status, card_id, re.findall(rb'id="[^"]*meta[^"]*"', card_html)[:12])
        for marker in (b"card__meta-avatars--author", b"card__meta-avatars--assignees",
                       b"card__meta-text--added", b"card__meta-text--updated",
                       b"card__meta-text--assignees"):
            assert marker in card_html, (target["name"], marker)
        if target["name"] == "rizzy":
            assert b'id="assignment-dialog"' in card_html
            assert f'src="/{account}/users/{owner_id}/avatar"'.encode() in card_html
        csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', card_html)
        csrf = csrf_match.group(1).decode() if csrf_match else None
        status, headers, body = request(target, path + "/new", accept="text/html")
        assert status == 200 and headers.get("ETag"), (target["name"], "picker", status)
        for marker in (b"turbo-frame", b"Assign this to", b"Filter", b"Me", b"Maximum 100 assignees"):
            assert marker in body, (target["name"], marker)
        assert re.search(rb'<div class="popup__footer" hidden data-assignment-limit-target="limitMessage">', body), (target["name"], "premature assignment limit warning")
        assert b'aria-checked="false"' in body
        status, _, _ = request({**target,"extra_headers":{"If-None-Match":headers["ETag"]}},
                               path + "/new", accept="text/html")
        assert status == 304, (target["name"], "cache", status)
        assert request(target, path + "/new.json", bearer=True)[0] == 406
        assert request(target, path + "/new", accept="text/vnd.turbo-stream.html")[0] == 406
        status, _, raw = request(target, path, "POST", {"assignee_id":owner_id}, bearer=True)
        assert status == 204, (target["name"], "assign", status, raw[:160])
        status, _, body = request(target, path + "/new", accept="text/html")
        assert status == 200 and re.search(rb'aria-checked="true"', body)
        def turbo_toggle(toggle_path):
            status, headers, stream = request(target, toggle_path, "POST", b"",
                                              accept="text/vnd.turbo-stream.html", csrf=csrf)
            assert status == 200 and headers.get_content_type() == "text/vnd.turbo-stream.html", (target["name"], status, stream[:200])
            assert b'action="replace"' in stream and f'meta_card_{card_id}'.encode() in stream
            assert f'assignment_card_{card_id}'.encode() in stream
            assert b"card__meta-avatars--assignees" in stream
            return stream
        stream = turbo_toggle(path + f"?assignee_id={owner_id}")
        assert b"Not assigned" in stream
        status, _, raw = request(target, f"/{account}/cards/{number}", bearer=True)
        assert status == 200 and not json.loads(raw)["assignees"], (target["name"], "unassign", status)
        stream = turbo_toggle(f"/{account}/cards/{number}/self_assignment")
        assert b"Assigned to" in stream
        status, _, raw = request(target, f"/{account}/cards/{number}", bearer=True)
        assert status == 200 and any(user["id"] == owner_id for user in json.loads(raw)["assignees"])
        status, _, body = request(target, path + "/new", accept="text/html")
        assert status == 200 and re.search(rb'aria-checked="true"', body)
        status, _, _ = request(target, f"/{account}/cards/{number}/self_assignment", "POST", b"", bearer=True)
        assert status == 204
        status, html_headers, html_body = request(target, path + f"?assignee_id={owner_id}",
                                                  "POST", b"", accept="text/html", csrf=csrf)
        assert status == 406, (target["name"], "plain HTML assignment", status)
        status, _, raw = request(target, f"/{account}/cards/{number}", bearer=True)
        assert status == 200 and [user["id"] for user in json.loads(raw)["assignees"]] == [owner_id], (target["name"], "plain HTML assignment side effect", status, raw[:200])
        status, _, _ = request(target, f"/{account}/cards/{number}/closure", "POST", b"", bearer=True)
        assert status == 204
        status, _, closed_html = request(target, f"/{account}/cards/{number}", accept="text/html")
        assert status == 200
        assert re.search(rb'id="assignees_card_' + card_id.encode() + rb'".{0,400}? hidden>', closed_html, re.S), target["name"]
        return {"picker":200,"cache":304,"json_picker":406,"toggle":204,"turbo_toggle":200,"self_assignment":200,"html_assignment":(406,html_headers.get_content_type(),html_body),"closed_picker_hidden":True}
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
    print("Assignment picker parity matched: choices, cache, JSON and Turbo toggles, in-place metadata, self-assignment")


if __name__ == "__main__":
    main()
