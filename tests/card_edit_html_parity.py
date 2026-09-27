#!/usr/bin/env python3
"""Exercise the card edit page in Fizzy and Rizzy with equal card data."""
import json
import os
import re
import sqlite3
import time
import http.cookiejar
import urllib.request

from draft_workflow_parity import NoRedirect, login, request


def target(name):
    return {"name": name.lower(), "url": os.environ[f"{name}_URL"],
            "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
            "email": os.environ[f"{name}_EMAIL"], "db": os.environ[f"{name}_DB"],
            "opener": urllib.request.build_opener(
                urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}


def check(target):
    login(target)
    account = target["account"]
    title = f"Edit parity {time.time_ns()}"
    status, _, raw = request(target, f"/{account}/boards", "POST", {"board":{"name":title}}, bearer=True)
    assert status == 201, (target["name"], "board", status)
    board = json.loads(raw)
    try:
        status, _, raw = request(target, f"/{account}/boards/{board['id']}/cards", "POST",
                                 {"card":{"title":title,"description":"A description for editing"}}, bearer=True)
        assert status == 201, (target["name"], "card", status, raw[:100])
        card = json.loads(raw)
        number = card["number"]
        path = f"/{account}/cards/{number}/edit"
        status, _, body = request(target, path, accept="text/html")
        assert status == 200, (target["name"], "edit page", status)
        for marker in (b'id="global-container"', b'id="header"', b'id="main"', b'id="footer"'):
            assert marker in body, (target["name"], "full edit shell", marker)
        for marker in (b'<turbo-frame id="edit_card_', b'<form id="edit_form_card_',
                       b'name="card[title]"', b'name="card[description]"', b'<lexxy-editor',
                       b'Close editor and discard changes', title.encode(), b'A description for editing'):
            assert marker in body, (target["name"], marker)
        frame_id = f"edit_card_{card['id']}"
        frame_request = urllib.request.Request(target["url"] + path, headers={
            "Accept": "text/html", "Turbo-Frame": frame_id})
        with target["opener"].open(frame_request, timeout=10) as response:
            frame_body = response.read()
            assert response.status == 200, (target["name"], "frame edit", response.status)
        assert f'<turbo-frame id="{frame_id}"'.encode() in frame_body
        assert b'id="global-container"' not in frame_body, (target["name"], "frame layout")
        assert len(frame_body) < len(body), (target["name"], "frame response size")
        status, _, _ = request(target, path, accept="application/json", bearer=True)
        assert status == 406, (target["name"], "JSON edit", status)
        status, _, raw = request(target, f"/{account}/cards/{number}", "PATCH",
                                 {"card":{"title":title+' updated'}}, bearer=True)
        assert status == 200 and json.loads(raw)["title"] == title+' updated', (target["name"], "save", status)
        status, _, body = request(target, path, accept="text/html")
        assert status == 200 and (title+' updated').encode() in body, (target["name"], "reloaded edit", status)
    finally:
        status, _, _ = request(target, f"/{account}/boards/{board['id']}", "DELETE", bearer=True)
        assert status == 204, (target["name"], "cleanup", status)
    return {"edit":200,"frame":200,"json":406,"save":200,"reload":200}


def main():
    source_db = os.environ.get("FIZZY_DB", "upstream/storage/production.sqlite3")
    with sqlite3.connect(source_db) as db:
        email = db.execute("SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1", (int(os.environ["FIZZY_ACCOUNT"]),)).fetchone()[0]
    os.environ["FIZZY_EMAIL"] = email
    source, rust = check(target("FIZZY")), check(target("RIZZY"))
    assert source == rust
    print("Card edit parity matched: form, Lexxy, JSON rejection, save, reload")


if __name__ == "__main__":
    main()
