#!/usr/bin/env python3
"""Compare Fizzy's browser webhook management flow with Rizzy."""
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
    return {"name": name.lower(), "url": os.environ[f"{name}_URL"],
            "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
            "email": os.environ[f"{name}_EMAIL"], "db": os.environ[f"{name}_DB"],
            "opener": urllib.request.build_opener(
                urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}


def form(**fields):
    return urllib.parse.urlencode(fields, doseq=True).encode()


def csrf_token(body):
    match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', body)
    return match.group(1).decode() if match else None


def assert_shell(target, body):
    for marker in (b'id="global-container"', b'id="header"', b'id="main"',
                   b'id="footer"', b'href="#main"'):
        assert marker in body, (target["name"], "webhook page shell", marker)


def check(target):
    login(target)
    account = target["account"]
    title = f"Webhook browser parity {time.time_ns()}"
    status, _, raw = request(target, f"/{account}/boards", "POST", {"board":{"name":title}}, bearer=True)
    assert status == 201, (target["name"], "board", status)
    board_id = json.loads(raw)["id"]
    base = f"/{account}/boards/{board_id}/webhooks"
    try:
        status, _, board_page = request(target, f"/{account}/boards/{board_id}", accept="text/html")
        assert status == 200 and base.encode() in board_page, (target["name"], "board webhook link", status)
        status, _, page = request(target, base, accept="text/html")
        assert status == 200 and b"Set up a new webhook" in page, (target["name"], "index", status)
        assert_shell(target, page)
        status, _, page = request(target, base + "/new", accept="text/html")
        assert status == 200 and b"Payload URL" in page and b"Enable all" in page, (target["name"], "new form", status)
        assert_shell(target, page)
        assert b"Back to Webhooks" in page and f'href="{base}"'.encode() in page, (target["name"], "new form navigation")
        csrf = csrf_token(page)

        status, _, page = request(target, base, "POST", form(**{"webhook[name]":"", "webhook[url]":"invalid-url"}), accept="text/html", csrf=csrf)
        assert status == 422 and b"form" in page, (target["name"], "invalid create", status)
        status, _, page = request(target, base, "POST", form(**{"webhook[name]":"Bad target", "webhook[url]":"invalid-url"}), accept="text/html", csrf=csrf)
        assert status == 422 and b'Bad target' in page and b'invalid-url' in page, (target["name"], "invalid values", status)
        name = "Parity webhook"
        payload_url = "https://example.com/webhook"
        data = form(**{"webhook[name]":name, "webhook[url]":payload_url,
                       "webhook[subscribed_actions][]":["", "card_published", "card_closed"]})
        status, headers, _ = request(target, base, "POST", data, accept="text/html", csrf=csrf)
        assert status == 302, (target["name"], "create", status)
        detail = urllib.parse.urlparse(headers["Location"]).path
        assert detail.startswith(base + "/") and "/edit" not in detail, (target["name"], detail)

        status, _, page = request(target, base, accept="text/html")
        assert status == 200 and name.encode() in page and detail.encode() in page, (target["name"], "populated index", status)
        status, _, page = request(target, detail, accept="text/html")
        assert status == 200 and name.encode() in page and payload_url.encode() in page, (target["name"], "show", status)
        assert_shell(target, page)
        assert b"Secret" in page and b"Subscribed to" in page and b"Card added" in page and b"Card moved to" in page

        status, _, page = request(target, detail + "/edit", accept="text/html")
        assert status == 200 and b"Save Changes" in page and name.encode() in page, (target["name"], "edit form", status)
        assert_shell(target, page)
        assert f"<title>{name} | ".encode() in page and b"Name your Webhook" in page, (target["name"], "edit title and placeholder")
        assert b'name="webhook[url]"' not in page, (target["name"], "url should not be editable")
        csrf = csrf_token(page)
        status, _, page = request(target, detail, "POST", form(**{"_method":"put", "webhook[name]":""}), accept="text/html", csrf=csrf)
        assert status == 422 and b"form" in page, (target["name"], "invalid update", status)
        status, headers, _ = request(target, detail, "POST", form(**{"_method":"put", "webhook[name]":"Renamed webhook", "webhook[subscribed_actions][]":["", "comment_created"]}), accept="text/html", csrf=csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == detail, (target["name"], "update", status)
        status, _, page = request(target, detail, accept="text/html")
        assert status == 200 and b"Renamed webhook" in page and b"Comment added" in page and payload_url.encode() in page, (target["name"], "updated show", status)
        status, headers, _ = request(target, detail, "POST", form(**{"_method":"delete"}), accept="text/html", csrf=csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == base, (target["name"], "delete", status)
        status, _, _ = request(target, detail, accept="text/html")
        assert status == 404, (target["name"], "deleted", status)
        status, _, raw = request(target, base, bearer=True)
        assert status == 200 and json.loads(raw) == [], (target["name"], "JSON collection", status)
        return {"index":200,"new":200,"invalid":422,"create":302,"show":200,"edit":200,"update":302,"delete":302}
    finally:
        status, _, _ = request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)
        assert status == 204, (target["name"], "cleanup", status)


def main():
    source_db = os.environ.get("FIZZY_DB", "upstream/storage/production.sqlite3")
    with sqlite3.connect(source_db) as db:
        email = db.execute("SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1", (int(os.environ["FIZZY_ACCOUNT"]),)).fetchone()[0]
    os.environ["FIZZY_EMAIL"] = email
    source, rust = check(target("FIZZY")), check(target("RIZZY"))
    assert source == rust
    print("Webhook browser parity matched: forms, validation, CRUD, rendered detail, and JSON collection")


if __name__ == "__main__":
    main()
