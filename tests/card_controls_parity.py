#!/usr/bin/env python3
"""Compare pin and watch frames, ETags, Turbo streams, and JSON toggles."""
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


def csrf(body):
    match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', body)
    return match.group(1).decode() if match else None


def check_control(target, number, kind):
    path = f"/{target['account']}/cards/{number}/{kind}"
    off_label, on_label = ("Pin this card", "Unpin this card") if kind == "pin" else ("Watch this", "Stop watching")
    if kind == "watch":
        status, _, _ = request(target, path, "DELETE", bearer=True)
        assert status == 204, (target["name"], kind, "reset", status)
    status, _, card_page = request(target, f"/{target['account']}/cards/{number}", accept="text/html")
    assert status == 200, (target["name"], kind, "card page", status)
    frame = re.search(rb'<turbo-frame\b(?=[^>]*id="' + kind.encode() + rb'_card_[^"]+")(?=[^>]*src="' + re.escape(path.encode()) + rb'")[^>]*>', card_page)
    assert frame, (target["name"], kind, "lazy card frame")
    form = re.search(rb'<form\b[^>]*action="' + re.escape(path.encode()) + rb'"[^>]*>(.*?)</form>',
                     card_page, re.S)
    assert form and off_label.encode() in form.group(1), (target["name"], kind, "card action form")
    status, headers, page = request(target, path, accept="text/html")
    assert status == 200 and headers.get("ETag"), (target["name"], kind, "show", status)
    assert f'id="{kind}_card_'.encode() in page and f'id="{kind}_button_card_'.encode() in page
    assert off_label.encode() in page and on_label.encode() not in page
    tag_before = headers["ETag"]
    status, _, _ = request({**target, "extra_headers":{"If-None-Match":tag_before}}, path, accept="text/html")
    assert status == 304, (target["name"], kind, "unchanged", status)
    token = csrf(page)
    status, headers, stream = request(target, path, "POST", b"", accept="text/vnd.turbo-stream.html", csrf=csrf(card_page) or token)
    assert status == 200 and headers.get_content_type() == "text/vnd.turbo-stream.html", (target["name"], kind, "turbo create", status)
    assert b'<turbo-stream action="replace"' in stream and f'{kind}_button_card_'.encode() in stream and on_label.encode() in stream
    if kind == "watch":
        assert b'comment_watchers_card_' in stream and b'Subscribers' in stream
    status, headers, page = request({**target, "extra_headers":{"If-None-Match":tag_before}}, path, accept="text/html")
    assert status == 200 and headers["ETag"] != tag_before and on_label.encode() in page, (target["name"], kind, "changed", status)
    status, _, _ = request({**target, "extra_headers":{"If-None-Match":headers["ETag"]}}, path, accept="text/html")
    assert status == 304, (target["name"], kind, "unchanged active", status)
    active_form = re.search(rb'<form\b[^>]*action="' + re.escape(path.encode()) + rb'"[^>]*>(.*?)</form>',
                            page, re.S)
    assert active_form and b'name="_method" value="delete"' in active_form.group(1), (target["name"], kind, "delete form")
    status, _, stream = request(target, path, "POST", urllib.parse.urlencode({"_method": "delete"}).encode(),
                                accept="text/vnd.turbo-stream.html", csrf=csrf(card_page))
    assert status == 200 and off_label.encode() in stream, (target["name"], kind, "turbo destroy", status)
    status, _, _ = request(target, path, "POST", b"", bearer=True)
    assert status == 204, (target["name"], kind, "JSON create", status)
    status, _, _ = request(target, path, "DELETE", bearer=True)
    assert status == 204, (target["name"], kind, "JSON destroy", status)
    status, _, _ = request(target, path + ".json", bearer=True)
    assert status == 406, (target["name"], kind, "JSON show", status)


def check(target):
    login(target)
    account = target["account"]
    title = f"Control parity {time.time_ns()}"
    status, _, raw = request(target, f"/{account}/boards", "POST", {"board":{"name":title}}, bearer=True)
    assert status == 201, (target["name"], "board", status)
    board_id = json.loads(raw)["id"]
    try:
        status, _, raw = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                 {"card":{"title":title}}, bearer=True)
        assert status == 201, (target["name"], "card", status)
        number = json.loads(raw)["number"]
        for kind in ("pin", "watch"):
            check_control(target, number, kind)
    finally:
        status, _, _ = request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)
        assert status == 204, (target["name"], "cleanup", status)
    return {"show":200,"cached":304,"turbo":200,"json":204,"json_show":406}


def main():
    source_db = os.environ.get("FIZZY_DB", "upstream/storage/production.sqlite3")
    with sqlite3.connect(source_db) as db:
        email = db.execute("SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1", (int(os.environ["FIZZY_ACCOUNT"]),)).fetchone()[0]
    os.environ["FIZZY_EMAIL"] = email
    source, rust = check(target("FIZZY")), check(target("RIZZY"))
    assert source == rust
    print("Pin/watch parity matched: HTML states, ETags, Turbo streams, and JSON toggles")


if __name__ == "__main__":
    main()
