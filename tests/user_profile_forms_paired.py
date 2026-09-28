#!/usr/bin/env python3
"""Compare populated profile-page form contracts with pinned Fizzy."""

import http.cookiejar
import base64
import hashlib
import json
import os
import re
import sqlite3
import sys
import tempfile
import urllib.parse
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from card_image_parity import call  # noqa: E402
from html_form_inventory import Forms, normalized  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)

PNG = (ROOT / "upstream/test/fixtures/files/avatar.png").read_bytes()


def check(target):
    login(target)
    account = target["account"]
    status, _, raw = request(target, f"/{account}/users", bearer=True)
    assert status == 200, (target["name"], status)
    owner = next(user for user in json.loads(raw) if user["role"] == "owner")
    forms = {}
    pages = {}
    for label, suffix in (("show", ""), ("edit", "/edit")):
        status, _, raw = request(target, f"/{account}/users/{owner['id']}{suffix}",
                                 accept="text/html")
        assert status == 200, (target["name"], label, status)
        parser = Forms(account)
        parser.feed(raw.decode())
        parser.close()
        pages[label] = raw
        forms[label] = [
            (normalized(urllib.parse.urlsplit(action).path, account)
             if action.startswith(("http://", "https://")) else action,
             method, fields)
            for action, method, fields in parser.forms
        ]
    csrf = re.search(rb'<meta name="csrf-token" content="([^"]+)"', pages["edit"])
    csrf = csrf.group(1).decode() if csrf else None
    blob = {"filename": "avatar.png", "byte_size": len(PNG),
            "checksum": base64.b64encode(hashlib.md5(PNG).digest()).decode(),
            "content_type": "image/png"}
    status, _, raw = call(target, f"/{account}/rails/active_storage/direct_uploads",
                          "POST", {"blob": blob})
    assert status == 200, (target["name"], status, raw[:120])
    upload = json.loads(raw)
    status, _, _ = call(target, upload["direct_upload"]["url"], "PUT", PNG,
                        bearer=False, headers=upload["direct_upload"]["headers"])
    assert status == 204, (target["name"], status)
    attached = urllib.parse.urlencode({"_method": "patch", "user[name]": owner["name"],
                                       "user[avatar]": upload["signed_id"]}).encode()
    status, _, _ = request(target, f"/{account}/users/{owner['id']}", "POST", attached,
                            accept="text/html", csrf=csrf)
    assert status == 302, (target["name"], status)
    avatar_path = f"/{account}/users/{owner['id']}/avatar"
    attached_status, _, _ = request(target, avatar_path, accept="text/html")
    attachment_table = ("active_storage_attachments" if target["name"] == "fizzy"
                        else "storage_attachments")
    with sqlite3.connect(target["db"]) as database:
        attached_count = database.execute(
            f"SELECT COUNT(*) FROM {attachment_table} WHERE record_type='User' AND name='avatar'").fetchone()[0]
    form = urllib.parse.urlencode({"_method": "delete"}).encode()
    avatar_status, avatar_headers, _ = request(
        target, avatar_path, "POST", form,
        accept="text/html", csrf=csrf)
    removed_status, _, _ = request(target, avatar_path, accept="image/svg+xml")
    with sqlite3.connect(target["db"]) as database:
        removed_count = database.execute(
            f"SELECT COUNT(*) FROM {attachment_table} WHERE record_type='User' AND name='avatar'").fetchone()[0]
    signout_status, signout_headers, _ = request(
        target, "/session", "POST", form, accept="text/html", csrf=csrf)
    after_signout, _, _ = request(target, f"/{account}/users/{owner['id']}",
                                  accept="text/html")
    def redirect_path(headers):
        location = headers.get("Location")
        return normalized(urllib.parse.urlsplit(location).path, account) if location else None
    return {"forms": forms,
            "profile_structure": tuple(marker in pages["show"] for marker in (
                b"settings--profile", b"settings__panel", b"Appearance",
                b"theme-switcher", b"Which cards are assigned to me?",
                b"Which cards were added by me?", b"Manage passkeys",
                b"Export your data")),
            "avatar_state": (attached_status, attached_count, removed_status, removed_count),
            "avatar_delete": (avatar_status, redirect_path(avatar_headers)),
            "signout": (signout_status, redirect_path(signout_headers), after_signout)}


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-user-forms-") as temporary:
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
            assert all(results["fizzy"]["profile_structure"]), results
            assert results["fizzy"]["avatar_state"] == (302, 1, 200, 0), results
            assert results["fizzy"]["signout"] == (302, "/session/new", 302), results
            print("PASS profile and edit forms, avatar deletion, and sign-out match Fizzy")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
