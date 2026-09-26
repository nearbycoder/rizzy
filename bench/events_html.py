#!/usr/bin/env python3
"""Compare filtered browser timeline throughput with equal card/event fixtures."""
import argparse
import atexit
import concurrent.futures
import http.cookiejar
import json
import os
import re
import sqlite3
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import Counter

from compare import fixture, pct, request as api_request


def browser_cookie(target):
    jar = http.cookiejar.CookieJar()
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    headers = {"Accept": "application/json", "Content-Type": "application/json"}
    if target["name"] == "fizzy":
        headers["X-Forwarded-For"] = "198.51.100." + str(30 + time.time_ns() % 200)
    email = target["email"]
    payload = json.dumps({"email_address": email}).encode()
    request = urllib.request.Request(target["url"] + "/session", payload, headers=headers)
    with opener.open(request, timeout=15) as response:
        code = response.headers.get("X-Magic-Link-Code")
    if not code:
        with sqlite3.connect(target["db"]) as db:
            code = db.execute("SELECT m.code FROM magic_links m JOIN identities i ON i.id=m.identity_id WHERE i.email_address=? ORDER BY m.created_at DESC LIMIT 1", (email,)).fetchone()[0]
    payload = json.dumps({"code": code}).encode()
    request = urllib.request.Request(target["url"] + "/session/magic_link", payload, headers=headers)
    with opener.open(request, timeout=15) as response:
        assert response.status == 200
    test = urllib.request.Request(target["url"] + f"/{target['account']}/events")
    jar.add_cookie_header(test)
    cookie = test.get_header("Cookie")
    assert cookie, "Browser session cookie was not created"
    return cookie


def html_response(target, path, etag=None):
    headers = {"Accept": "text/html", "Cookie": target["cookie"]}
    if etag:
        headers["If-None-Match"] = etag
    request = urllib.request.Request(target["url"] + f"/{target['account']}{path}",
        headers=headers)
    try:
        response = urllib.request.urlopen(request, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


def html(target, path):
    status, _, body = html_response(target, path)
    assert status == 200, status
    return body


def run(target, path, concurrency, seconds, etag=None):
    deadline = time.monotonic() + seconds
    started = time.monotonic()
    samples, failures = [], Counter()
    lock = threading.Lock()

    def worker(_):
        local, errors = [], Counter()
        while time.monotonic() < deadline:
            beginning = time.perf_counter()
            try:
                status, _, _ = html_response(target, path, etag)
                assert status == (304 if etag else 200), status
                local.append(time.perf_counter() - beginning)
            except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as error:
                errors[str(getattr(error, "code", type(error).__name__))] += 1
        with lock:
            samples.extend(local)
            failures.update(errors)

    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
        list(pool.map(worker, range(concurrency)))
    elapsed = time.monotonic() - started
    return {"concurrency": concurrency, "seconds": round(elapsed, 2),
        "requests": len(samples), "errors": dict(failures),
        "rps": round(len(samples) / elapsed, 1),
        "p50_ms": pct(samples, .5), "p95_ms": pct(samples, .95),
        "p99_ms": pct(samples, .99)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--concurrency", default="1,8,32")
    parser.add_argument("--conditional", action="store_true", help="measure matching If-None-Match reads")
    args = parser.parse_args()
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        keys = [f"{name}_{field}" for field in ("URL", "ACCOUNT", "TOKEN", "EMAIL", "DB")]
        if any(not os.getenv(key) for key in keys):
            parser.error("Set " + ", ".join(keys))
        targets[name.lower()] = {"name": name.lower(), **{field.lower(): os.environ[f"{name}_{field}"] for field in ("URL", "ACCOUNT", "TOKEN", "EMAIL", "DB")}}
    results = {"fixture_cards": args.cards, "conditional": args.conditional,
               "sample_parity": {}, "runs": {}}
    paths = {}
    etags = {}
    created_boards = []

    def cleanup():
        for target, board in reversed(created_boards):
            try:
                api_request(target, f"/boards/{board}", "DELETE")
            except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError):
                pass

    atexit.register(cleanup)
    for name, target in targets.items():
        board, _ = fixture(target, args.cards,
                           lambda board_id: created_boards.append((target, board_id)))
        target["cookie"] = browser_cookie(target)
        path = "/events?board_ids%5B%5D=" + urllib.parse.quote(board)
        paths[name] = path
        sample_status, sample_headers, sample = html_response(target, path)
        assert sample_status == 200
        etags[name] = sample_headers.get("ETag")
        event_count = len(re.findall(rb'class="event event--', sample))
        assert event_count == min(args.cards, 100), (name, event_count)
        assert b"events__columns" in sample and b"events__time-block" in sample
        assert b"Benchmark card 0" in sample
        results["sample_parity"][name] = {"visible_events": event_count,
            "columns": sample.count(b'class="events__column"'), "html_bytes": len(sample),
            "etag": bool(etags[name])}
        for _ in range(10):
            html(target, path)
        if args.conditional:
            assert etags[name], (name, "missing ETag")
            status, _, body = html_response(target, path, etags[name])
            assert status == 304 and not body, (name, status, body[:100])
    assert results["sample_parity"]["fizzy"]["visible_events"] == results["sample_parity"]["rizzy"]["visible_events"]
    for concurrency in map(int, args.concurrency.split(",")):
        for name, target in targets.items():
            result = run(target, paths[name], concurrency, args.seconds,
                         etags[name] if args.conditional else None)
            results["runs"].setdefault(name, []).append(result)
            print(f"{name} c={concurrency}: {result}", file=sys.stderr, flush=True)
    for fizzy, rust in zip(results["runs"]["fizzy"], results["runs"]["rizzy"]):
        results.setdefault("comparison", []).append({"concurrency": fizzy["concurrency"],
            "rps_ratio_rizzy_over_fizzy": round(rust["rps"] / fizzy["rps"], 3),
            "p95_ratio_fizzy_over_rizzy": round(fizzy["p95_ms"] / rust["p95_ms"], 3)})
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
