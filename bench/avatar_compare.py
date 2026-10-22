#!/usr/bin/env python3
"""Measure anonymous avatar-to-thumbnail retrieval on the same PNG fixture."""
import argparse
import base64
import concurrent.futures
import hashlib
import json
import os
import sys
import threading
import time
import urllib.error
import urllib.request
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tests"))
from avatar_parity import PNG
from card_image_parity import call


def pct(samples, fraction):
    return round(sorted(samples)[min(len(samples) - 1, int((len(samples) - 1) * fraction))] * 1000, 2)


def fixture(target):
    prefix = f"/{target['account']}"
    status, _, body = call(target, prefix + "/users")
    assert status == 200
    user = next(row for row in json.loads(body) if row["role"] == "owner")
    blob = {"filename": "avatar.png", "byte_size": len(PNG),
            "checksum": base64.b64encode(hashlib.md5(PNG).digest()).decode(),
            "content_type": "image/png"}
    status, _, body = call(target, prefix + "/rails/active_storage/direct_uploads", "POST", {"blob": blob})
    assert status == 200, (status, body[:100])
    upload = json.loads(body)
    status, _, body = call(target, upload["direct_upload"]["url"], "PUT", PNG,
                           bearer=False, headers=upload["direct_upload"]["headers"])
    assert status == 204, (status, body[:100])
    status, _, body = call(target, prefix + f"/users/{user['id']}.json", "PATCH",
                           {"user": {"avatar": upload["signed_id"]}})
    assert status == 200, (status, body[:100])
    path = prefix + f"/users/{user['id']}/avatar"
    return path


def fetch(target, path):
    with urllib.request.urlopen(target["url"] + path, timeout=15) as response:
        data = response.read()
        assert response.status == 200 and response.headers.get_content_type() == "image/png"
        return data


def run(target, path, concurrency, seconds):
    deadline = time.monotonic() + seconds
    samples, failures = [], Counter()
    lock = threading.Lock()

    def worker(_):
        local, errors = [], Counter()
        while time.monotonic() < deadline:
            start = time.perf_counter()
            try:
                fetch(target, path)
                local.append(time.perf_counter() - start)
            except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, AssertionError) as error:
                errors[str(getattr(error, "code", type(error).__name__))] += 1
        with lock:
            samples.extend(local)
            failures.update(errors)

    start = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
        list(pool.map(worker, range(concurrency)))
    elapsed = time.monotonic() - start
    return {"concurrency": concurrency, "seconds": round(elapsed, 2), "requests": len(samples),
            "errors": dict(failures), "rps": round(len(samples) / elapsed, 1),
            "p50_ms": pct(samples, .5), "p95_ms": pct(samples, .95), "p99_ms": pct(samples, .99)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--concurrency", default="32,64")
    parser.add_argument("--order", choices=("fizzy,rizzy", "rizzy,fizzy"), default="fizzy,rizzy")
    args = parser.parse_args()
    order = args.order.split(",")
    targets = {name.lower(): {key.lower(): os.environ[f"{name}_{key}"] for key in ("URL", "ACCOUNT", "TOKEN")}
               for name in ("FIZZY", "RIZZY")}
    paths = {}
    results = {"fixture": "806x926 PNG resized to 256x256", "order": order, "runs": {}}
    try:
        for name in order:
            target = targets[name]
            paths[name] = fixture(target)
            thumbnail = fetch(target, paths[name])
            assert thumbnail.startswith(b"\x89PNG\r\n\x1a\n")
            assert (int.from_bytes(thumbnail[16:20], "big"), int.from_bytes(thumbnail[20:24], "big")) == (256, 256)
            results.setdefault("thumbnail_bytes", {})[name] = len(thumbnail)
            for _ in range(20):
                fetch(target, paths[name])
        for concurrency in map(int, args.concurrency.split(",")):
            for name in order:
                target = targets[name]
                outcome = run(target, paths[name], concurrency, args.seconds)
                results["runs"].setdefault(name, []).append(outcome)
                print(name, outcome, file=sys.stderr, flush=True)
    finally:
        for name, path in paths.items():
            status, _, body = call(targets[name], path + ".json", "DELETE")
            assert status == 204, (status, body[:100])
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
