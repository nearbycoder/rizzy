#!/usr/bin/env python3
"""Compare the signed-in first board HTML page with equal card fixtures."""
import argparse
import json
import os
import re
import sys

from compare import fixture, request
from events_html import browser_cookie, html, run


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=3)
    parser.add_argument("--concurrency", default="1,8,32,64,128")
    args = parser.parse_args()
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        keys = [f"{name}_{field}" for field in ("URL", "ACCOUNT", "TOKEN", "EMAIL", "DB")]
        if any(not os.getenv(key) for key in keys):
            parser.error("Set " + ", ".join(keys))
        targets[name.lower()] = {"name": name.lower(), **{
            field.lower(): os.environ[f"{name}_{field}"]
            for field in ("URL", "ACCOUNT", "TOKEN", "EMAIL", "DB")}}
    results = {"fixture_cards": args.cards, "sample_parity": {}, "runs": {}}
    boards = {}
    try:
        paths = {}
        for name, target in targets.items():
            board, _ = fixture(target, args.cards)
            boards[name] = board
            target["cookie"] = browser_cookie(target)
            path = f"/boards/{board}"
            paths[name] = path
            sample = html(target, path)
            visible = len(re.findall(rb'id="article_card_[^"]+"', sample))
            assert visible == min(args.cards, 15), (name, visible)
            assert b'cards--maybe' in sample and b'Add a card' in sample
            assert f'Benchmark card {args.cards - 1}'.encode() in sample
            results["sample_parity"][name] = {"visible_cards": visible, "html_bytes": len(sample)}
            for _ in range(5):
                html(target, path)
        for concurrency in map(int, args.concurrency.split(",")):
            for name, target in targets.items():
                result = run(target, paths[name], concurrency, args.seconds)
                results["runs"].setdefault(name, []).append(result)
                print(f"{name} c={concurrency}: {result}", file=sys.stderr, flush=True)
        for fizzy, rust in zip(results["runs"]["fizzy"], results["runs"]["rizzy"]):
            results.setdefault("comparison", []).append({
                "concurrency": fizzy["concurrency"],
                "rps_ratio_rizzy_over_fizzy": round(rust["rps"] / fizzy["rps"], 3),
                "p95_ratio_fizzy_over_rizzy": round(fizzy["p95_ms"] / rust["p95_ms"], 3)})
        print(json.dumps(results, indent=2))
    finally:
        for name, board in boards.items():
            request(targets[name], f"/boards/{board}", method="DELETE")


if __name__ == "__main__":
    main()
