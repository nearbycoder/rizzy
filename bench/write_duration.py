#!/usr/bin/env python3
"""Fixed-duration card creation diagnostic with post-run count checks."""
import argparse
import concurrent.futures
import json
import os
import sys
import threading
import time
import urllib.error

from compare import request
from write import percentile


def run(target, concurrency, seconds):
    board = request(target, "/boards", "POST", {"board": {"name": f"Timed write benchmark {time.time_ns()}"}})
    board_id = board["id"]
    started = time.monotonic()
    deadline = started + seconds
    lock = threading.Lock()
    numbers, samples, errors = [], [], []

    def worker(worker_id):
        local_numbers, local_samples, local_errors = [], [], []
        index = worker_id
        while time.monotonic() < deadline:
            began = time.perf_counter()
            try:
                card = request(target, f"/boards/{board_id}/cards", "POST",
                               {"card": {"title": f"Timed write {index}", "description": "Equal write payload"}})
                local_numbers.append(card["number"])
                local_samples.append(time.perf_counter() - began)
            except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as error:
                local_errors.append(str(getattr(error, "code", type(error).__name__)))
            index += concurrency
        with lock:
            numbers.extend(local_numbers)
            samples.extend(local_samples)
            errors.extend(local_errors)

    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
        list(pool.map(worker, range(concurrency)))
    elapsed = time.monotonic() - started
    cards, card_headers = request(target, f"/cards?board_ids%5B%5D={board_id}", with_headers=True)
    activities, activity_headers = request(target, f"/activities?board_ids%5B%5D={board_id}", with_headers=True)
    card_count = int(card_headers["X-Total-Count"])
    activity_count = int(activity_headers["X-Total-Count"])
    assert len(numbers) == len(set(numbers)) == card_count == activity_count
    assert cards and activities and activities[0]["action"] == "card_published"
    return {"concurrency": concurrency, "target_seconds": seconds, "actual_seconds": round(elapsed, 2),
            "board_id": board_id, "created": len(numbers), "verified_cards": card_count,
            "verified_activity_events": activity_count, "errors": errors[:20],
            "rps": round(len(numbers) / elapsed, 1), "p50_ms": percentile(samples, .5),
            "p95_ms": percentile(samples, .95), "p99_ms": percentile(samples, .99)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--concurrency", default="32,64")
    args = parser.parse_args()
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        fields = ("URL", "ACCOUNT", "TOKEN")
        if all(os.getenv(f"{name}_{field}") for field in fields):
            targets[name.lower()] = {field.lower(): os.environ[f"{name}_{field}"] for field in fields}
    if not targets:
        parser.error("Set at least one complete FIZZY_* or RIZZY_* target")
    result = {"workload": "card create with post-run card and activity count checks", "runs": {}}
    for concurrency in map(int, args.concurrency.split(",")):
        for name, target in targets.items():
            sample = run(target, concurrency, args.seconds)
            result["runs"].setdefault(name, []).append(sample)
            print(f"{name} c={concurrency}: {sample}", file=sys.stderr, flush=True)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
