#!/usr/bin/env python3
"""Compare Fizzy's request-timezone ETag behavior on common cached routes."""

import http.cookiejar
import json
import os
import sqlite3
import time
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request
from platform_html_parity import cookie_for


def set_timezone(target, name):
    host = urllib.parse.urlparse(target["url"]).hostname
    try:
        target["jar"].clear(host, "/", "timezone")
    except KeyError:
        pass
    if name:
        target["jar"].set_cookie(cookie_for(target, "timezone", name))


def probe(target, name, path, accept):
    set_timezone(target, None)
    status, headers, body = request(target, path, accept=accept)
    assert status == 200 and body, (target["name"], name, "initial", status)
    initial = headers.get("ETag")
    assert initial, (target["name"], name, "missing ETag")

    target["extra_headers"]["If-None-Match"] = initial
    status, _, body = request(target, path, accept=accept)
    assert status == 304 and not body, (target["name"], name, "same zone", status)

    set_timezone(target, "America/New_York")
    status, headers, body = request(target, path, accept=accept)
    assert status == 200 and body, (target["name"], name, "New York", status)
    new_york = headers.get("ETag")
    assert new_york and new_york != initial
    target["extra_headers"]["If-None-Match"] = new_york
    status, _, body = request(target, path, accept=accept)
    assert status == 304 and not body, (target["name"], name, "New York repeat", status)

    set_timezone(target, "America/Los_Angeles")
    status, headers, body = request(target, path, accept=accept)
    assert status == 200 and body, (target["name"], name, "Los Angeles", status)
    assert headers.get("ETag") not in (initial, new_york)

    set_timezone(target, "Not/A_Time_Zone")
    target["extra_headers"]["If-None-Match"] = initial
    status, headers, body = request(target, path, accept=accept)
    assert status == 304 and not body and headers.get("ETag") == initial, (
        target["name"], name, "invalid zone", status,
    )
    target["extra_headers"].pop("If-None-Match")
    set_timezone(target, None)
    return (200, 304, 200, 304, 200, 304)


def run(target):
    login(target)
    status, _, raw = request(target, f"/{target['account']}/users", bearer=True)
    assert status == 200
    owner = next(user for user in json.loads(raw) if user["role"] == "owner")
    status, _, raw = request(
        target, f"/{target['account']}/boards", "POST",
        {"board": {"name": f"Timezone ETag parity {time.time_ns()}"}}, bearer=True,
    )
    assert status == 201
    board_id = json.loads(raw)["id"]
    try:
        return {
            "avatar": probe(target, "avatar", f"/{target['account']}/users/{owner['id']}/avatar", "image/svg+xml"),
            "board": probe(target, "board", f"/{target['account']}/boards/{board_id}", "text/html"),
            "tray": probe(target, "tray", f"/{target['account']}/notifications/tray", "text/html"),
        }
    finally:
        set_timezone(target, None)
        target["extra_headers"].pop("If-None-Match", None)
        status, _, _ = request(target, f"/{target['account']}/boards/{board_id}", "DELETE", bearer=True)
        assert status == 204


def main():
    source_db = os.environ.get("FIZZY_DB_PATH", "upstream/storage/production.sqlite3")
    with sqlite3.connect(source_db) as db:
        source_email = db.execute(
            "SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id "
            "WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
            (int(os.environ["FIZZY_ACCOUNT"]),),
        ).fetchone()[0]
    targets = []
    for name in ("FIZZY", "RIZZY"):
        jar = http.cookiejar.CookieJar()
        targets.append({
            "name": name.lower(), "url": os.environ[f"{name}_URL"].rstrip("/"),
            "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
            "email": source_email if name == "FIZZY" else os.environ["RIZZY_EMAIL"],
            "db": source_db, "jar": jar,
            "opener": urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar), NoRedirect()),
            "extra_headers": {"X-Forwarded-For": f"198.51.100.{time.time_ns() % 200 + 30}"} if name == "FIZZY" else {},
        })
    source, rust = (run(target) for target in targets)
    assert source == rust, (source, rust)
    print("Timezone ETag decisions matched on avatar, board, and notification tray")


if __name__ == "__main__":
    main()
