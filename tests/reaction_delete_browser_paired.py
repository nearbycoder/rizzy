#!/usr/bin/env python3
"""Delete card and comment reactions through the Fizzy and Rizzy browsers."""

import http.cookiejar
import json
import os
import subprocess
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


def browser(session, *args):
    result = subprocess.run(["agent-browser", "--session", session, *args],
                            capture_output=True, text=True)
    if result.returncode:
        raise AssertionError((session, args, result.stdout[-500:], result.stderr[-1000:]))
    return result.stdout.strip()


def wait_for(session, reaction_id, present):
    for _ in range(40):
        value = browser(session, "eval", f"!!document.getElementById('reaction_{reaction_id}')")
        if (value == "true") == present:
            return
        time.sleep(0.2)
    raise AssertionError((reaction_id, present, value, browser(session, "errors")))


def check(name, directory):
    if name == "fizzy":
        env, credentials = prepare_source(directory, 2)
    else:
        env = rust_env(directory)
        env["RIZZY_DEV_MAGIC_LINKS"] = "1"
        env, credentials = prepare_rust(directory, env)
    process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
    jar = http.cookiejar.CookieJar()
    target = {"name": name, "url": url, "account": str(credentials["account"]),
              "token": credentials["token"], "email": "owner@example.com",
              "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
              "opener": urllib.request.build_opener(
                  urllib.request.HTTPCookieProcessor(jar), NoRedirect())}
    session = f"rizzy-reaction-delete-{name}-{os.getpid()}"
    board_id = None
    try:
        login(target)
        account = target["account"]
        status, _, body = request(target, f"/{account}/boards", "POST",
                                 {"board": {"name": "Reaction browser"}}, bearer=True)
        assert status == 201
        board_id = json.loads(body)["id"]
        status, _, body = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                 {"card": {"title": "Reaction browser card"}}, bearer=True)
        assert status == 201
        number = json.loads(body)["number"]
        path = f"/{account}/cards/{number}"
        status, _, _ = request(target, path + "/publish", "POST", b"", bearer=True)
        assert status == 201
        status, _, body = request(target, path + "/comments", "POST",
                                 {"comment": {"body": "React here"}}, bearer=True)
        assert status == 201
        comment_id = json.loads(body)["id"]
        reaction_paths = [path + "/reactions", path + f"/comments/{comment_id}/reactions"]
        reaction_ids = []
        for reaction_path in reaction_paths:
            status, _, body = request(target, reaction_path, "POST",
                                     {"reaction": {"content": "👍"}}, bearer=True)
            assert status == 201
            reaction_ids.append(json.loads(body)["id"])
        browser(session, "open", url + "/session/new")
        for cookie in jar:
            browser(session, "cookies", "set", cookie.name, cookie.value,
                    "--url", url, "--path", cookie.path, "--httpOnly")
        browser(session, "open", url + path)
        browser(session, "set", "viewport", "1280", "900")
        browser(session, "wait", "--load", "networkidle")
        for reaction_id, reaction_path in zip(reaction_ids, reaction_paths):
            wait_for(session, reaction_id, True)
            browser(session, "eval", f"document.getElementById('reaction_{reaction_id}').scrollIntoView({{block:'center'}})")
            browser(session, "click", f'#reaction_{reaction_id} span[data-reaction-delete-target="content"]')
            browser(session, "click", f"#reaction_{reaction_id} .reaction__delete")
            wait_for(session, reaction_id, False)
            status, _, body = request(target, reaction_path, bearer=True)
            assert status == 200 and reaction_id not in [item["id"] for item in json.loads(body)]
        assert browser(session, "get", "url") == url + path
        assert not browser(session, "errors"), (name, "browser errors")
        return True
    finally:
        try:
            browser(session, "close")
        except AssertionError:
            pass
        if board_id:
            request(target, f"/{target['account']}/boards/{board_id}", "DELETE", bearer=True)
        stop_server(process)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-reaction-delete-browser-") as temporary:
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = Path(temporary) / name
            directory.mkdir()
            results[name] = check(name, directory)
        assert results["fizzy"] == results["rizzy"]
    print("PASS paired browser card and comment reaction deletion")


if __name__ == "__main__":
    main()
