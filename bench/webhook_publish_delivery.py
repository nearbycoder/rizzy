#!/usr/bin/env python3
"""Compare card publication through completed webhook delivery on fresh SQLite apps."""

import argparse
import concurrent.futures
import hashlib
import json
import os
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from card_workflow import expect
from compare import pct, process_sample, shape
from sustained_mixed import (ROOT, RUST, SOURCE, prepare_rust, prepare_source,
                             rust_env, start_server, stop_server)


def revision(path):
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=path,
                                   text=True).strip()


def dirty(path):
    return bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=path))


def counts(database):
    with sqlite3.connect(database, timeout=10) as db:
        return {"cards": db.execute("SELECT COUNT(*) FROM cards").fetchone()[0],
                "events": db.execute("SELECT COUNT(*) FROM events").fetchone()[0],
                "deliveries": db.execute("SELECT COUNT(*) FROM webhook_deliveries").fetchone()[0],
                "completed": db.execute("SELECT COUNT(*) FROM webhook_deliveries "
                                        "WHERE state='completed'").fetchone()[0]}


def resources(processes):
    samples = [process_sample(process.pid) for process in processes]
    return {key: sum(sample[key] for sample in samples)
            for key in ("cpu_seconds", "pss_mib", "write_mib")}


def start_source_jobs(directory, env, cpus):
    log_path = directory / "jobs.log"
    with log_path.open("w") as log:
        process = subprocess.Popen(["bin/jobs", "start"], cwd=SOURCE,
                                   env={**env, "JOB_CONCURRENCY": "1"}, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True,
                                   preexec_fn=lambda: os.sched_setaffinity(0, cpus))
    return process, log_path


def stop_jobs(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)


def measure(name, target, database, processes, cards, clients, deadline_seconds):
    board, _ = expect(target, "/boards", "POST", {"board": {
        "name": "Webhook publish and deliver"}}, 201)
    webhook, _ = expect(target, f"/boards/{board['id']}/webhooks", "POST", {
        "webhook": {"name": "Blocked local destination",
                    "url": "http://127.0.0.1:1/hook",
                    "subscribed_actions": ["card_published"]}}, 201)
    path = f"/boards/{board['id']}/webhooks/{webhook['id']}/deliveries"
    assert counts(database) == {"cards": 0, "events": 0, "deliveries": 0,
                                "completed": 0}
    before = resources(processes)
    started = time.monotonic()

    def publish(index):
        beginning = time.perf_counter()
        try:
            card, _ = expect(target, f"/boards/{board['id']}/cards", "POST", {
                "card": {"title": f"Webhook write {index:06d}"}}, 201)
            assert card["title"] == f"Webhook write {index:06d}"
            return card["number"], time.perf_counter() - beginning, None
        except Exception as error:
            return None, None, f"{type(error).__name__}: {str(error)[:180]}"

    with concurrent.futures.ThreadPoolExecutor(max_workers=clients) as pool:
        outcomes = list(pool.map(publish, range(cards)))
    publish_elapsed = time.monotonic() - started
    errors = [error for _, _, error in outcomes if error]
    if errors:
        raise AssertionError((name, "card publication", errors[:10]))
    numbers = [number for number, _, _ in outcomes]
    assert len(numbers) == len(set(numbers)) == cards, (name, "card numbers")
    deadline = started + deadline_seconds
    while time.monotonic() < deadline:
        current = counts(database)
        if current["completed"] == cards:
            break
        for process in processes:
            if process.poll() is not None:
                raise RuntimeError(f"{name} worker exited during drain: {process.pid}")
        time.sleep(.1)
    else:
        raise AssertionError((name, "delivery drain timed out", counts(database)))
    total_elapsed = time.monotonic() - started
    after = resources(processes)
    final = counts(database)
    assert final == {"cards": cards, "events": cards, "deliveries": cards,
                     "completed": cards}, (name, final)
    rows, headers = expect(target, path)
    assert len(rows) == min(cards, 15) and int(headers["X-Total-Count"]) == cards
    assert all(row["state"] == "completed" and
               row["response"] == {"code": None, "error": "private_uri"} and
               row["event"]["action"] == "card_published" for row in rows), (name, rows[:2])
    current_webhook, _ = expect(target, f"/boards/{board['id']}/webhooks/{webhook['id']}")
    assert current_webhook["active"], (name, "webhook deactivated")
    durations = [duration for _, duration, _ in outcomes]
    return {"clients": clients, "cards": cards, "card_post_seconds": round(publish_elapsed, 2),
            "all_deliveries_seconds": round(total_elapsed, 2),
            "card_posts_per_second": round(cards / publish_elapsed, 2),
            "completed_deliveries_per_second": round(cards / total_elapsed, 2),
            "p50_card_post_ms": pct(durations, .5), "p95_card_post_ms": pct(durations, .95),
            "http_errors": errors, "unique_card_numbers": len(set(numbers)),
            "final_counts": final, "delivery_sample_shape": shape(rows),
            "server_resources": {"cpu_seconds": round(after["cpu_seconds"] -
                                                      before["cpu_seconds"], 2),
                                 "pss_mib_start": round(before["pss_mib"], 1),
                                 "pss_mib_end": round(after["pss_mib"], 1),
                                 "write_mib": round(after["write_mib"] -
                                                    before["write_mib"], 1)}}


def run_one(name, directory, args, cpus):
    directory.mkdir()
    workers = max(1, args.server_cpus - 1)
    env, credentials = (prepare_source(directory, workers) if name == "fizzy" else
                        prepare_rust(directory, {**rust_env(directory),
                                                 "RIZZY_DISABLE_WEBHOOK_DELIVERY": "0",
                                                 "RIZZY_DISABLE_WEBHOOK_CLEANUP": "1"}))
    server, url = start_server(name, directory, env, cpus)
    extra = None
    log_path = None
    try:
        if name == "fizzy":
            extra, log_path = start_source_jobs(directory, env, cpus)
        target = {"url": url, "account": str(credentials["account"]),
                  "token": credentials["token"]}
        database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
        return measure(name, target, database, [server, extra] if extra else [server],
                       args.cards, args.clients, args.deadline)
    except Exception:
        if log_path and log_path.exists():
            print(f"Fizzy jobs log: {log_path.read_text()[-2500:]}", file=sys.stderr)
        server_log = directory / f"{name}.log"
        if server_log.exists():
            print(f"{name} server log: {server_log.read_text()[-2500:]}", file=sys.stderr)
        raise
    finally:
        if extra:
            stop_jobs(extra)
        stop_server(server)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--clients", type=int, default=8)
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--server-cpus", type=int, default=4)
    parser.add_argument("--deadline", type=float, default=120)
    parser.add_argument("--output")
    args = parser.parse_args()
    if min(args.cards, args.clients, args.rounds, args.server_cpus, args.deadline) <= 0:
        parser.error("all workload settings must be positive")
    available = sorted(os.sched_getaffinity(0))
    if len(available) <= args.server_cpus:
        parser.error("leave at least one CPU for clients")
    cpus, clients_cpus = available[:args.server_cpus], available[args.server_cpus:]
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    result = {"database": "SQLite", "source_revision": revision(SOURCE),
              "rizzy_revision": revision(ROOT), "source_dirty": dirty(SOURCE),
              "rizzy_dirty": dirty(ROOT),
              "rizzy_binary_sha256": hashlib.sha256(RUST.read_bytes()).hexdigest(),
              "workload": "Concurrent card POST through completed blocked webhook delivery",
              "cards_per_target": args.cards, "clients": args.clients,
              "server_cpus": cpus, "source_puma_workers": max(1, args.server_cpus - 1),
              "source_job_processes": 1, "rizzy_processes": 1,
              "client_cpus": clients_cpus, "rounds": []}
    original_affinity = os.sched_getaffinity(0)
    try:
        os.sched_setaffinity(0, clients_cpus)
        for number in range(1, args.rounds + 1):
            with tempfile.TemporaryDirectory(prefix="rizzy-webhook-publish-") as temporary:
                order = ("fizzy", "rizzy") if number % 2 else ("rizzy", "fizzy")
                round_result = {"number": number, "order": order}
                for name in order:
                    sample = run_one(name, Path(temporary) / name, args, cpus)
                    round_result[name] = sample
                    print(f"round {number} {name}: {sample['card_posts_per_second']} "
                          f"card POST/s; {sample['completed_deliveries_per_second']} "
                          f"completed delivery/s; p95 POST {sample['p95_card_post_ms']} ms",
                          file=sys.stderr, flush=True)
                assert round_result["fizzy"]["delivery_sample_shape"] == round_result["rizzy"]["delivery_sample_shape"]
                result["rounds"].append(round_result)
    finally:
        os.sched_setaffinity(0, original_affinity)
    output = json.dumps(result, indent=2) + "\n"
    if args.output:
        destination = (ROOT / args.output).resolve()
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(output)
        print(destination)
    else:
        print(output)


if __name__ == "__main__":
    main()
