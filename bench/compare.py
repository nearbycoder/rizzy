#!/usr/bin/env python3
"""Build the same API fixture in Fizzy and Rizzy, then measure both HTTP servers.

Example:
  FIZZY_URL=http://127.0.0.1:3006 FIZZY_ACCOUNT=123 FIZZY_TOKEN=... \
  RIZZY_URL=http://127.0.0.1:3007 RIZZY_ACCOUNT=1 RIZZY_TOKEN=... \
  python3 bench/compare.py --cards 100 --seconds 10 --workload scoped
"""
import argparse
import atexit
import concurrent.futures
import json
import os
import sys
import threading
import time
import urllib.error
import urllib.request
from collections import Counter


def request(target, path, method="GET", payload=None, with_headers=False):
    body = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(
        f"{target['url']}/{target['account']}{path}", body, method=method,
        headers={"Authorization": f"Bearer {target['token']}",
                 "Accept": "application/json", "Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=15) as response:
        data = response.read()
        parsed = json.loads(data) if data else None
        return (parsed, response.headers) if with_headers else parsed


def shape(value):
    if isinstance(value, dict):
        return {key: shape(item) for key, item in value.items()}
    if isinstance(value, list):
        return [shape(value[0])] if value else []
    return type(value).__name__


def fixture(target, count, on_board=None):
    board = request(target, "/boards", "POST", {"board": {"name": f"Rizzy benchmark {int(time.time())}"}})
    board_id = board["id"]
    if on_board:
        on_board(board_id)
    card_number = None
    for n in range(count):
        card = request(target, f"/boards/{board_id}/cards", "POST", {"card": {"title": f"Benchmark card {n}", "description": "The same workload on each application."}})
        if n == 0:
            card_number = card["number"]
    return board_id, card_number


def pct(samples, p):
    return round(sorted(samples)[min(len(samples)-1, int((len(samples)-1)*p))]*1000, 2) if samples else None


def run(target, paths, concurrency, seconds):
    deadline = time.monotonic() + seconds
    starts = time.monotonic()
    samples, failures = [], Counter()
    lock = threading.Lock()

    def worker(worker_id):
        local, errs = [], Counter()
        n = worker_id
        while time.monotonic() < deadline:
            t = time.perf_counter()
            try:
                request(target, paths[n % len(paths)])
                local.append(time.perf_counter()-t)
            except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as e:
                errs[str(getattr(e, "code", type(e).__name__))] += 1
            n += concurrency
        with lock:
            samples.extend(local)
            failures.update(errs)

    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
        list(pool.map(worker, range(concurrency)))
    elapsed = time.monotonic()-starts
    return {"concurrency": concurrency, "seconds": round(elapsed, 2),
            "requests": len(samples), "errors": dict(failures),
            "rps": round(len(samples)/elapsed, 1), "p50_ms": pct(samples, .50),
            "p95_ms": pct(samples, .95), "p99_ms": pct(samples, .99)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=10)
    parser.add_argument("--concurrency", default="1,8,32")
    parser.add_argument("--workload", choices=("mixed", "scoped"), default="mixed",
                        help="scoped excludes the account-wide board list")
    args = parser.parse_args()
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        keys = [f"{name}_{field}" for field in ("URL", "ACCOUNT", "TOKEN")]
        absent = [k for k in keys if not os.getenv(k)]
        if len(absent) == len(keys):
            continue
        if absent:
            parser.error("Missing " + ", ".join(absent))
        targets[name.lower()] = {field.lower(): os.environ[f"{name}_{field}"] for field in ("URL", "ACCOUNT", "TOKEN")}
    if not targets:
        parser.error("Set at least one complete FIZZY_* or RIZZY_* target")
    results = {"fixture_cards": args.cards, "workload": args.workload, "runs": {}}
    paths = {}
    samples = {}
    created_boards = []

    def cleanup():
        for target, board_id in reversed(created_boards):
            try:
                request(target, f"/boards/{board_id}", "DELETE")
            except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError):
                pass

    atexit.register(cleanup)
    for name, target in targets.items():
        board, number = fixture(target, args.cards, lambda board_id: created_boards.append((target, board_id)))
        filtered_cards = f"/cards?board_ids%5B%5D={board}"
        activities = f"/activities?board_ids%5B%5D={board}"
        paths[name] = [f"/boards/{board}", f"/cards/{number}", filtered_cards, activities]
        if args.workload == "mixed":
            paths[name].insert(0, "/boards")
        # Verify each app actually returns the seeded objects before timing it.
        board_sample = request(target, f"/boards/{board}")
        card_sample = request(target, f"/cards/{number}")
        page_sample, page_headers = request(target, filtered_cards, with_headers=True)
        activity_sample, activity_headers = request(target, activities, with_headers=True)
        assert board_sample["name"].startswith("Rizzy benchmark")
        assert card_sample["title"] == "Benchmark card 0"
        assert len(page_sample) == min(args.cards, 15), (name, "first page count", len(page_sample))
        assert int(page_headers["X-Total-Count"]) == args.cards, (name, "total count", page_headers["X-Total-Count"])
        assert len(activity_sample) == min(args.cards, 15), (name, "activity first page count", len(activity_sample))
        assert int(activity_headers["X-Total-Count"]) == args.cards, (name, "activity total count", activity_headers["X-Total-Count"])
        samples[name] = {"board_shape": shape(board_sample), "card_shape": shape(card_sample),
                         "activity_shape":shape(activity_sample),
                         "card_description_html": card_sample["description_html"],
                         "first_page_count": len(page_sample), "first_page_bytes": len(json.dumps(page_sample))}
        for path in paths[name]:
            for _ in range(20):
                request(target, path)
    if len(targets) == 2:
        left, right = samples["fizzy"], samples["rizzy"]
        results["sample_parity"] = {"board_shape": left["board_shape"] == right["board_shape"],
                                    "card_shape": left["card_shape"] == right["card_shape"],
                                    "activity_shape": left["activity_shape"] == right["activity_shape"],
                                    "description_html": left["card_description_html"] == right["card_description_html"],
                                    "first_page_count": left["first_page_count"] == right["first_page_count"],
                                    "first_page_bytes": {name: sample["first_page_bytes"] for name, sample in samples.items()}}
        assert all(value for key,value in results["sample_parity"].items() if key != "first_page_bytes"), results["sample_parity"]
    for concurrency in map(int, args.concurrency.split(",")):
        for name, target in targets.items():
            results["runs"].setdefault(name, []).append(run(target, paths[name], concurrency, args.seconds))
            print(f"{name} c={concurrency}: {results['runs'][name][-1]}", file=sys.stderr, flush=True)
    if len(targets) == 2:
        for fizzy, rizzy in zip(results["runs"]["fizzy"], results["runs"]["rizzy"]):
            results.setdefault("comparison", []).append({"concurrency": fizzy["concurrency"],
                "rps_ratio_rizzy_over_fizzy": round(rizzy["rps"]/fizzy["rps"], 3) if fizzy["rps"] else None,
                "p95_ratio_fizzy_over_rizzy": round(fizzy["p95_ms"]/rizzy["p95_ms"], 3) if rizzy["p95_ms"] else None})
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
