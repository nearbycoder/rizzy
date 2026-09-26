#!/usr/bin/env python3
"""Exercise the browser profile form's direct-upload and removal path."""
import base64
import hashlib
import json
import os
import sqlite3
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from card_image_parity import NoRedirect, call
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench"))
from events_html import browser_cookie


IMAGE = (Path(__file__).resolve().parents[1] / "upstream/test/fixtures/files/avatar.png").read_bytes()


def browser_request(target, path, method="GET", fields=None, accept="text/html"):
    headers = {"Accept": accept, "Cookie": target["cookie"]}
    body = None
    if fields is not None:
        headers["Content-Type"] = "application/x-www-form-urlencoded"
        body = urllib.parse.urlencode(fields).encode()
    request = urllib.request.Request(target["url"] + path, data=body, headers=headers, method=method)
    try:
        response = urllib.request.build_opener(NoRedirect()).open(request, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


def main():
    target = {key.lower(): os.environ[f"RIZZY_{key}"] for key in
              ("URL", "ACCOUNT", "TOKEN", "EMAIL", "DB")}
    target["name"] = "rizzy"
    target["cookie"] = browser_cookie(target)
    with sqlite3.connect(target["db"]) as db:
        user_id = db.execute("SELECT id FROM users WHERE role='owner' AND active=1 LIMIT 1").fetchone()[0]
        original_name = db.execute("SELECT name FROM users WHERE id=?", (user_id,)).fetchone()[0]
    prefix = f"/{target['account']}"
    profile = f"{prefix}/users/{user_id}"
    status, _, form = browser_request(target, profile + "/edit")
    assert status == 200 and b'name="user[avatar]"' in form
    assert b"/assets/lexxy/activestorage.esm.js" in form
    assert b"data-direct-upload-url=" in form

    blob = {"filename": "avatar.png", "byte_size": len(IMAGE),
            "checksum": base64.b64encode(hashlib.md5(IMAGE).digest()).decode(),
            "content_type": "image/png"}
    status, _, raw = call(target, prefix + "/rails/active_storage/direct_uploads", "POST", {"blob": blob})
    assert status == 200, (status, raw[:100])
    upload = json.loads(raw)
    status, _, _ = call(target, upload["direct_upload"]["url"], "PUT", IMAGE,
                        bearer=False, headers=upload["direct_upload"]["headers"])
    assert status == 204, status
    status, headers, _ = browser_request(target, profile, "POST", {
        "_method": "patch", "user[name]": original_name, "user[avatar]": upload["signed_id"]})
    assert status == 302 and headers["Location"] == profile, (status, headers)
    with sqlite3.connect(target["db"]) as db:
        active, attachments = db.execute(
            "SELECT u.active, (SELECT COUNT(*) FROM storage_attachments WHERE record_type='User' AND name='avatar' AND record_id=u.id) FROM users u WHERE u.id=?",
            (user_id,)).fetchone()
        assert (active, attachments) == (1, 1), (active, attachments)
    status, _, _ = call(target, profile + "/avatar", bearer=False)
    assert status == 302, status

    status, _, form = browser_request(target, profile + "/edit")
    assert status == 200 and b'id="remove-avatar"' in form
    assert b"Accept:'application/json'" in form
    status, _, _ = browser_request(target, profile + "/avatar", "DELETE", accept="application/json")
    assert status == 204, status
    with sqlite3.connect(target["db"]) as db:
        active, attachments = db.execute(
            "SELECT u.active, (SELECT COUNT(*) FROM storage_attachments WHERE record_type='User' AND name='avatar' AND record_id=u.id) FROM users u WHERE u.id=?",
            (user_id,)).fetchone()
        assert (active, attachments) == (1, 0), (active, attachments)
    status, _, _ = browser_request(target, profile, "POST", {
        "_method": "patch", "user[name]": original_name})
    assert status == 302, status
    print("Profile avatar form upload, attachment, removal, and owner retention passed")


if __name__ == "__main__":
    main()
