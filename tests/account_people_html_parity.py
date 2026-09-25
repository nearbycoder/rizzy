#!/usr/bin/env python3
"""Compare account people role and removal actions with Fizzy."""
import http.cookiejar
import json
import re
import sqlite3
import time
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request
from users_events_html_parity import target


def csrf(page):
    match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
    return match.group(1).decode() if match else None


def check(owner):
    login(owner)
    account = owner["account"]
    status, _, raw = request(owner, f"/{account}/account/join_code.json", bearer=True)
    assert status == 200
    invite = json.loads(raw)
    if invite["usage_limit"] - invite["usage_count"] < 2:
        status, _, _ = request(owner, f"/{account}/account/join_code.json", "DELETE", bearer=True)
        assert status == 204
        status, _, raw = request(owner, f"/{account}/account/join_code.json", bearer=True)
        assert status == 200
        invite = json.loads(raw)
    join = f"/{account}/join/{invite['code']}"
    email = f"people-{time.time_ns()}@example.com"
    visitor = dict(owner)
    visitor["opener"] = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
    member_id = None
    removed = False
    rejoined_id = None
    try:
        status, _, join_page = request(visitor, join, accept="text/html")
        assert status == 200
        form = urllib.parse.urlencode({"email_address": email}).encode()
        status, _, _ = request(visitor, join, "POST", form, accept="text/html", csrf=csrf(join_page))
        assert status == 302
        status, _, magic_page = request(visitor, "/session/magic_link", accept="text/html")
        assert status == 200
        with sqlite3.connect(owner["db"]) as db:
            code = db.execute("SELECT m.code FROM magic_links m JOIN identities i ON i.id=m.identity_id WHERE i.email_address=? ORDER BY m.created_at DESC LIMIT 1", (email,)).fetchone()[0]
        form = urllib.parse.urlencode({"code": code}).encode()
        status, _, _ = request(visitor, "/session/magic_link", "POST", form, accept="text/html", csrf=csrf(magic_page))
        assert status == 302

        status, _, raw = request(owner, f"/{account}/users.json", bearer=True)
        member = next(user for user in json.loads(raw) if user["email_address"] == email)
        member_id = member["id"]
        settings = f"/{account}/account/settings"
        status, _, page = request(owner, settings, accept="text/html")
        assert status == 200 and email.encode() in page
        owner_csrf = csrf(page)
        role = f"/{account}/users/{member_id}/role"
        promote = urllib.parse.urlencode({"_method": "patch", "user[role]": "admin"}).encode()
        status, headers, _ = request(owner, role, "POST", promote, accept="text/html", csrf=owner_csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == settings, (owner["name"], status)
        status, _, raw = request(owner, f"/{account}/users/{member_id}.json", bearer=True)
        assert json.loads(raw)["role"] == "admin"

        self_role = f"/{account}/users/{owner['user_id']}/role"
        status, _, _ = request(visitor, self_role + ".json", "PATCH", {"user": {"role": "member"}}, accept="application/json")
        assert status == 403, (owner["name"], status)
        status, _, _ = request(owner, self_role + ".json", "PATCH", {"user": {"role": "member"}}, bearer=True)
        assert status == 403
        demote = urllib.parse.urlencode({"_method": "patch", "user[role]": "member"}).encode()
        status, headers, _ = request(owner, role, "POST", demote, accept="text/html", csrf=owner_csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == settings
        status, _, raw = request(owner, f"/{account}/users/{member_id}.json", bearer=True)
        assert json.loads(raw)["role"] == "member"
        status, _, _ = request(visitor, role + ".json", "PATCH", {"user": {"role": "admin"}})
        assert status == 403
        status, _, _ = request(visitor, f"/{account}/users/{owner['user_id']}.json", "DELETE")
        assert status == 403

        user_path = f"/{account}/users/{member_id}"
        status, headers, _ = request(owner, user_path, "DELETE", accept="text/html", csrf=owner_csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == settings, (owner["name"], status)
        removed = True
        status, _, _ = request(owner, user_path + ".json", bearer=True)
        assert status == 404
        with sqlite3.connect(owner["db"]) as db:
            if owner["name"] == "fizzy":
                row = db.execute("SELECT active,identity_id FROM users WHERE id=?", (int(member_id, 36).to_bytes(16, "big"),)).fetchone()
            else:
                row = db.execute("SELECT active,identity_id FROM users WHERE id=?", (member_id,)).fetchone()
        assert row == (0, None), (owner["name"], row)
        status, headers, _ = request(visitor, join, "POST", urllib.parse.urlencode({"email_address": email}).encode(), accept="text/html", csrf=csrf(join_page))
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == f"/{account}/users/verifications/new", (owner["name"], status, headers.get("Location"))
        status, _, raw = request(owner, f"/{account}/users.json", bearer=True)
        rejoined = next(user for user in json.loads(raw) if user["email_address"] == email)
        rejoined_id = rejoined["id"]
        assert rejoined_id != member_id and rejoined["role"] == "member"
        return {"listing": True, "promote": True, "permission": True,
                "demote": True, "remove": True, "identity_detached": True, "rejoin_new_user": True}
    finally:
        if rejoined_id:
            request(owner, f"/{account}/users/{rejoined_id}.json", "DELETE", bearer=True)
        if member_id and not removed:
            request(owner, f"/{account}/users/{member_id}.json", "DELETE", bearer=True)


def main():
    source, rust = check(target("fizzy")), check(target("rizzy"))
    assert source == rust
    print("Account people matched: listing, roles, permissions, removal and rejoining with a new user ID")


if __name__ == "__main__":
    main()
