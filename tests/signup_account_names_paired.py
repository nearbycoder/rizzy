#!/usr/bin/env python3
"""Compare repeated account completion names for one signed-in identity."""

import http.cookiejar
import concurrent.futures
import os
import sqlite3
import subprocess
import sys
import tempfile
import threading
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def account_names(target):
    with sqlite3.connect(target["db"]) as db:
        return [row[0] for row in db.execute(
            "SELECT a.name FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id WHERE i.email_address=? "
            "ORDER BY a.external_account_id", (target["email"],)
        )]


def check(target):
    login(target)
    before = account_names(target)
    suffix = "Fizzy" if target["name"] == "fizzy" else "Rizzy"
    expected = [f"Newart's {suffix}", f"Newart's 2nd {suffix}",
                f"Newart's 3rd {suffix}", f"Other's {suffix}"]
    for full_name in ("Newart Userbaum", "Newart Userbaum", "Newart Userbaum",
                      "Other Userbaum"):
        status, _, body = request(target, "/signup/completion.json", "POST",
                                  {"signup": {"full_name": full_name}})
        assert status == 201 and not body, (target["name"], status, body[:200])
    actual = account_names(target)
    assert actual == before + expected, (target["name"], actual, expected)
    return [name.replace("Fizzy", "Rizzy") for name in actual[len(before):]]


def check_two_process_names(target, urls):
    barrier = threading.Barrier(8)

    def complete(index):
        peer = {"url": urls[index % len(urls)], "token": target["token"],
                "opener": urllib.request.build_opener(NoRedirect())}
        barrier.wait(timeout=10)
        status, _, body = request(peer, "/signup/completion.json", "POST",
                                  {"signup": {"full_name": "Concurrent Owner"}},
                                  bearer=True)
        return status, body

    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(complete, range(8)))
    assert all(status == 201 and not body for status, body in results), results
    names = [name for name in account_names(target) if name.startswith("Concurrent's ")]
    expected = ["Concurrent's Rizzy"] + [
        f"Concurrent's {number}th Rizzy" if number > 3
        else f"Concurrent's {number}{'nd' if number == 2 else 'rd'} Rizzy"
        for number in range(2, 9)
    ]
    assert names == expected, (names, expected)


def main():
    subprocess.run(["cargo", "build", "--release", "--locked", "--quiet"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-signup-names-") as temporary:
        processes = []
        try:
            targets = []
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env, credentials = prepare_source(directory, 2)
                else:
                    env = rust_env(directory)
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                    env["RIZZY_MULTI_TENANT"] = "1"
                    env, credentials = prepare_rust(directory, env)
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                targets.append({
                    "name": name, "url": url,
                    "account": str(credentials["account"]),
                    "token": credentials["token"],
                    "email": "owner@example.com",
                    "db": str(directory / ("db.sqlite3" if name == "fizzy"
                                           else "rizzy.sqlite3")),
                    "opener": urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                        NoRedirect()),
                    "extra_headers": {"X-Forwarded-For": "198.51.100.121"}
                    if name == "fizzy" else {},
                })
            left, right = (check(target) for target in targets)
            assert left == right, (left, right)
            second_directory = Path(temporary) / "rizzy-second-process"
            second_directory.mkdir()
            second_process, second_url = start_server(
                "rizzy", second_directory, env, sorted(os.sched_getaffinity(0))[:2])
            processes.append(second_process)
            check_two_process_names(targets[1], [targets[1]["url"], second_url])
            print("Signup names matched: paired first/second/third and eight concurrent across two Rust processes")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
