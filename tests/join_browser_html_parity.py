#!/usr/bin/env python3
"""Compare the public invite form and magic-code redirects against Fizzy."""
import http.cookiejar
import json
import re
import sqlite3
import time
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request
from users_events_html_parity import target


def csrf_from(page):
    match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
    return match.group(1).decode() if match else None


def check(owner):
    login(owner)
    account = owner["account"]
    path = f"/{account}/account/join_code.json"
    status, _, raw = request(owner, path, bearer=True)
    assert status == 200
    invite = json.loads(raw)
    join = f"/{account}/join/{invite['code']}"
    status, _, owner_page = request(owner, join, accept="text/html")
    assert status == 200
    owner_form = urllib.parse.urlencode({"email_address": owner["email"]}).encode()
    status, headers, _ = request(owner, join, "POST", owner_form, accept="text/html", csrf=csrf_from(owner_page))
    assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == f"/{account}/landing", (owner["name"], status, headers.get("Location"))
    visitor = dict(owner)
    visitor["opener"] = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
    email = f"invite-{time.time_ns()}@example.com"
    member_id = None
    try:
        status, _, page = request(visitor, join, accept="text/html")
        assert status == 200 and b"Join " in page and b'name="email_address"' in page
        csrf = csrf_from(page)
        status, _, _ = request(visitor, f"/{account}/join/INVALID-CODE", accept="text/html")
        assert status == 404
        invalid = urllib.parse.urlencode({"email_address": "not-a-valid-email"}).encode()
        status, _, _ = request(visitor, f"/{account}/join/INVALID-CODE", "POST", invalid, accept="text/html", csrf=csrf)
        assert status == 404
        status, _, _ = request(visitor, join, "POST", invalid, accept="text/html", csrf=csrf)
        assert status == 422
        valid = urllib.parse.urlencode({"email_address": email}).encode()
        status, headers, _ = request(visitor, join, "POST", valid, accept="text/html", csrf=csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == "/session/magic_link", (owner["name"], status, headers.get("Location"))
        status, _, magic_page = request(visitor, "/session/magic_link", accept="text/html")
        assert status == 200 and b"Check your email" in magic_page
        with sqlite3.connect(owner["db"]) as db:
            code = db.execute("SELECT m.code FROM magic_links m JOIN identities i ON i.id=m.identity_id WHERE i.email_address=? ORDER BY m.created_at DESC LIMIT 1", (email,)).fetchone()[0]
        form = urllib.parse.urlencode({"code": code}).encode()
        status, headers, _ = request(visitor, "/session/magic_link", "POST", form, accept="text/html", csrf=csrf_from(magic_page))
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == f"/{account}/users/verifications/new", (owner["name"], status, headers.get("Location"))
        status, _, page = request(visitor, f"/{account}/users/verifications/new", accept="text/html")
        assert status == 200
        status, headers, _ = request(visitor, f"/{account}/users/verifications", "POST", b"", accept="text/html", csrf=csrf_from(page))
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == f"/{account}/users/joins/new", (owner["name"], status)
        status, _, page = request(visitor, f"/{account}/users/joins/new", accept="text/html")
        assert status == 200 and b"Great! Now enter your name" in page
        form = urllib.parse.urlencode({"user[name]": "Joined Parity Person"}).encode()
        status, headers, _ = request(visitor, f"/{account}/users/joins", "POST", form, accept="text/html", csrf=csrf_from(page))
        assert status == 302
        landing = urllib.parse.urlparse(headers["Location"]).path
        assert landing == f"/{account}/landing", (owner["name"], landing)
        status, _, raw = request(owner, f"/{account}/users.json", bearer=True)
        assert status == 200
        member = next(user for user in json.loads(raw) if user["email_address"] == email)
        member_id = member["id"]
        assert member["name"] == "Joined Parity Person" and member["role"] == "member"
        return {"page": True, "existing_member_redirect": True, "invalid_code": True, "invalid_code_post": True, "invalid_email": True,
                "join_redirect": True, "magic_redirect": True, "verify": True,
                "name": True, "landing": True}
    finally:
        if member_id:
            request(owner, f"/{account}/users/{member_id}.json", "DELETE", bearer=True)


def main():
    source, rust = check(target("fizzy")), check(target("rizzy"))
    assert source == rust, (source, rust)
    print("Public invite browser flow matched: form, code, verification, name and landing")


if __name__ == "__main__":
    main()
