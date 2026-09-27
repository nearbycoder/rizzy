#!/usr/bin/env python3
"""Measure the board-page cost of a partial named-column index on one fixture."""

import argparse
import hashlib
import json
import os
import re
import sqlite3
import statistics
import subprocess
import tempfile
from pathlib import Path

from column_delete import fixture
from events_html import browser_cookie, html_response, run
from sustained_mixed import ROOT, RUST, prepare_rust, start_server, stop_server


INDEX = "cards_board_column_active"
WIDE = "CREATE INDEX cards_board_column_active ON cards(board_id,column_id,status,closed,postponed,golden DESC,last_active_at DESC,id DESC)"
NONNULL = WIDE + " WHERE column_id IS NOT NULL"


def set_index(database, definition):
    with sqlite3.connect(database, timeout=30) as db:
        db.execute("DROP INDEX " + INDEX)
        db.execute(definition)
        db.commit()


def sample(target, path):
    status, _, body = html_response(target, path)
    assert status == 200, (status, path)
    assert len(re.findall(rb'id="article_card_[^"]+"', body)) == 15, path
    return len(body)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=30000)
    parser.add_argument("--seconds", type=float, default=2)
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--concurrency", default="1,8")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    assert args.cards >= 45 and args.rounds >= 2 and args.seconds > 0
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"], cwd=ROOT, check=True)
    cpus = sorted(os.sched_getaffinity(0))[:2]
    with tempfile.TemporaryDirectory(prefix="rizzy-column-index-board-") as temporary:
        directory = Path(temporary)
        env, credentials = prepare_rust(directory)
        env["RIZZY_DEV_MAGIC_LINKS"] = "1"
        process, url = start_server("rizzy", directory, env, cpus)
        try:
            target = {"name": "rizzy", "url": url, "account": str(credentials["account"]),
                      "token": credentials["token"], "db": str(directory / "rizzy.sqlite3"),
                      "database": directory / "rizzy.sqlite3", "email": "owner@example.com"}
            item = fixture(target, args.cards, 0)
            target["cookie"] = browser_cookie(target)
            paths = {"board": item["path"], "named_column": item["path"] + "/columns/" + item["delete_id"]}
            results = {"cards": args.cards, "cpus": cpus, "seconds": args.seconds,
                       "rounds": args.rounds, "binary_sha256": hashlib.sha256(Path(RUST).read_bytes()).hexdigest(),
                       "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                       "runs": []}
            original = sqlite3.connect(target["db"]).execute(
                "SELECT sql FROM sqlite_master WHERE type='index' AND name=?", (INDEX,)).fetchone()[0]
            assert "WHERE column_id IS NOT NULL" not in original
            for round_index in range(args.rounds):
                order = ("wide", "nonnull") if round_index % 2 == 0 else ("nonnull", "wide")
                for variant in order:
                    set_index(target["db"], WIDE if variant == "wide" else NONNULL)
                    for path in paths.values():
                        for _ in range(5):
                            sample(target, path)
                    for path_name, path in paths.items():
                        size = sample(target, path)
                        for concurrency in map(int, args.concurrency.split(",")):
                            measured = run(target, path, concurrency, args.seconds)
                            assert not measured["errors"], measured
                            results["runs"].append({"round": round_index + 1, "index": variant,
                                                    "page": path_name, "html_bytes": size, **measured})
            set_index(target["db"], WIDE)
            results["median"] = {}
            for variant in ("wide", "nonnull"):
                results["median"][variant] = {}
                for page in paths:
                    for concurrency in map(int, args.concurrency.split(",")):
                        group = [entry for entry in results["runs"] if entry["index"] == variant
                                 and entry["page"] == page and entry["concurrency"] == concurrency]
                        results["median"][variant][f"{page}_c{concurrency}"] = {
                            "rps": statistics.median(entry["rps"] for entry in group),
                            "p95_ms": statistics.median(entry["p95_ms"] for entry in group),
                        }
            output = json.dumps(results, indent=2) + "\n"
            if args.output:
                args.output.parent.mkdir(parents=True, exist_ok=True)
                args.output.write_text(output)
            print(output)
        finally:
            stop_server(process)


if __name__ == "__main__":
    main()
