#!/usr/bin/env python3
"""Measure card reads with matching formatted Action Text descriptions."""
import argparse
import json
import os
import sys
import time

from compare import request, run


DESCRIPTION = '<p>A <strong>bold</strong> idea &amp; a <a href="https://example.com">link</a>.</p><ul><li>Plan</li><li>Ship</li></ul>'


def fixture(target, count):
    board = request(target, "/boards", "POST", {"board": {"name": f"Rich text benchmark {time.time_ns()}"}})
    first = None
    for index in range(count):
        card = request(target, f"/boards/{board['id']}/cards", "POST",
                       {"card": {"title": f"Formatted card {index}", "description": DESCRIPTION}})
        if first is None:
            first = card["number"]
    return board["id"], first


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--concurrency", default="32,64")
    args = parser.parse_args()
    targets = {name.lower(): {key.lower(): os.environ[f"{name}_{key}"] for key in ("URL", "ACCOUNT", "TOKEN")}
               for name in ("FIZZY", "RIZZY")}
    results = {"fixture_cards": args.cards, "description_input": DESCRIPTION, "runs": {}}
    paths, samples = {}, {}
    for name, target in targets.items():
        board, number = fixture(target, args.cards)
        paths[name] = [f"/cards/{number}", f"/cards?board_ids%5B%5D={board}"]
        card = request(target, paths[name][0])
        page, headers = request(target, paths[name][1], with_headers=True)
        assert len(page) == min(args.cards, 15)
        assert int(headers["X-Total-Count"]) == args.cards
        assert all(row["description"] == card["description"] and row["description_html"] == card["description_html"] for row in page)
        samples[name] = {"description": card["description"], "description_html": card["description_html"],
                         "first_page_bytes": len(json.dumps(page))}
        for path in paths[name]:
            for _ in range(20):
                request(target, path)
    assert samples["fizzy"]["description"] == samples["rizzy"]["description"]
    assert samples["fizzy"]["description_html"] == samples["rizzy"]["description_html"]
    results["sample_parity"] = samples
    for concurrency in map(int, args.concurrency.split(",")):
        for name, target in targets.items():
            outcome = run(target, paths[name], concurrency, args.seconds)
            results["runs"].setdefault(name, []).append(outcome)
            print(name, outcome, file=sys.stderr, flush=True)
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
