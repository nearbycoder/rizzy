#!/usr/bin/env python3
"""Compare whether a User mention counts as a card file attachment."""

import http.cookiejar
import json
import os
import re
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
                          {"board": {"name": "Mention attachment flag"}})
    assert status == 201, (target["name"], status, raw[:120])
    board_id = json.loads(raw)["id"]
    status, _, prompt = request(target, f"/{account}/prompts/boards/{board_id}/users",
                                accept="text/html")
    assert status == 200, (target["name"], status)
    items = re.findall(rb'<lexxy-prompt-item search="([^"]+)" sgid="([^"]+)">', prompt)
    assert items, (target["name"], prompt[:300])
    sgid = items[0][1].decode()
    markup = (f'<p>Hello <action-text-attachment sgid="{sgid}" '
              'content-type="application/vnd.actiontext.mention"></action-text-attachment></p>')
    status, _, raw = call(target, f"/{account}/boards/{board_id}/cards", "POST",
                          {"card": {"title": "Mention", "description": markup}})
    assert status == 201, (target["name"], status, raw[:120])
    card = json.loads(raw)
    assert 'content-type="application/vnd.actiontext.mention"' in card["description_html"], (
        target["name"], card)
    status, _, raw = call(target, f"/{account}/cards/{card['number']}.json")
    assert status == 200, (target["name"], status, raw[:120])
    reread = json.loads(raw)
    assert card["has_attachments"] == reread["has_attachments"], (target["name"], card, reread)
    return card["has_attachments"]


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-mention-attachment-") as temporary:
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
            assert results["fizzy"] == results["rizzy"] == False, results
            print("PASS User mentions do not count as file attachments")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
