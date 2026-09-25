#!/usr/bin/env python3
"""Diagnostic card creation load. Run against both servers with FIZZY_* and RIZZY_* set."""
import argparse
import concurrent.futures
import json
import os
import sys
import statistics
import time
import urllib.error

from compare import request


def percentile(values, fraction):
    if not values:
        return None
    return round(sorted(values)[int((len(values)-1)*fraction)]*1000, 2)


def run(target, count, concurrency):
    board = request(target, "/boards", "POST", {"board": {"name": f"Write benchmark {time.time_ns()}"}})
    board_id = board["id"]
    started = time.monotonic()

    def create(i):
        t = time.perf_counter()
        try:
            result = request(target, f"/boards/{board_id}/cards", "POST", {"card": {"title": f"Write card {i}", "description": "Equal write payload"}})
            return result["number"], time.perf_counter()-t, None
        except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as e:
            return None, None, str(getattr(e, "code", type(e).__name__))

    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
        outcomes = list(pool.map(create, range(count)))
    elapsed = time.monotonic()-started
    durations = [duration for _, duration, error in outcomes if not error]
    errors = [error for _, _, error in outcomes if error]
    numbers = [number for number, _, error in outcomes if not error]
    return {"concurrency": concurrency, "count": count, "seconds": round(elapsed, 2),
            "created": len(numbers), "unique_numbers": len(set(numbers)), "errors": errors[:20],
            "rps": round(len(numbers)/elapsed, 1), "p50_ms": percentile(durations, .5),
            "p95_ms": percentile(durations, .95), "p99_ms": percentile(durations, .99)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=200)
    parser.add_argument("--concurrency", default="1,8,32")
    args = parser.parse_args()
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        fields = ("URL", "ACCOUNT", "TOKEN")
        missing = [field for field in fields if not os.getenv(f"{name}_{field}")]
        if len(missing) == len(fields):
            continue
        if missing:
            parser.error(f"Missing {name}: {', '.join(missing)}")
        targets[name.lower()] = {field.lower(): os.environ[f"{name}_{field}"] for field in fields}
    if not targets:
        parser.error("Set at least one complete FIZZY_* or RIZZY_* target")
    result = {"runs": {}}
    for concurrency in map(int, args.concurrency.split(",")):
        for name, target in targets.items():
            run_result = run(target, args.cards, concurrency)
            result["runs"].setdefault(name, []).append(run_result)
            print(f"{name} c={concurrency}: {run_result}", file=sys.stderr, flush=True)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
