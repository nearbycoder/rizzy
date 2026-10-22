#!/usr/bin/env python3
"""Recheck first-page board HTML throughput after adding dynamic CSP nonces."""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

from compare import fixture, request
from events_html import browser_cookie, html_response, run
from sustained_mixed import ROOT, RUST, SOURCE, prepare_rust, prepare_source, start_server, stop_server


def revision(path):
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=path, text=True).strip()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--concurrency", default="8,32")
    parser.add_argument("--rounds", type=int, default=2)
    args = parser.parse_args()
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    cpus = sorted(os.sched_getaffinity(0))[:4]
    assert len(cpus) == 4
    results = {
        "source_revision": revision(SOURCE),
        "rizzy_revision": revision(ROOT),
        "rizzy_binary_sha256": hashlib.sha256(Path(RUST).read_bytes()).hexdigest(),
        "server_cpus": cpus,
        "fizzy_workers": 2,
        "cards": args.cards,
        "seconds_per_run": args.seconds,
        "runs": [],
    }
    with tempfile.TemporaryDirectory(prefix="rizzy-board-csp-bench-") as temporary:
        processes = []
        targets = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                if name == "rizzy":
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                process, url = start_server(name, directory, env, cpus)
                processes.append(process)
                target = {
                    "name": name, "url": url,
                    "account": str(credentials["account"]),
                    "token": credentials["token"],
                    "email": "mixed-benchmark@example.com" if name == "fizzy" else "owner@example.com",
                    "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                }
                board, _ = fixture(target, args.cards)
                target["board"] = board
                target["cookie"] = browser_cookie(target)
                status, headers, body = html_response(target, f"/boards/{board}")
                assert status == 200 and headers["Content-Security-Policy"].count("'nonce-") == 1
                assert len(re.findall(rb'id="article_card_[^"]+"', body)) == min(args.cards, 15)
                assert f"Benchmark card {args.cards - 1}".encode() in body
                for _ in range(5):
                    warm_status, _, _ = html_response(target, f"/boards/{board}")
                    assert warm_status == 200
                target["html_bytes"] = len(body)
                targets[name] = target
            results["sample_html_bytes"] = {name: target["html_bytes"]
                                             for name, target in targets.items()}
            for round_index in range(args.rounds):
                names = ("fizzy", "rizzy") if round_index % 2 == 0 else ("rizzy", "fizzy")
                for concurrency in map(int, args.concurrency.split(",")):
                    for name in names:
                        target = targets[name]
                        result = run(target, f"/boards/{target['board']}", concurrency,
                                     args.seconds)
                        assert not result["errors"], (name, result)
                        results["runs"].append({"round": round_index + 1,
                                                "target": name, **result})
                        print(f"round {round_index + 1} {name} c={concurrency}: "
                              f"{result['rps']:.1f} req/s, p95 {result['p95_ms']:.1f} ms",
                              file=sys.stderr, flush=True)
        finally:
            for name, target in targets.items():
                request(target, f"/boards/{target['board']}", method="DELETE")
            for process in processes:
                stop_server(process)
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
