#!/usr/bin/env python3
"""Compare parity-checked, account-scoped full-text search reads."""
import argparse
import json
import os
import sys
import time
from urllib.parse import quote

from compare import request, run, shape


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=int, default=5)
    parser.add_argument("--concurrency", default="32,64")
    parser.add_argument("--formatted", action="store_true", help="Index the query term inside formatted descriptions")
    args = parser.parse_args()
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        fields = ("URL", "ACCOUNT", "TOKEN")
        if all(os.getenv(f"{name}_{field}") for field in fields):
            targets[name.lower()] = {field.lower(): os.environ[f"{name}_{field}"] for field in fields}
    if not targets:
        parser.error("Set at least one complete FIZZY_* or RIZZY_* target")
    term = "searchprobe" + str(time.time_ns())
    hidden = "hiddenattribute" + str(time.time_ns())
    samples, paths = {}, {}
    for name, target in targets.items():
        board = request(target, "/boards", "POST", {"board": {"name": "Search benchmark " + term}})
        for i in range(args.cards):
            if args.formatted:
                card = {"title": f"Formatted search card {i}",
                        "description": f'<p class="{hidden}">A <strong>{term}</strong> in formatted text</p>'}
            else:
                card = {"title": f"{term} card {i}", "description": "Identical search fixture"}
            request(target, f"/boards/{board['id']}/cards", "POST", {"card": card})
        path = "/search?q=" + quote(term)
        result, headers = request(target, path, with_headers=True)
        assert len(result) == min(args.cards, 15), (name, len(result))
        assert int(headers["X-Total-Count"]) == args.cards, (name, headers["X-Total-Count"])
        if args.formatted:
            excluded, excluded_headers = request(target, "/search?q=" + quote(hidden), with_headers=True)
            assert excluded == [] and int(excluded_headers["X-Total-Count"]) == 0, name
        samples[name] = {"shape": shape(result), "description": result[0]["description"],
                         "description_html": result[0]["description_html"],
                         "first_page_bytes": len(json.dumps(result))}
        paths[name] = path
        for _ in range(20):
            request(target, path)
    if len(targets) == 2:
        for field in ("shape", "description", "description_html"):
            assert samples["fizzy"][field] == samples["rizzy"][field], (field, samples)
    output = {"fixture_cards": args.cards, "fixture": "formatted description" if args.formatted else "title",
              "sample_parity": {"shape": len(targets) == 2 and samples["fizzy"]["shape"] == samples["rizzy"]["shape"],
                                "description": len(targets) == 2 and samples["fizzy"]["description"] == samples["rizzy"]["description"],
                                "description_html": len(targets) == 2 and samples["fizzy"]["description_html"] == samples["rizzy"]["description_html"],
                                "first_page_bytes": {name: sample["first_page_bytes"] for name, sample in samples.items()}},
              "runs": {}}
    for concurrency in map(int, args.concurrency.split(",")):
        for name, target in targets.items():
            result = run(target, [paths[name]], concurrency, args.seconds)
            output["runs"].setdefault(name, []).append(result)
            print(f"{name} c={concurrency}: {result}", file=sys.stderr, flush=True)
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
