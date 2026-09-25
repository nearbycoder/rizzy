#!/usr/bin/env python3
"""Benchmark parity-checked term and closure card filters."""
import argparse
import json
import os
import sys
import time
from urllib.parse import urlencode

from compare import request, run, shape


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--concurrency", default="32,64")
    args = parser.parse_args()
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        if all(os.getenv(f"{name}_{field}") for field in ("URL", "ACCOUNT", "TOKEN")):
            targets[name.lower()] = {field.lower(): os.environ[f"{name}_{field}"] for field in ("URL", "ACCOUNT", "TOKEN")}
    if len(targets) != 2:
        parser.error("Set both FIZZY_* and RIZZY_* targets")
    term = "filterbench" + str(time.time_ns())
    closed_count = args.cards // 5
    paths, samples = {}, {}
    for name, target in targets.items():
        board = request(target, "/boards", "POST", {"board": {"name": "Filter benchmark " + term}})
        numbers, owner_id = [], None
        for index in range(args.cards):
            card = request(target, f"/boards/{board['id']}/cards", "POST", {"card": {"title": f"{term} card {index}", "description": "Equal filter workload"}})
            numbers.append(card["number"])
            owner_id = card["creator"]["id"]
        for number in numbers[:closed_count]:
            request(target, f"/cards/{number}/closure", "POST")
        term_query = urlencode({"board_ids[]": [board["id"]], "terms[]": [term]}, doseq=True)
        closed_query = urlencode({"board_ids[]": [board["id"]], "closure": "today", "closer_ids[]": [owner_id]}, doseq=True)
        paths[name] = ["/cards?" + term_query, "/cards?" + closed_query]
        samples[name] = []
        for path, expected in zip(paths[name], (args.cards - closed_count, closed_count)):
            result, headers = request(target, path, with_headers=True)
            assert int(headers["X-Total-Count"]) == expected, (name, path, headers["X-Total-Count"])
            assert len(result) == min(expected, 15), (name, path, len(result))
            samples[name].append({"shape": shape(result), "count": expected, "first_page_bytes": len(json.dumps(result))})
            for _ in range(20):
                request(target, path)
    equal = all(samples["fizzy"][i]["shape"] == samples["rizzy"][i]["shape"] and
                samples["fizzy"][i]["count"] == samples["rizzy"][i]["count"] for i in range(2))
    assert equal, samples
    output = {"fixture_cards": args.cards, "closed_cards": closed_count,
              "sample_shape_and_count_parity": equal, "samples": samples, "runs": {}}
    for concurrency in map(int, args.concurrency.split(",")):
        for name, target in targets.items():
            result = run(target, paths[name], concurrency, args.seconds)
            output["runs"].setdefault(name, []).append(result)
            print(f"{name} c={concurrency}: {result}", file=sys.stderr, flush=True)
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
