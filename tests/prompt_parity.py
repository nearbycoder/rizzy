#!/usr/bin/env python3
"""Compare Fizzy and Rizzy's User mention picker markup and cache behavior."""
import html
import http.cookiejar
import json
import os
import re
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from draft_workflow_parity import NoRedirect, login, request


def items(body):
    found = []
    for search, sgid, inner in re.findall(
            rb'<lexxy-prompt-item search="([^"]+)" sgid="([^"]+)">(.*?)</lexxy-prompt-item>',
            body, re.S):
        def template(kind):
            match = re.search(rb'<template type="' + kind + rb'">(.*?)</template>', inner, re.S)
            assert match, inner
            return " ".join(html.unescape(re.sub(rb'<[^>]+>', b'', match.group(1)).decode()).split())
        found.append((html.unescape(search.decode()), template(b'menu'), template(b'editor'), sgid.decode()))
    return found


def check_prompt(target, path):
    status, headers, body = request(target, path, accept="text/html")
    assert status == 200 and headers.get_content_type() == "text/html", (status, headers)
    assert headers.get("ETag") and body
    parsed = items(body)
    assert parsed and all(sgid for _, _, _, sgid in parsed)
    assert body.count(b'<img aria-hidden="true"') == 2 * len(parsed)
    assert len(re.findall(rb'src="/\d+/users/[^"/]+/avatar" width="48" height="48"', body)) == 2 * len(parsed)
    conditional = urllib.request.Request(
        target["url"] + path,
        headers={"Accept": "text/html", "If-None-Match": headers["ETag"]})
    try:
        response = target["opener"].open(conditional, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        assert response.status == 304
    return [(search, menu, editor) for search, menu, editor, _ in parsed]


def main():
    source_db = Path(os.environ.get("FIZZY_DB_PATH", ROOT / "upstream/storage/production.sqlite3"))
    source_account = int(os.environ["FIZZY_ACCOUNT"])
    with sqlite3.connect(source_db) as db:
        source_user = db.execute(
            "SELECT i.email_address,u.name FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
            (source_account,)).fetchone()
    assert source_user
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        rust_db = Path(temp) / "prompts.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{rust_db}",
               "RIZZY_SEED_EMAIL": source_user[0], "RIZZY_SEED_NAME": source_user[1]}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        rust_account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
        rust_token = re.search(r"Bearer token: (\w+)", seeded.stdout).group(1)
        port, server = start(env)
        try:
            targets = [
                {"name": "fizzy", "url": os.environ.get("FIZZY_URL", "http://127.0.0.1:3006"),
                 "account": str(source_account), "token": os.environ["FIZZY_TOKEN"],
                 "email": source_user[0], "db": str(source_db)},
                {"name": "rizzy", "url": f"http://127.0.0.1:{port}", "account": rust_account,
                 "token": rust_token, "email": source_user[0], "db": str(rust_db)},
            ]
            for target in targets:
                target["opener"] = urllib.request.build_opener(
                    urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
                login(target)
            global_results = [check_prompt(t, f"/{t['account']}/prompts/users") for t in targets]
            assert global_results[0] == global_results[1], global_results
            board_results = []
            for target in targets:
                status, _, body = request(target, f"/{target['account']}/boards", "POST",
                                          {"board": {"name": f"Prompt parity {time.time_ns()}", "all_access": False}},
                                          bearer=True)
                assert status == 201, (status, body[:200])
                board_id = json.loads(body)["id"]
                try:
                    board_results.append(check_prompt(
                        target, f"/{target['account']}/prompts/boards/{board_id}/users"))
                finally:
                    status, _, _ = request(target, f"/{target['account']}/boards/{board_id}", "DELETE", bearer=True)
                    assert status == 204
            assert board_results[0] == board_results[1], board_results
            print("User mention prompts matched: search/menu/editor text, board scope, ETag revalidation")
        finally:
            stop(server)


if __name__ == "__main__":
    main()
