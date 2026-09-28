#!/usr/bin/env python3
"""Compare browser and JSON timezone updates with pinned Fizzy."""

import http.cookiejar
import os
import sqlite3
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)

CASES = (
    ("PATCH", {"timezone_name": "America/Chicago"}, "application/json"),
    ("PUT", urllib.parse.urlencode({"timezone_name": "America/New_York"}).encode(),
     "application/json"),
    ("PATCH", urllib.parse.urlencode({"timezone_name": "Europe/London"}).encode(),
     "text/html"),
    ("PATCH", {"timezone_name": "Mars/Olympus"}, "application/json"),
    ("PATCH", {"timezone_name": 42}, "application/json"),
    ("PATCH", {"timezone_name": 1.5}, "application/json"),
    ("PATCH", {"timezone_name": True}, "application/json"),
    ("PATCH", {"timezone_name": False}, "application/json"),
    ("PATCH", {"timezone_name": []}, "application/json"),
    ("PATCH", {"timezone_name": {}}, "application/json"),
    ("PATCH", {"timezone_name": [1, "x", True, None, {"a": 1}]},
     "application/json"),
    ("PATCH", {"timezone_name": {"a": "x", "b": 2,
                                 "nested": {"flag": False}}}, "application/json"),
    ("PATCH", {"timezone_name": ""}, "application/json"),
    ("PATCH", {}, "application/json"),
    ("PATCH", None, "text/html"),
)


def timezone_state(target):
    table = "user_settings" if target["name"] == "fizzy" else "user_timezones"
    with sqlite3.connect(target["db"]) as database:
        row = database.execute(f"SELECT timezone_name FROM {table} LIMIT 1").fetchone()
    return row[0] if row else None


def check(target):
    login(target)
    path = f"/{target['account']}/my/timezone"
    results = []
    for method, data, accept in CASES:
        status, headers, body = request(target, path, method, data, accept=accept)
        results.append((status, headers.get("Content-Type"), body, timezone_state(target)))
    unusual_results = []
    for content_type, data, accept in (
        ("application/json", b'{"timezone_name":', "application/json"),
        ("application/json", b'{"timezone_name":', "text/html"),
        ("text/plain", b"timezone_name=UTC", "application/json"),
        ("application/json", b"true", "application/json"),
    ):
        call = urllib.request.Request(
            target["url"] + path, data=data, method="PATCH",
            headers={"Content-Type": content_type, "Accept": accept})
        try:
            response = target["opener"].open(call, timeout=10)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            unusual_results.append((response.status, response.headers.get("Content-Type"),
                                    response.read(), timezone_state(target)))
    anonymous = dict(target)
    anonymous["opener"] = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
    anonymous_results = []
    for accept in ("text/html", "application/json"):
        status, headers, body = request(
            anonymous, path, "PATCH", b"timezone_name=UTC", accept=accept)
        location = headers.get("Location")
        anonymous_results.append((status, headers.get("Content-Type"),
                                  urllib.parse.urlsplit(location).path if location else None,
                                  body))
    return results, unusual_results, anonymous_results


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-timezone-paired-") as temporary:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env, credentials = prepare_source(directory, 1)
                else:
                    env = rust_env(directory)
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                    env, credentials = prepare_rust(directory, env)
                process, url = start_server(
                    name, directory, env, sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                results[name] = check({
                    "name": name, "url": url, "account": str(credentials["account"]),
                    "email": "owner@example.com",
                    "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                    "opener": urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                        NoRedirect()),
                })
            assert results["fizzy"] == results["rizzy"], results
            assert all(result[0] == 204 and not result[2] for result in results["fizzy"][0])
            print("PASS 15 timezone updates, parsing errors, and anonymous responses match Fizzy")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
