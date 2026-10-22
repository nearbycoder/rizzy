#!/usr/bin/env python3
"""Run the assignment browser contract on disposable Fizzy and Rizzy accounts."""

import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from assignment_html_parity import check, target  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-assignment-pair-") as temporary:
        processes = []
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
                prefix = name.upper()
                os.environ[f"{prefix}_URL"] = url
                os.environ[f"{prefix}_ACCOUNT"] = str(credentials["account"])
                os.environ[f"{prefix}_TOKEN"] = credentials["token"]
                os.environ[f"{prefix}_EMAIL"] = "owner@example.com"
                os.environ[f"{prefix}_DB"] = str(directory / ("db.sqlite3" if name == "fizzy"
                                                        else "rizzy.sqlite3"))
            source, rust = check(target("FIZZY")), check(target("RIZZY"))
            assert source == rust, (source, rust)
            print("PASS paired assignment picker and request contracts")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
