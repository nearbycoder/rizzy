#!/usr/bin/env python3
"""Measure card detail and list reads with equal image-embed fixtures."""
import argparse
import json
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tests"))
from embed_parity import normalized, upload
from compare import request, run, shape


def fixture(target, count):
    board = request(target, "/boards", "POST", {"board": {"name": f"Embed benchmark {time.time_ns()}"}})
    first = None
    for index in range(count):
        blob = upload(target)
        description = f'<p>Image <action-text-attachment sgid="{blob["attachable_sgid"]}"></action-text-attachment></p>'
        card = request(target, f"/boards/{board['id']}/cards", "POST",
                       {"card": {"title": f"Image card {index}", "description": description}})
        if first is None:
            first = (card["number"], blob)
    return board["id"], first


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=20)
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--concurrency", default="32,64")
    args = parser.parse_args()
    targets = {name.lower(): {field.lower(): os.environ[f"{name}_{field}"]
                              for field in ("URL", "ACCOUNT", "TOKEN")}
               for name in ("FIZZY", "RIZZY")}
    results = {"fixture_cards": args.cards, "fixture": "one moon.jpg embed per card", "runs": {}}
    boards, paths, samples = {}, {}, {}
    try:
        for name, target in targets.items():
            board, (number, blob) = fixture(target, args.cards)
            boards[name] = board
            paths[name] = [f"/cards/{number}", f"/cards?board_ids%5B%5D={board}"]
            card = request(target, paths[name][0])
            page, headers = request(target, paths[name][1], with_headers=True)
            assert len(page) == min(args.cards, 15)
            assert int(headers["X-Total-Count"]) == args.cards
            assert card["has_attachments"] and all(item["has_attachments"] for item in page)
            samples[name] = {"card_shape": shape(card), "description": card["description"],
                             "html": normalized(card["description_html"], target, blob),
                             "first_page_bytes": len(json.dumps(page))}
            for path in paths[name]:
                for _ in range(20):
                    request(target, path)
        left, right = samples["fizzy"], samples["rizzy"]
        assert left["card_shape"] == right["card_shape"]
        assert (left["description"], left["html"]) == (right["description"], right["html"])
        results["sample_parity"] = {"shape": True, "description": True, "html": True,
                                    "first_page_bytes": {name: sample["first_page_bytes"]
                                                         for name, sample in samples.items()}}
        for concurrency in map(int, args.concurrency.split(",")):
            for name, target in targets.items():
                outcome = run(target, paths[name], concurrency, args.seconds)
                results["runs"].setdefault(name, []).append(outcome)
                print(name, outcome, file=sys.stderr, flush=True)
        print(json.dumps(results, indent=2))
    finally:
        for name, board in boards.items():
            request(targets[name], f"/boards/{board}", "DELETE")


if __name__ == "__main__":
    main()
