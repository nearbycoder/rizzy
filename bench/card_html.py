#!/usr/bin/env python3
"""Compare signed-in card HTML reads after creating equal card and message fixtures."""
import argparse
import json
import os
import sys

from compare import fixture, request
from events_html import browser_cookie, html, run


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=2)
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
            board, number = fixture(target, args.cards)
            boards[name] = board
            for content in ("Review the idea", "Share the update"):
                request(target, f"/cards/{number}/steps", "POST", {"step": {"content": content}})
            request(target, f"/cards/{number}/comments", "POST",
                    {"comment": {"body": "Looks useful."}})
            request(target, f"/cards/{number}/taggings", "POST", {"tag_title": "benchmark-tag"})
            target["cookie"] = browser_cookie(target)
            path = f"/cards/{number}"
            paths[name] = path
            sample = html(target, path)
            assert b'card-perma' in sample and b'Benchmark card 0' in sample
            assert all(value in sample for value in (b'Review the idea', b'Share the update', b'Looks useful.', b'benchmark-tag'))
            results["sample_parity"][name] = {"steps": 2, "comment": True, "tag": True, "html_bytes": len(sample)}
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
