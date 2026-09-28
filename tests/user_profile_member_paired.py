#!/usr/bin/env python3
"""Compare another member's unverified, verified, and removed profile states."""

import datetime
import http.cookiejar
import json
import os
import re
import sqlite3
import sys
import tempfile
import urllib.parse
import urllib.request
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from card_image_parity import call  # noqa: E402
from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from html_form_inventory import normalized  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


class Profile(HTMLParser):
    def __init__(self):
        super().__init__()
        self.panels = 0
        self.frame_present = False
        self.frame_src = None

    def handle_starttag(self, tag, attributes):
        attrs = dict(attributes)
        if "settings__panel" in attrs.get("class", "").split():
            self.panels += 1
        if tag == "turbo-frame" and attrs.get("id") == "user_events":
            self.frame_present = True
            self.frame_src = attrs.get("src")


def summary(body, account):
    html = body.decode()
    parsed = Profile()
    parsed.feed(html)
    return {
        "panels": parsed.panels,
        "has_frame": parsed.frame_present,
        "frame_src": normalized(parsed.frame_src, account) if parsed.frame_src else None,
        "unverified": "Unverified" in html,
        "verification_message": "A sign-in code has been sent" in html,
        "email_link": 'href="mailto:member@example.com"' in html,
        "assigned_link": "Which cards are assigned to Ada?" in html,
        "added_link": "Which cards were added by Ada?" in html,
        "owner_controls": any(item in html for item in ("Edit profile", "Manage passkeys",
                                                      "Sign out of Rizzy", "Sign out of Fizzy")),
    }


def check(target):
    login(target)
    account = target["account"]
    status, _, raw = call(target, f"/{account}/account/join_code.json")
    assert status == 200, (target["name"], status, raw[:120])
    code = json.loads(raw)["code"]
    visitor = {**target, "opener": urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
    join_path = f"/{account}/join/{code}"
    status, _, page = request(visitor, join_path, accept="text/html")
    assert status == 200, (target["name"], status)
    csrf = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
    csrf = csrf.group(1).decode() if csrf else None
    form = urllib.parse.urlencode({"email_address": "member@example.com"}).encode()
    status, _, _ = request(visitor, join_path, "POST", form, accept="text/html", csrf=csrf)
    assert status == 302, (target["name"], status)
    status, _, raw = call(target, f"/{account}/users.json")
    assert status == 200, (target["name"], status)
    member = next(user for user in json.loads(raw)
                  if user["email_address"] == "member@example.com")
    status, _, raw = call(target, f"/{account}/users/{member['id']}.json", "PATCH",
                          {"user": {"name": "Ada Example"}})
    assert status == 200, (target["name"], status, raw[:120])
    profile_path = f"/{account}/users/{member['id']}"
    status, _, body = request(target, profile_path, accept="text/html")
    assert status == 200, (target["name"], status)
    unverified = summary(body, account)

    verified_at = datetime.datetime.now(datetime.timezone.utc).isoformat()
    with sqlite3.connect(target["db"]) as database:
        database.execute(
            "UPDATE users SET verified_at=? WHERE identity_id IN "
            "(SELECT id FROM identities WHERE email_address='member@example.com')",
            (verified_at,))
    status, _, body = request(target, profile_path, accept="text/html")
    assert status == 200, (target["name"], status)
    verified = summary(body, account)

    status, _, _ = call(target, profile_path + ".json", "DELETE")
    assert status == 204, (target["name"], status)
    status, _, _ = request(target, profile_path, accept="text/html")
    return {"unverified": unverified, "verified": verified, "removed_status": status}


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-member-profile-") as temporary:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env, credentials = prepare_source(directory, 2)
                else:
                    env = rust_env(directory)
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                    env, credentials = prepare_rust(directory, env)
                process, url = start_server(
                    name, directory, env, sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                results[name] = check({
                    "name": name,
                    "url": url,
                    "account": str(credentials["account"]),
                    "token": credentials["token"],
                    "email": "owner@example.com",
                    "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                    "opener": urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                        NoRedirect()),
                })
            assert results["fizzy"] == results["rizzy"], results
            assert results["fizzy"]["unverified"]["panels"] == 1, results
            assert not results["fizzy"]["unverified"]["has_frame"], results
            assert results["fizzy"]["verified"]["frame_src"] == "/:account/users/:id/events", results
            assert results["fizzy"]["removed_status"] == 404, results
            print("PASS unverified, verified, and removed member profile states match Fizzy")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
