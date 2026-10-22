#!/usr/bin/env python3
"""Compare signed-in board HTML reads on fresh, CPU-pinned servers."""

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

import sustained_mixed as suite


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=1000)
    parser.add_argument("--seconds", type=float, default=2)
    parser.add_argument("--concurrency", default="8,32")
    parser.add_argument("--cpus", type=int, default=8)
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--conditional", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    original_cpus = os.sched_getaffinity(0)
    cpus = sorted(original_cpus)[: args.cpus]
    assert cpus and args.workers > 0 and args.cards > 0 and args.seconds > 0
    processes = []
    try:
        with tempfile.TemporaryDirectory(prefix="rizzy-board-html-fresh-") as temporary:
            env = os.environ.copy()
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                server_env, credentials = (
                    suite.prepare_source(directory, args.workers)
                    if name == "fizzy"
                    else suite.prepare_rust(directory)
                )
                server_env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                process, url = suite.start_server(name, directory, server_env, cpus)
                processes.append(process)
                prefix = name.upper()
                env[prefix + "_URL"] = url
                env[prefix + "_ACCOUNT"] = str(credentials["account"])
                env[prefix + "_TOKEN"] = credentials["token"]
                env[prefix + "_EMAIL"] = "owner@example.com"
                env[prefix + "_DB"] = str(
                    directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
                )
            os.sched_setaffinity(0, cpus)
            command = [
                sys.executable,
                str(suite.ROOT / "bench/board_html.py"),
                "--cards", str(args.cards),
                "--seconds", str(args.seconds),
                "--concurrency", args.concurrency,
            ]
            if args.conditional:
                command.append("--conditional")
            run = subprocess.run(
                command, cwd=suite.ROOT, env=env,
                stdout=subprocess.PIPE, text=True, check=True,
            )
            result = json.loads(run.stdout)
            result["environment"] = {
                "fizzy_revision": subprocess.check_output(
                    ["git", "rev-parse", "HEAD"], cwd=suite.SOURCE, text=True
                ).strip(),
                "rizzy_sha256": hashlib.sha256(suite.RUST.read_bytes()).hexdigest(),
                "shared_cpu_ids": cpus,
                "fizzy_workers": args.workers,
                "rizzy_processes": 1,
            }
            output = json.dumps(result, indent=2) + "\n"
            if args.output:
                args.output.write_text(output)
            else:
                print(output, end="")
    finally:
        os.sched_setaffinity(0, original_cpus)
        for process in reversed(processes):
            suite.stop_server(process)


if __name__ == "__main__":
    main()
