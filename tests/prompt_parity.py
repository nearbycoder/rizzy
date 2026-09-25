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
import urllib.parse
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


def check_card_prompt(target, path):
    status, headers, body = request(target, path, accept="text/html")
    assert status == 200 and headers.get_content_type() == "text/html" and headers.get("ETag")
    parsed = []
    for inner in re.findall(rb'<lexxy-prompt-item>(.*?)</lexxy-prompt-item>', body, re.S):
        menu = re.search(rb'<template type="menu">(.*?)</template>', inner, re.S)
        editor = re.search(rb'<template type="editor">(.*?)</template>', inner, re.S)
        anchor = re.search(rb'<a href="/\d+/cards/(\d+)">([^<]+)</a>', editor.group(1) if editor else b'')
        assert menu and anchor, inner
        menu_label = " ".join(html.unescape(re.sub(rb'<[^>]+>', b'', menu.group(1)).decode()).split())
        editor_label = html.unescape(anchor.group(2).decode())
        assert menu_label == editor_label
        parsed.append((int(anchor.group(1)), re.sub(r'^#\d+', '#N', menu_label)))
    conditional = urllib.request.Request(
        target["url"] + path,
        headers={"Accept": "text/html", "If-None-Match": headers["ETag"]})
    try:
        response = target["opener"].open(conditional, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        assert response.status == 304
    return parsed


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
            card_results = []
            marker = f"promptmarker{time.time_ns()}"
            comment_marker = f"commentmarker{time.time_ns()}"
            for target in targets:
                status, _, body = request(target, f"/{target['account']}/boards", "POST",
                                          {"board": {"name": f"Prompt parity {time.time_ns()}", "all_access": False}},
                                          bearer=True)
                assert status == 201, (status, body[:200])
                board_id = json.loads(body)["id"]
                try:
                    board_results.append(check_prompt(
                        target, f"/{target['account']}/prompts/boards/{board_id}/users"))
                    created = []
                    for index in range(12):
                        status, _, raw = request(target, f"/{target['account']}/boards/{board_id}/cards", "POST",
                                                 {"card": {"title": f"{marker} {index}"}}, bearer=True)
                        assert status == 201
                        created.append(json.loads(raw)["number"])
                    status, _, raw = request(target, f"/{target['account']}/boards/{board_id}/cards", "POST",
                                             {"card": {"title": "Comment target"}}, bearer=True)
                    assert status == 201
                    comment_number = json.loads(raw)["number"]
                    status, _, _ = request(target, f"/{target['account']}/cards/{comment_number}/comments", "POST",
                                           {"comment": {"body": comment_marker}}, bearer=True)
                    assert status == 201
                    next_number = comment_number + 1
                    status, _, raw = request(target, f"/{target['account']}/boards/{board_id}/cards", "POST",
                                             {"card": {"title": "Reference exact", "description": f"Number {next_number}"}}, bearer=True)
                    assert status == 201
                    numeric_number = json.loads(raw)["number"]
                    assert numeric_number == next_number
                    board_path = f"/{target['account']}/boards/{board_id}"
                    status, _, board_html = request(target, board_path, accept="text/html")
                    assert status == 200
                    csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', board_html)
                    csrf = csrf_match.group(1).decode() if csrf_match else None
                    status, draft_headers, _ = request(target, board_path + "/cards", "POST", b"", accept="text/html", csrf=csrf)
                    assert status == 302
                    draft_match = re.search(r"/cards/(\d+)/draft$", urllib.parse.urlparse(draft_headers["Location"]).path)
                    assert draft_match
                    draft_number = int(draft_match.group(1))
                    draft_title = f"draftprompt{marker}"
                    status, _, raw = request(target, f"/{target['account']}/cards/{draft_number}.json", "PATCH",
                                             {"card": {"title": draft_title}}, bearer=True)
                    assert status == 200 and json.loads(raw)["status"] == "drafted"
                    base_path = f"/{target['account']}/prompts/cards"
                    unfiltered = check_card_prompt(target, base_path)
                    filtered = check_card_prompt(target, base_path + "?filter=" + urllib.parse.quote(marker))
                    by_comment = check_card_prompt(target, base_path + "?filter=" + urllib.parse.quote(comment_marker))
                    exact = check_card_prompt(target, base_path + f"?filter={numeric_number}")
                    draft_search = check_card_prompt(target, base_path + "?filter=" + urllib.parse.quote(draft_title))
                    draft_exact = check_card_prompt(target, base_path + f"?filter={draft_number}")
                    assert len(filtered) == 10 and [number for number, _ in filtered] == list(reversed(created[-10:])), filtered
                    assert len(by_comment) == 1 and by_comment[0][0] == comment_number, by_comment
                    assert len(exact) == 2 and all(number == numeric_number for number, _ in exact), exact
                    assert all(number != draft_number for number, _ in unfiltered)
                    assert not draft_search and len(draft_exact) == 1 and draft_exact[0][0] == draft_number
                    card_results.append({"latest_titles": [title for _, title in unfiltered[:2]],
                                         "filtered_titles": [title for _, title in filtered],
                                         "comment_titles": [title for _, title in by_comment],
                                         "exact_titles": [title for _, title in exact],
                                         "draft_exact_titles": [title for _, title in draft_exact]})
                finally:
                    status, _, _ = request(target, f"/{target['account']}/boards/{board_id}", "DELETE", bearer=True)
                    assert status == 204
            assert board_results[0] == board_results[1], board_results
            assert card_results[0] == card_results[1], card_results
            print("User and card prompts matched: markup, board scope, search, ordering, cap, exact and draft numbers, ETag")
        finally:
            stop(server)


if __name__ == "__main__":
    main()
