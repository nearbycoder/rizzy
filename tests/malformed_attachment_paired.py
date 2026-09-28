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

CASES = {
    "relative image": '<action-text-attachment url="image.png" content-type="image/png" caption="Relative"></action-text-attachment>',
    "remote image": '<action-text-attachment url="https://example.com/image.png" content-type="image/png" caption="Remote"></action-text-attachment>',
    "remote image entity caption": '<action-text-attachment url="https://example.com/image.png" content-type="image/png" caption="A &amp; B"></action-text-attachment>',
    "remote image quoted greater-than": '<action-text-attachment url="https://example.com/image.png" content-type="image/png" caption="A > B"></action-text-attachment>',
    "remote image escaped greater-than": '<action-text-attachment url="https://example.com/image.png" content-type="image/png" caption="A &gt; B"></action-text-attachment>',
    "self-closing remote image": '<action-text-attachment url="https://example.com/image.png" content-type="image/png" caption="Self-closing" />',
    "remote image empty SGID": '<action-text-attachment sgid="" url="https://example.com/image.png" content-type="image/png"></action-text-attachment>',
    "remote image dimensions": '<action-text-attachment url="https://example.com/image.png" content-type="image/png" width="80" height="60"></action-text-attachment>',
    "remote image with unresolved SGID": '<action-text-attachment sgid="gid://fizzy/Card/missing" url="https://example.com/image.png" content-type="image/png" caption="Fallback"></action-text-attachment>',
    "remote video": '<action-text-attachment url="https://example.com/video.mp4" content-type="video/mp4" caption="Remote"></action-text-attachment>',
    "remote video filename": '<action-text-attachment url="video.mp4" content-type="video/mp4" filename="movie.mp4"></action-text-attachment>',
    "unsafe image URL": '<action-text-attachment url="data:text/html,PAYLOAD" content-type="image/png" caption="Danger"></action-text-attachment>',
    "unsafe video URL": '<action-text-attachment url="javascript:alert(1)" content-type="video/mp4" caption="Danger"></action-text-attachment>',
    "javascript URL": '<action-text-attachment url="javascript:alert(1)" content-type="image/png" caption="Danger"></action-text-attachment>',
}


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
    matrix = {}
    for name, markup in CASES.items():
        status, _, raw = call(target, f"/{account}/cards/{card['number']}", "PATCH",
                              {"card": {"description": markup}})
        assert status == 200, (target["name"], name, status, raw[:120])
        updated = json.loads(raw)
        matrix[name] = {key: updated[key] for key in
                        ("description", "description_html", "has_attachments")}
    return {"description": card["description_html"], "comment": comment["body"],
            "matrix": matrix}


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
            print("PASS unresolved and remote Action Text attachments match Fizzy")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
