#!/usr/bin/env python3
"""Verify editing a completed step submits Fizzy's hidden unchecked value."""

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


def check(target):
    login(target)
    account = target["account"]
    status, _, body = request(target, f"/{account}/boards", "POST",
                             {"board": {"name": f"Step edit {time.time_ns()}"}}, bearer=True)
    assert status == 201
    board_id = json.loads(body)["id"]
    try:
        status, _, body = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                 {"card": {"title": "Step edit"}}, bearer=True)
        assert status == 201
        number = json.loads(body)["number"]
        status, _, body = request(target, f"/{account}/cards/{number}/steps", "POST",
                                 {"step": {"content": "Already done", "completed": True}}, bearer=True)
        assert status == 201
        step_id = json.loads(body)["id"]
        step_path = f"/{account}/cards/{number}/steps/{step_id}"
        status, _, page = request(target, step_path + "/edit", accept="text/html")
        assert status == 200
        inputs = re.findall(rb'<input\b[^>]*name="step\[completed\]"[^>]*>', page)
        assert any(b'type="hidden"' in item and b'value="0"' in item for item in inputs), (
            target["name"], "hidden unchecked value", inputs)
        assert any(b'type="checkbox"' in item and b'disabled' in item for item in inputs), (
            target["name"], "disabled completed checkbox", inputs)
        token_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
        token = token_match.group(1).decode() if token_match else None
        data = urllib.parse.urlencode([
            ("_method", "patch"), ("step[completed]", "0"),
            ("step[content]", "Edited while done")]).encode()
        status, headers, stream = request(target, step_path, "POST", data,
                                           accept="text/vnd.turbo-stream.html", csrf=token)
        assert status == 200 and headers.get_content_type() == "text/vnd.turbo-stream.html", (
            target["name"], status, stream[:150])
        assert f"step_{step_id}".encode() in stream
        status, _, body = request(target, step_path, bearer=True)
        assert status == 200
        step = json.loads(body)
        assert step["content"] == "Edited while done" and step["completed"] is False, (
            target["name"], step)
        return {"edit": 200, "turbo": 200, "completed": False}
    finally:
        status, _, _ = request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)
        assert status == 204


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-step-edit-completed-") as temporary:
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
            print("Matched Fizzy completed-step edit form and resulting state")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
