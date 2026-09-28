#!/usr/bin/env python3
"""Run step read and edit parity against disposable Fizzy and Rizzy accounts."""

import http.cookiejar
import os
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from draft_workflow_parity import NoRedirect  # noqa: E402
from step_read_parity import run  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-step-read-paired-") as temporary:
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
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url,
                          "account": str(credentials["account"]),
                          "token": credentials["token"],
                          "email": "owner@example.com",
                          "db": str(directory / ("db.sqlite3" if name == "fizzy"
                                                 else "rizzy.sqlite3")),
                          "opener": urllib.request.build_opener(
                              urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                              NoRedirect())}
                results[name] = run(target)
            assert results["fizzy"] == results["rizzy"], results
            print("Step detail and edit reads match disposable Fizzy and Rizzy")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
