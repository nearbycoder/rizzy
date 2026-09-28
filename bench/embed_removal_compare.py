#!/usr/bin/env python3
"""Compare last-attachment JPEG embed removals with purge workers paused."""

import argparse
import hashlib
import json
import math
import os
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests"))

from avatar_purge_parity import old_blob_state, pending_jobs  # noqa: E402
from card_image_parity import IMAGE, call  # noqa: E402
from embed_parity import upload  # noqa: E402
from sustained_mixed import (RUST, prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def summary(samples):
    ordered = sorted(samples)
    return {"count": len(ordered), "median_ms": round(statistics.median(ordered), 2),
            "p95_ms": round(ordered[math.ceil(len(ordered) * .95) - 1], 2),
            "mean_ms": round(statistics.mean(ordered), 2)}


def checked(target, path, method="GET", payload=None, expected=200):
    status, _, raw = call(target, f"/{target['account']}{path}", method, payload)
    assert status == expected, (target["name"], path, status, raw[:180])
    return json.loads(raw) if raw else None


def prepare_requests(target, count):
    board = checked(target, "/boards", "POST",
                    {"board": {"name": "Embed removal benchmark"}}, 201)
    requests = []
    for index in range(count):
        blob = upload(target, IMAGE, "moon.jpg", "image/jpeg")
        markup = f'<p><action-text-attachment sgid="{blob["attachable_sgid"]}"></action-text-attachment></p>'
        card = checked(target, f"/boards/{board['id']}/cards", "POST",
                       {"card": {"title": f"Embed {index:03d}", "description": markup}}, 201)
        assert card["has_attachments"]
        assert old_blob_state(target, blob) == (1, 2, 1), target["name"]
        requests.append((card["number"], blob))
    assert pending_jobs(target) == 0, target["name"]
    return requests


def run(target, requests, warmup):
    samples, response_bytes, response_fields = [], [], None
    for index, (number, blob) in enumerate(requests):
        started = time.perf_counter_ns()
        status, _, raw = call(target, f"/{target['account']}/cards/{number}.json", "PATCH",
                              {"card": {"description": "Removed embed"}})
        elapsed = (time.perf_counter_ns() - started) / 1_000_000
        assert status == 200, (target["name"], status, raw[:180])
        card = json.loads(raw)
        assert card["description"] == "Removed embed" and not card["has_attachments"], card
        fields = sorted(card)
        assert response_fields is None or response_fields == fields, target["name"]
        response_fields = fields
        assert old_blob_state(target, blob) == (1, 2, 0), target["name"]
        if index >= warmup:
            samples.append(elapsed)
            response_bytes.append(len(raw))
    jobs = pending_jobs(target)
    assert jobs == len(requests), (target["name"], jobs, len(requests))
    return {"latency": summary(samples), "queued_originals": jobs,
            "retained_variants": 2 * len(requests),
            "response_bytes": {"min": min(response_bytes), "max": max(response_bytes)},
            "response_fields": response_fields}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--samples", type=int, default=12)
    parser.add_argument("--warmup", type=int, default=2)
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.samples < 2 or args.warmup < 1 or args.rounds < 1:
        parser.error("requires at least two samples, one warmup, and one round")
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    cpus = sorted(os.sched_getaffinity(0))[:2]
    report = {
        "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"],
                                                   cwd=ROOT / "upstream", text=True).strip(),
        "rizzy_revision": subprocess.check_output(["git", "rev-parse", "HEAD"],
                                                  cwd=ROOT, text=True).strip(),
        "rizzy_dirty": bool(subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"],
                                                  cwd=ROOT, text=True).strip()),
        "rizzy_binary_sha256": hashlib.sha256(RUST.read_bytes()).hexdigest(),
        "fixture": {"format": "JPEG", "bytes": len(IMAGE), "derived_variants": 2},
        "server_cpus": len(cpus), "source_workers": 2,
        "samples_per_app_per_round": args.samples, "warmup": args.warmup,
        "purge_workers_paused": True, "rounds": []}
    with tempfile.TemporaryDirectory(prefix="rizzy-embed-removal-benchmark-") as temp:
        for round_index in range(args.rounds):
            processes, targets, requests = [], {}, {}
            try:
                for name, prepare in (("fizzy", prepare_source), ("rizzy", prepare_rust)):
                    directory = Path(temp) / f"round-{round_index}" / name
                    directory.mkdir(parents=True)
                    env, seed = (prepare(directory, 2) if name == "fizzy" else
                                 prepare(directory, {**rust_env(directory),
                                                     "RIZZY_DISABLE_STORAGE_PURGE": "1"}))
                    process, url = start_server(name, directory, env, cpus)
                    processes.append(process)
                    target = {"name": name, "url": url,
                              "database": directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3"),
                              **seed}
                    if name == "fizzy":
                        target["queue_database"] = directory / "queue_db.sqlite3"
                    targets[name] = target
                    requests[name] = prepare_requests(target, args.samples + args.warmup)
                order = ("fizzy", "rizzy") if round_index % 2 == 0 else ("rizzy", "fizzy")
                round_result = {"order": order}
                for name in order:
                    round_result[name] = run(targets[name], requests[name], args.warmup)
                    print(f"round {round_index + 1} {name}: {round_result[name]}",
                          file=sys.stderr, flush=True)
                assert round_result["fizzy"]["queued_originals"] == round_result["rizzy"]["queued_originals"]
                report["rounds"].append(round_result)
            finally:
                for process in reversed(processes):
                    stop_server(process)
    output = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(output)
    print(output, end="")


if __name__ == "__main__":
    main()
