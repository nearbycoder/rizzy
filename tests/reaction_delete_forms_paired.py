#!/usr/bin/env python3
"""Compare card and comment reaction delete forms on fresh paired apps."""

import http.cookiejar
import json
import os
import re
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def check(target):
    login(target)
    account = target["account"]
    status, _, body = request(target, f"/{account}/boards", "POST",
                             {"board": {"name": f"Reaction form {time.time_ns()}"}}, bearer=True)
    assert status == 201
    board_id = json.loads(body)["id"]
    try:
        status, _, body = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                 {"card": {"title": "Reaction form"}}, bearer=True)
        assert status == 201
        number = json.loads(body)["number"]
        card_path = f"/{account}/cards/{number}"
        status, _, _ = request(target, card_path + "/publish", "POST", b"", bearer=True)
        assert status == 201
        status, _, body = request(target, card_path + "/comments", "POST",
                                 {"comment": {"body": "React here"}}, bearer=True)
        assert status == 201
        comment_id = json.loads(body)["id"]
        outcomes = []
        for path in (card_path + "/reactions",
                     card_path + f"/comments/{comment_id}/reactions"):
            status, _, body = request(target, path, "POST", {"reaction": {"content": "👍"}}, bearer=True)
            assert status == 201
            reaction_id = json.loads(body)["id"]
            delete_path = path + "/" + reaction_id
            status, _, page = request(target, card_path, accept="text/html")
            assert status == 200
            pattern = rb'<form\b[^>]*action="' + re.escape(delete_path.encode()) + rb'"[^>]*>(.*?)</form>'
            match = re.search(pattern, page, re.S)
            assert match and b'name="_method" value="delete"' in match.group(1), (
                target["name"], delete_path, "delete form")
            token_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
            csrf = token_match.group(1).decode() if token_match else None
            status, headers, stream = request(target, delete_path, "POST", b"_method=delete",
                                               accept="text/vnd.turbo-stream.html", csrf=csrf)
            assert status == 200 and headers.get_content_type() == "text/vnd.turbo-stream.html", (
                target["name"], delete_path, status, stream[:150])
            assert b'action="remove"' in stream and f"reaction_{reaction_id}".encode() in stream
            status, _, body = request(target, path, bearer=True)
            assert status == 200 and reaction_id not in [item["id"] for item in json.loads(body)]
            status, _, _ = request(target, delete_path, "POST", b"", bearer=True)
            outcomes.append(status)
        return outcomes
    finally:
        status, _, _ = request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)
        assert status == 204


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-reaction-delete-forms-") as temporary:
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
                process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url, "account": str(credentials["account"]),
                          "token": credentials["token"], "email": "owner@example.com",
                          "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                          "opener": urllib.request.build_opener(
                              urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
                results[name] = check(target)
            assert results["fizzy"] == results["rizzy"], results
            print("Matched Fizzy card and comment reaction delete forms and Turbo removal")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
