#!/usr/bin/env python3
"""Compare all-cards browser grid reads with equal 100-card board fixtures."""
import argparse
import json
import os
import re
import sys
import urllib.parse

from compare import fixture, request
from events_html import browser_cookie, html, run


def add_preview_details(target, board, count):
    cards = request(target, "/cards?board_ids%5B%5D=" + urllib.parse.quote(board))
    assert len(cards) == min(count, 15)
    column = request(target, f"/boards/{board}/columns", "POST",
                     {"column": {"name": "Working"}})
    for card in cards:
        number = card["number"]
        request(target, f"/cards/{number}/triage", "POST", {"column_id": column["id"]})
        request(target, f"/cards/{number}/taggings", "POST", {"tag_title": "#rich-preview"})
        step = request(target, f"/cards/{number}/steps", "POST",
                       {"step": {"content": "Check the preview"}})
        request(target, f"/cards/{number}/steps/{step['id']}", "PUT",
                {"step": {"completed": True}})
        request(target, f"/cards/{number}/comments", "POST",
                {"comment": {"body": "Checked"}})
        request(target, f"/cards/{number}/reactions", "POST",
                {"reaction": {"content": "👍"}})
        request(target, f"/cards/{number}/assignments", "POST",
                {"assignee_id": card["creator"]["id"]})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=3)
    parser.add_argument("--concurrency", default="1,8,32")
    parser.add_argument("--rich", action="store_true", help="Add column, tag, step, comment, boost, and assignee to each visible card")
    args = parser.parse_args()
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        keys = [f"{name}_{field}" for field in ("URL", "ACCOUNT", "TOKEN", "EMAIL", "DB")]
        if any(not os.getenv(key) for key in keys):
            parser.error("Set " + ", ".join(keys))
        targets[name.lower()] = {"name": name.lower(), **{
            field.lower(): os.environ[f"{name}_{field}"]
            for field in ("URL", "ACCOUNT", "TOKEN", "EMAIL", "DB")}}
    results = {"fixture_cards": args.cards, "rich_preview": args.rich, "sample_parity": {}, "runs": {}}
    boards = {}
    try:
        paths = {}
        for name, target in targets.items():
            board, _ = fixture(target, args.cards)
            boards[name] = board
            if args.rich:
                add_preview_details(target, board, args.cards)
            target["cookie"] = browser_cookie(target)
            path = "/cards?board_ids%5B%5D=" + urllib.parse.quote(board)
            paths[name] = path
            sample = html(target, path)
            preview_count = len(re.findall(rb'id="article_card_[^"]+"', sample))
            assert preview_count == min(args.cards, 15), (name, preview_count)
            assert b'cards_container' in sample and b'cards--grid' in sample
            assert f'Benchmark card {args.cards - 1}'.encode() in sample
            if args.rich:
                assert b'rich-preview' in sample and b'1/1' in sample
            results["sample_parity"][name] = {
                "visible_cards": preview_count, "html_bytes": len(sample)}
            for _ in range(5):
                html(target, path)
        assert results["sample_parity"]["fizzy"]["visible_cards"] == results["sample_parity"]["rizzy"]["visible_cards"]
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
