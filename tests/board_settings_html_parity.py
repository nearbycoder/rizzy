#!/usr/bin/env python3
"""Compare browser board creation and settings forms with pinned Fizzy."""
import http.cookiejar
import json
import os
import re
import sqlite3
import time
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request


def run(target):
    login(target)
    account = target["account"]
    new_path = f"/{account}/boards/new"
    status, _, html = request(target, new_path, accept="text/html")
    assert status == 200 and b"Create a new board" in html and b'board[name]' in html, (target["name"], status)
    csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', html)
    csrf = csrf_match.group(1).decode() if csrf_match else None
    title = f"Settings parity {time.time_ns()}"
    data = urllib.parse.urlencode({"board[name]": title}).encode()
    status, headers, body = request(target, f"/{account}/boards", "POST", data, accept="text/html", csrf=csrf)
    assert status == 302, (target["name"], status, body[:200])
    board_path = urllib.parse.urlparse(headers["Location"]).path
    match = re.fullmatch(rf"/{account}/boards/([^/]+)", board_path)
    assert match, board_path
    board_id = match.group(1)
    deleted = False
    try:
        status, _, raw = request(target, board_path + ".json", bearer=True)
        board = json.loads(raw)
        assert status == 200 and board["name"] == title and board["all_access"], (status, board)
        status, _, html = request(target, board_path + "/edit", accept="text/html")
        assert status == 200 and b"Board Settings" in html and title.encode() in html
        assert b"Who can access this board?" in html and b"Delete this board" in html
        options = []
        for tag in re.findall(rb'<input\b[^>]*>', html):
            if b'type="radio"' in tag and b'name="board[auto_postpone_period_in_days]"' in tag:
                value = re.search(rb'value="(\d+)"', tag)
                assert value, tag
                options.append(value.group(1))
        assert options == [b"3", b"7", b"30", b"90", b"365", b"11"], (target["name"], options)
        updated = title + " edited"
        form = urllib.parse.urlencode([
            ("_method", "patch"), ("board[name]", updated),
            ("board[all_access]", "0"), ("board[auto_postpone_period_in_days]", "11"),
        ]).encode()
        status, headers, body = request(target, board_path, "POST", form, accept="text/html", csrf=csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == board_path + "/edit", (target["name"], status, body[:200])
        status, _, raw = request(target, board_path + ".json", bearer=True)
        board = json.loads(raw)
        assert status == 200 and board["name"] == updated and not board["all_access"] and board["auto_postpone_period_in_days"] == 11, (status, board)
        status, headers, body = request(target, board_path, "DELETE", accept="text/html", csrf=csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == f"/{account}/", (target["name"], status, headers.get("Location"), body[:200])
        deleted = True
        status, _, _ = request(target, board_path + ".json", bearer=True)
        assert status == 404, status
        return {"new": True, "create_redirect": True, "edit": True,
                "entropy_options": [int(option) for option in options], "update_redirect": True,
                "saved_name_access_entropy": True, "delete_redirect": True}
    finally:
        if not deleted:
            request(target, board_path, "DELETE", bearer=True)


def main():
    fizzy_db = os.environ.get("FIZZY_DB_PATH", "upstream/storage/production.sqlite3")
    with sqlite3.connect(fizzy_db) as db:
        email = db.execute(
            "SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
            (int(os.environ["FIZZY_ACCOUNT"]),),
        ).fetchone()[0]
    targets = []
    for name in ("FIZZY", "RIZZY"):
        target = {"name": name.lower(), "url": os.environ[f"{name}_URL"],
                  "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
                  "email": email if name == "FIZZY" else os.environ.get("RIZZY_EMAIL", "owner@example.com"),
                  "db": fizzy_db}
        if name == "FIZZY":
            target["extra_headers"] = {"X-Forwarded-For": f"198.51.100.{time.time_ns() % 200 + 30}"}
        target["opener"] = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
        targets.append(target)
    source, rust = (run(target) for target in targets)
    assert source == rust, (source, rust)
    print("Board HTML settings matched: new, create, edit, update, and delete")


if __name__ == "__main__":
    main()
