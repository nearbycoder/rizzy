#!/usr/bin/env python3
"""Compare signed-in named-column HTML on fresh, CPU-pinned fixtures."""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

import sustained_mixed as suite
from compare import request
from events_html import browser_cookie, html, run


def fixture(target, cards):
    board = request(target, "/boards", "POST", {"board": {"name": "Named column benchmark"}})
    column = request(target, f"/boards/{board['id']}/columns", "POST",
                     {"column": {"name": "In progress"}})
    for index in range(cards):
        card = request(target, f"/boards/{board['id']}/cards", "POST",
                       {"card": {"title": f"Named column card {index:04d}"}})
        request(target, f"/cards/{card['number']}/triage", "POST",
                {"column_id": column["id"]})
    return board["id"], column["id"]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=2)
    parser.add_argument("--concurrency", default="8,32")
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--cpus", type=int, default=8)
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    original_cpus = os.sched_getaffinity(0)
    cpus = sorted(original_cpus)[:args.cpus]
    assert cpus and args.cards > 0 and args.seconds > 0 and args.rounds > 0
    assert args.workers > 0
    processes = []
    try:
        with tempfile.TemporaryDirectory(prefix="rizzy-named-column-html-") as temporary:
            targets = {}
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                server_env, credentials = (
                    suite.prepare_source(directory, args.workers)
                    if name == "fizzy" else suite.prepare_rust(directory)
                )
                server_env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                process, url = suite.start_server(name, directory, server_env, cpus)
                processes.append(process)
                targets[name] = {
                    "name": name, "url": url, "account": str(credentials["account"]),
                    "token": credentials["token"], "email": "owner@example.com",
                    "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                }
            results = {"fixture_cards": args.cards, "sample_parity": {}, "runs": []}
            paths = {}
            boards = {}
            for name, target in targets.items():
                board, column = fixture(target, args.cards)
                boards[name] = board
                target["cookie"] = browser_cookie(target)
                path = f"/boards/{board}/columns/{column}"
                paths[name] = path
                sample = html(target, path)
                visible = len(re.findall(rb'id="article_card_[^"]+"', sample))
                assert visible == min(args.cards, 15), (name, visible)
                assert b'cards--grid' in sample and b'Column: In progress' in sample
                assert f'Named column card {args.cards - 1:04d}'.encode() in sample
                results["sample_parity"][name] = {
                    "visible_cards": visible, "html_bytes": len(sample),
                }
                for _ in range(5):
                    html(target, path)
            os.sched_setaffinity(0, cpus)
            for round_index in range(args.rounds):
                order = ("fizzy", "rizzy") if round_index % 2 == 0 else ("rizzy", "fizzy")
                for concurrency in map(int, args.concurrency.split(",")):
                    for name in order:
                        result = run(targets[name], paths[name], concurrency, args.seconds)
                        assert not result["errors"], (name, result)
                        results["runs"].append({"round": round_index + 1, "name": name, **result})
                        print(f"round {round_index + 1} {name} c={concurrency}: {result}",
                              file=sys.stderr, flush=True)
            for name, board in boards.items():
                request(targets[name], f"/boards/{board}", "DELETE")
            results["environment"] = {
                "fizzy_revision": subprocess.check_output(
                    ["git", "rev-parse", "HEAD"], cwd=suite.SOURCE, text=True
                ).strip(),
                "rizzy_sha256": hashlib.sha256(suite.RUST.read_bytes()).hexdigest(),
                "shared_cpu_ids": cpus, "fizzy_workers": args.workers,
                "rizzy_processes": 1,
            }
            output = json.dumps(results, indent=2) + "\n"
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
