#!/usr/bin/env python3
"""Compare populated JSON and browser GET contracts on disposable paired apps."""

import http.cookiejar
import os
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from draft_workflow_parity import NoRedirect, login  # noqa: E402
from route_get_valid_parity import run, run_html  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def compare(browser):
    with tempfile.TemporaryDirectory(prefix="rizzy-valid-get-") as temporary:
        processes = []
        targets = {}
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
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                targets[name] = {"name": name, "url": url,
                                 "account": str(credentials["account"]),
                                 "token": credentials["token"],
                                 "email": "owner@example.com",
                                 "db": str(directory / ("db.sqlite3" if name == "fizzy"
                                                        else "rizzy.sqlite3")),
                                 "opener": urllib.request.build_opener(
                                     urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                                     NoRedirect())}
                if browser:
                    login(targets[name])
            runner = run_html if browser else run
            source = runner(targets["fizzy"])
            rust = runner(targets["rizzy"])
            differences = {label: (value, rust.get(label)) for label, value in source.items()
                           if value != rust.get(label)}
            assert not differences, differences
            assert source.keys() == rust.keys(), (source.keys(), rust.keys())
            return len(source)
        finally:
            for process in reversed(processes):
                stop_server(process)


def main():
    json_count = compare(False)
    browser_count = compare(True)
    print(f"Matched pinned Fizzy on {json_count} populated JSON and {browser_count} browser GET contracts")


if __name__ == "__main__":
    main()
