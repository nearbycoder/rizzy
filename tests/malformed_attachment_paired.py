#!/usr/bin/env python3
"""Compare unresolved Action Text attachments on live Fizzy and Rizzy cards."""

import http.cookiejar
import json
import os
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from card_image_parity import call  # noqa: E402
from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def check(target):
    login(target)
    account = target["account"]
    status, _, raw = call(target, f"/{account}/boards", "POST",
                          {"board": {"name": "Malformed attachment"}})
    assert status == 201, (target["name"], status, raw[:120])
    board = json.loads(raw)
    unresolved = ('<action-text-attachment sgid="gid://fizzy/Card/nonexistent" '
                  'content-type="application/octet-stream" '
                  'onclick="alert(1)"></action-text-attachment>')
    status, _, raw = call(target, f"/{account}/boards/{board['id']}/cards", "POST",
                          {"card": {"title": "Attachment", "description": unresolved}})
    assert status == 201, (target["name"], status, raw[:120])
    card = json.loads(raw)
    malformed = ('<action-text-attachment url="image.png" content-type="image/*" '
                 'presentation="gallery" onerror="alert(1)"></action-text-attachment>')
    status, _, raw = call(target, f"/{account}/cards/{card['number']}/comments", "POST",
                          {"comment": {"body": malformed}})
    assert status == 201, (target["name"], status, raw[:120])
    comment = json.loads(raw)
    for suffix in ("", "/edit"):
        status, headers, page = request(
            target, f"/{account}/cards/{card['number']}{suffix}", accept="text/html")
        assert status == 200 and headers.get_content_type() == "text/html", (
            target["name"], suffix, status)
        assert b"Attachment" in page, (target["name"], suffix)
    return {"description": card["description_html"], "comment": comment["body"]}


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-malformed-attachment-") as temporary:
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
            print("PASS unresolved card and comment attachments match Fizzy")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
