#!/usr/bin/env python3
"""Compare populated webhook delivery JSON reads on fresh paired SQLite apps."""

import argparse
import hashlib
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

from card_workflow import expect
from compare import run_rust, shape
from sustained_mixed import (ROOT, RUST, SOURCE, comparable_json,
                             first_difference, prepare_rust, prepare_source,
                             start_server, stop_server)

LOAD = ROOT / "target/release/rizzy_load"


def revision(path):
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=path,
                                   text=True).strip()


def dirty(path):
    return bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=path))


def counts(database):
    with sqlite3.connect(database) as db:
        return {table: db.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
                for table in ("cards", "events", "webhook_deliveries")}


def fixture(name, target, env, deliveries):
    board, _ = expect(target, "/boards", "POST", {"board": {
        "name": "Webhook delivery read"}}, 201)
    board_id = board["id"]
    webhook, _ = expect(target, f"/boards/{board_id}/webhooks", "POST", {
        "webhook": {"name": "Read fixture", "url": "https://example.com/hook",
                    "subscribed_actions": ["card_published"]}}, 201)
    webhook_id = webhook["id"]
    for index in range(deliveries):
        expect(target, f"/boards/{board_id}/cards", "POST", {
            "card": {"title": f"Delivery card {index:04d}"}}, 201)
    if name == "fizzy":
        script = ('Webhook.find(' + json.dumps(webhook_id) + ').board.events'
                  '.where(action: "card_published").find_each { |event| '
                  'Event::WebhookDispatchJob.perform_now(event) }')
        result = subprocess.run(["bin/rails", "runner", script], cwd=SOURCE,
                                env=env, capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError(f"Fizzy webhook dispatch failed: {result.stderr[-2000:]}")
    path = f"/boards/{board_id}/webhooks/{webhook_id}/deliveries?page=4"
    rows, headers = expect(target, path)
    assert len(rows) == 100 and int(headers["X-Total-Count"]) == deliveries, (
        name, len(rows), headers)
    assert all(item["state"] == "pending" and item["event"]["action"] ==
               "card_published" for item in rows), name
    for _ in range(3):
        expect(target, path)
    return path, {"body": comparable_json(rows, target), "shape": shape(rows),
                  "total": int(headers["X-Total-Count"]),
                  "bytes": len(json.dumps(rows, separators=(",", ":")).encode())}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--deliveries", type=int, default=200)
    parser.add_argument("--seconds", type=float, default=10)
    parser.add_argument("--concurrency", default="8,32,64")
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--server-cpus", type=int, default=4)
    parser.add_argument("--output")
    args = parser.parse_args()
    levels = [int(value) for value in args.concurrency.split(",")]
    if args.deliveries < 195 or min(args.seconds, args.rounds, args.server_cpus,
                                    *levels) <= 0:
        parser.error("need at least 195 deliveries and positive run settings")
    available = sorted(os.sched_getaffinity(0))
    if len(available) <= args.server_cpus:
        parser.error("leave at least one CPU for the load generator")
    cpus, generator_cpus = available[:args.server_cpus], available[args.server_cpus:]
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy",
                    "--bin", "rizzy_load"], cwd=ROOT, check=True)
    result = {"database": "SQLite", "source_revision": revision(SOURCE),
              "rizzy_revision": revision(ROOT), "source_dirty": dirty(SOURCE),
              "rizzy_dirty": dirty(ROOT),
              "rizzy_binary_sha256": hashlib.sha256(RUST.read_bytes()).hexdigest(),
              "load_binary_sha256": hashlib.sha256(LOAD.read_bytes()).hexdigest(),
              "fixture_deliveries": args.deliveries,
              "workload": "GET webhook deliveries JSON page 4 (100 rows)",
              "seconds_per_target_level": args.seconds, "concurrency": levels,
              "server_cpus": cpus, "source_puma_workers": args.server_cpus,
              "rizzy_processes": 1, "generator_cpus": generator_cpus,
              "rounds": []}
    original_affinity = os.sched_getaffinity(0)
    try:
        for number in range(1, args.rounds + 1):
            with tempfile.TemporaryDirectory(prefix="rizzy-webhook-read-") as temporary:
                processes = []
                targets, samples, paths, baselines = {}, {}, {}, {}
                try:
                    for name in ("fizzy", "rizzy"):
                        directory = Path(temporary) / name
                        directory.mkdir()
                        env, credentials = (prepare_source(directory, args.server_cpus)
                                            if name == "fizzy" else prepare_rust(directory))
                        process, url = start_server(name, directory, env, cpus)
                        processes.append(process)
                        target = {"url": url, "account": str(credentials["account"]),
                                  "token": credentials["token"], "pid": process.pid}
                        database = directory / ("db.sqlite3" if name == "fizzy"
                                                else "rizzy.sqlite3")
                        path, sample = fixture(name, target, env, args.deliveries)
                        targets[name], paths[name], samples[name] = target, path, sample
                        baselines[name] = counts(database)
                    if difference := first_difference(samples["fizzy"]["body"],
                                                      samples["rizzy"]["body"]):
                        raise AssertionError("delivery JSON: " + difference)
                    assert samples["fizzy"]["shape"] == samples["rizzy"]["shape"]
                    assert baselines == {name: {"cards": args.deliveries,
                                                 "events": args.deliveries,
                                                 "webhook_deliveries": args.deliveries}
                                         for name in targets}, baselines
                    round_result = {"number": number, "sample_parity": {
                        "shape": samples["fizzy"]["shape"],
                        "decoded_content": True,
                        "compact_json_bytes": {name: samples[name]["bytes"] for name in targets},
                        "total": args.deliveries}, "baseline_counts": baselines,
                        "runs": [], "comparable": True}
                    os.sched_setaffinity(0, generator_cpus)
                    for concurrency in levels:
                        order = ("fizzy", "rizzy") if number % 2 else ("rizzy", "fizzy")
                        for name in order:
                            sample = run_rust(targets[name], [paths[name]], concurrency,
                                              args.seconds, str(LOAD))
                            round_result["runs"].append({"target": name, **sample})
                            if sample["errors"]:
                                round_result["comparable"] = False
                            print(f"round {number} {name} c={concurrency}: "
                                  f"{sample['rps']} req/s, p95 {sample['p95_ms']} ms, "
                                  f"errors={sample['errors']}", file=sys.stderr, flush=True)
                    os.sched_setaffinity(0, original_affinity)
                    after = {name: counts(Path(temporary) / name /
                            ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3"))
                             for name in targets}
                    round_result["final_counts"] = after
                    if after != baselines:
                        round_result["comparable"] = False
                    result["rounds"].append(round_result)
                finally:
                    os.sched_setaffinity(0, original_affinity)
                    for process in reversed(processes):
                        stop_server(process)
    finally:
        os.sched_setaffinity(0, original_affinity)
    result["status"] = ("complete" if all(item["comparable"] for item in result["rounds"])
                        else "has_errors")
    output = json.dumps(result, indent=2) + "\n"
    if args.output:
        destination = (ROOT / args.output).resolve()
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(output)
        print(destination)
    else:
        print(output)
    if result["status"] != "complete":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
