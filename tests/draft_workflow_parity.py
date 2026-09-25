#!/usr/bin/env python3
"""Exercise Fizzy's browser draft flow against a Rust server with dev magic links."""
import http.cookiejar
import json
import os
import re
import sqlite3
import time
import urllib.error
import urllib.parse
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


def request(target, path, method="GET", data=None, *, accept="application/json", bearer=False, csrf=None):
    headers = {"Accept": accept}
    if bearer:
        headers["Authorization"] = f"Bearer {target['token']}"
    if csrf:
        headers["X-CSRF-Token"] = csrf
    if isinstance(data, dict):
        headers["Content-Type"] = "application/json"
        data = json.dumps(data).encode()
    elif data is not None:
        headers["Content-Type"] = "application/x-www-form-urlencoded"
    raw_request = urllib.request.Request(target["url"] + path, method=method, data=data, headers=headers)
    try:
        response = target["opener"].open(raw_request, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


def login(target):
    status, headers, _ = request(target, "/session", "POST", {"email_address": target["email"]})
    assert status == 201, status
    if target["name"] == "fizzy":
        with sqlite3.connect(target["db"]) as db:
            code = db.execute(
                "SELECT m.code FROM magic_links m JOIN identities i ON i.id=m.identity_id "
                "WHERE i.email_address=? ORDER BY m.created_at DESC LIMIT 1", (target["email"],)
            ).fetchone()[0]
    else:
        code = headers["X-Magic-Link-Code"]
    status, _, _ = request(target, "/session/magic_link", "POST", {"code": code})
    assert status == 200, status


def run(target):
    login(target)
    account = target["account"]
    status, _, raw = request(target, f"/{account}/boards", "POST", {"board": {"name": f"Draft parity {time.time_ns()}"}}, bearer=True)
    assert status == 201, (status, raw[:100])
    board = json.loads(raw)
    board_path = f"/{account}/boards/{board['id']}"
    try:
        status, _, html = request(target, board_path, accept="text/html")
        assert status == 200, status
        token = re.search(rb'<meta name="csrf-token" content="([^"]+)"', html)
        csrf = token.group(1).decode() if token else None

        def new_draft():
            status, headers, body = request(target, board_path + "/cards", "POST", b"", accept="text/html", csrf=csrf)
            assert status == 302 and not body, (status, body[:100])
            path = urllib.parse.urlparse(headers["Location"]).path
            match = re.fullmatch(rf"/{account}/cards/(\d+)/draft", path)
            assert match, path
            return match.group(1)

        number = new_draft()
        status, _, html = request(target, f"/{account}/cards/{number}/draft", accept="text/html")
        assert status == 200 and b"card-perma--draft" in html, (status, html[:100])
        status, _, _ = request(target, f"/{account}/cards/{number}/draft.json", bearer=True)
        assert status == 406, status
        status, headers, _ = request(target, f"/{account}/cards/{number}.json", bearer=True)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path.endswith(f"/cards/{number}/draft")
        assert new_draft() == number, "creating a second card did not resume the draft"

        title = f"draftparity{time.time_ns()}"
        status, _, raw = request(target, f"/{account}/cards/{number}.json", "PATCH", {
            "card": {"title": title, "description": "Draft parity body", "status": "published"}
        }, bearer=True)
        card = json.loads(raw)
        assert status == 200 and card["status"] == "drafted" and card["title"] == title, (status, card)
        status, _, raw = request(target, f"/{account}/search.json?q={title}", bearer=True)
        assert status == 200 and all(item["number"] != int(number) for item in json.loads(raw)), raw[:200]
        status, _, raw = request(target, f"/{account}/activities.json?board_ids%5B%5D={board['id']}", bearer=True)
        assert status == 200 and not json.loads(raw), raw[:200]

        status, _, body = request(target, f"/{account}/cards/{number}/publish.json", "POST", b"", bearer=True)
        assert status == 201 and body == b"", (status, body)
        status, _, raw = request(target, f"/{account}/cards/{number}.json", bearer=True)
        card = json.loads(raw)
        assert status == 200 and card["status"] == "published" and card["title"] == title, (status, card)
        status, _, raw = request(target, f"/{account}/search.json?q={title}", bearer=True)
        assert status == 200 and any(item["number"] == int(number) for item in json.loads(raw)), raw[:200]
        status, _, raw = request(target, f"/{account}/activities.json?board_ids%5B%5D={board['id']}", bearer=True)
        activities = json.loads(raw)
        assert status == 200 and len(activities) == 1 and activities[0]["action"] == "card_published", (status, activities)
        status, headers, _ = request(target, f"/{account}/cards/{number}/draft", accept="text/html")
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path.endswith(f"/cards/{number}")
        status, _, raw = request(target, f"/{account}/cards/{number}.json", "PATCH", {"card": {"title": "", "status": "drafted"}}, bearer=True)
        updated = json.loads(raw)
        assert status == 200 and updated["status"] == "published" and updated["title"] == "Untitled", (status, updated)

        another = new_draft()
        status, headers, _ = request(target, f"/{account}/cards/{another}/publish", "POST", b"creation_type=add_another", accept="text/html", csrf=csrf)
        assert status == 302, status
        new_path = urllib.parse.urlparse(headers["Location"]).path
        next_number = re.fullmatch(rf"/{account}/cards/(\d+)/draft", new_path)
        assert next_number and next_number.group(1) != another, new_path
        status, _, raw = request(target, f"/{account}/cards/{another}.json", bearer=True)
        assert status == 200 and json.loads(raw)["title"] == "Untitled", raw[:150]
        return {"draft_reused": True, "draft_json_status": 406, "publish_status": 201,
                "add_another": True, "published_title_matches": card["title"] == title, "search_after_publish": True,
                "activity_after_publish": True, "empty_title_default": True, "published_status_immutable": True}
    finally:
        status, _, _ = request(target, board_path, "DELETE", bearer=True)
        assert status == 204, status


def main():
    fizzy_db = os.environ.get("FIZZY_DB_PATH", "upstream/storage/production.sqlite3")
    with sqlite3.connect(fizzy_db) as db:
        email = db.execute(
            "SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
            (int(os.environ["FIZZY_ACCOUNT"]),)
        ).fetchone()[0]
    targets = []
    for name in ("FIZZY", "RIZZY"):
        target = {"name": name.lower(), "url": os.environ[f"{name}_URL"],
                  "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
                  "email": email if name == "FIZZY" else os.environ.get("RIZZY_EMAIL", "owner@example.com"),
                  "db": fizzy_db}
        target["opener"] = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
        targets.append(target)
    source, rust = (run(target) for target in targets)
    assert source == rust, (source, rust)
    print("Draft workflow matched: create, resume, edit, publish, and add another")


if __name__ == "__main__":
    main()
