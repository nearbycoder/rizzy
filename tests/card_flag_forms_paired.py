#!/usr/bin/env python3
"""Exercise Golden Ticket and completion forms against fresh Fizzy and Rizzy."""

import http.cookiejar
import json
import os
import re
import sys
import tempfile
import time
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def csrf(page):
    match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
    return match.group(1).decode() if match else None


def form(page, path, delete=False):
    matches = re.findall(rb'<form\b[^>]*action="' + re.escape(path.encode()) + rb'"[^>]*>(.*?)</form>', page, re.S)
    assert any((b'name="_method" value="delete"' in body) == delete for body in matches), (path, delete, len(matches))


def mutate(target, path, token, delete=False, bearer=False):
    data = urllib.parse.urlencode({"_method": "delete"}).encode() if delete else b""
    accept = "application/json" if bearer else "text/vnd.turbo-stream.html"
    status, headers, body = request(target, path, "POST", data, accept=accept, bearer=bearer, csrf=token)
    assert status == (204 if bearer else 200), (target["name"], path, delete, status, body[:150])
    if not bearer:
        assert headers.get_content_type() == "text/vnd.turbo-stream.html"
        assert b'<turbo-stream method="morph" action="replace" target="card_container_card_' in body, (target["name"], path, body[:250])
        return sorted({re.sub(rb'_card_[a-z0-9]+$', b'_card_<id>', item)
                       for item in re.findall(rb'<turbo-stream\b[^>]*target="([^"]+)"', body)})
    return []


def check(target):
    login(target)
    account = target["account"]
    status, _, body = request(target, f"/{account}/boards", "POST", {"board": {"name": f"Flag parity {time.time_ns()}"}}, bearer=True)
    assert status == 201
    board_id = json.loads(body)["id"]
    try:
        status, _, body = request(target, f"/{account}/boards/{board_id}/cards", "POST", {"card": {"title": "Flag parity"}}, bearer=True)
        assert status == 201
        number = json.loads(body)["number"]
        card_path = f"/{account}/cards/{number}"
        gold_path, close_path = card_path + "/goldness", card_path + "/closure"
        status, _, page = request(target, card_path, accept="text/html")
        assert status == 200
        token = csrf(page)
        form(page, gold_path)
        form(page, close_path)
        gold_on = mutate(target, gold_path, token)
        status, _, page = request(target, card_path, accept="text/html")
        assert status == 200
        form(page, gold_path, delete=True)
        gold_off = mutate(target, gold_path, csrf(page), delete=True)
        status, _, page = request(target, card_path, accept="text/html")
        assert status == 200
        form(page, gold_path)
        closed = mutate(target, close_path, csrf(page))
        status, _, page = request(target, card_path, accept="text/html")
        assert status == 200
        form(page, close_path, delete=True)
        reopened = mutate(target, close_path, csrf(page), delete=True)
        status, _, page = request(target, card_path, accept="text/html")
        assert status == 200
        form(page, close_path)
        mutate(target, gold_path, None, bearer=True)
        status, _, _ = request(target, gold_path, "DELETE", bearer=True)
        assert status == 204
        mutate(target, close_path, None, bearer=True)
        status, _, _ = request(target, close_path, "DELETE", bearer=True)
        assert status == 204
        return {"gold_on": gold_on, "gold_off": gold_off, "closed": closed, "reopened": reopened}
    finally:
        status, _, _ = request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)
        assert status == 204


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-card-flag-forms-") as temporary:
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
            print("Matched Fizzy Golden Ticket and completion forms, Turbo targets, and JSON actions")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
