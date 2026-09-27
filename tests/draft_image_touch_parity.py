#!/usr/bin/env python3
"""Compare draft image attachment and removal timestamp effects with Fizzy."""
import base64
import hashlib
import http.cookiejar
import json
import os
import re
import sqlite3
import time
import urllib.request

from card_image_parity import IMAGE, call
from draft_workflow_parity import NoRedirect, login, request


def times(target, number):
    with sqlite3.connect(target["db"]) as db:
        return db.execute(
            "SELECT c.updated_at,c.last_active_at,b.updated_at FROM cards c "
            "JOIN boards b ON b.id=c.board_id WHERE c.number=?", (number,)
        ).fetchone()


def changes(before, after):
    return tuple(left != right for left, right in zip(before, after))


def run(target):
    login(target)
    account = target["account"]
    status, _, raw = request(target, f"/{account}/boards", "POST",
                             {"board": {"name": "Draft image timestamps"}}, bearer=True)
    assert status == 201, (status, raw[:150])
    board = json.loads(raw)
    board_path = f"/{account}/boards/{board['id']}"
    try:
        status, _, html = request(target, board_path, accept="text/html")
        assert status == 200, status
        token = re.search(rb'<meta name="csrf-token" content="([^"]+)"', html)
        csrf = token.group(1).decode() if token else None
        status, headers, _ = request(target, board_path + "/cards", "POST", b"",
                                     accept="text/html", csrf=csrf)
        assert status == 302, status
        number = int(re.search(r"/cards/(\d+)/draft", headers["Location"]).group(1))
        initial = times(target, number)

        blob = {"filename": "moon.jpg", "byte_size": len(IMAGE),
                "checksum": base64.b64encode(hashlib.md5(IMAGE).digest()).decode(),
                "content_type": "image/jpeg"}
        status, _, raw = call(target, f"/{account}/rails/active_storage/direct_uploads",
                              "POST", {"blob": blob})
        assert status == 200, (status, raw[:150])
        upload = json.loads(raw)
        status, _, raw = call(target, upload["direct_upload"]["url"], "PUT", IMAGE,
                              bearer=False, headers=upload["direct_upload"]["headers"])
        assert status == 204, (status, raw[:150])
        time.sleep(.05)
        status, _, raw = request(target, f"/{account}/cards/{number}.json", "PATCH",
                                 {"card": {"image": upload["signed_id"]}}, bearer=True)
        assert status == 200 and json.loads(raw)["image_url"], (status, raw[:150])
        attached = times(target, number)
        time.sleep(.05)
        status, _, raw = request(target, f"/{account}/cards/{number}/image.json", "DELETE",
                                 bearer=True)
        assert status == 204, (status, raw[:150])
        removed = times(target, number)
        return {"attach": changes(initial, attached), "remove": changes(attached, removed)}
    finally:
        status, _, _ = request(target, board_path, "DELETE", bearer=True)
        assert status == 204, status


def main():
    source_db = os.environ["FIZZY_DB_PATH"]
    with sqlite3.connect(source_db) as db:
        source_email = db.execute(
            "SELECT i.email_address FROM identities i JOIN users u ON u.identity_id=i.id "
            "JOIN accounts a ON a.id=u.account_id WHERE a.external_account_id=? "
            "AND u.role='owner' LIMIT 1", (os.environ["FIZZY_ACCOUNT"],)
        ).fetchone()[0]
    results = {}
    for name in ("FIZZY", "RIZZY"):
        target = {"name": name.lower(), "url": os.environ[f"{name}_URL"],
                  "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
                  "db": source_db if name == "FIZZY" else os.environ["RIZZY_DB"],
                  "email": source_email if name == "FIZZY" else os.environ.get("RIZZY_EMAIL", "owner@example.com"),
                  "opener": urllib.request.build_opener(
                      urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
        results[name] = run(target)
    assert results["FIZZY"] == results["RIZZY"], results
    print("Draft image timestamps passed on FIZZY, RIZZY:", results["RIZZY"])


if __name__ == "__main__":
    main()
