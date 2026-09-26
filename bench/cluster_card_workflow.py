#!/usr/bin/env python3
"""Measure card-workflow throughput with multiple Rust processes on one SQLite file."""
import argparse
import concurrent.futures
import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from card_workflow import STEPS, expect, preflight, workflow
from compare import pct, process_sample


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/release/rizzy"


def start(env, log_path):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    log = open(log_path, "w")
    process = subprocess.Popen(
        [BINARY], cwd=ROOT, env={**env, "RIZZY_BIND": f"127.0.0.1:{port}"},
        stdout=log, stderr=subprocess.STDOUT,
    )
    log.close()
    for _ in range(100):
        if process.poll() is not None:
            raise RuntimeError(f"Rizzy stopped at startup: {Path(log_path).read_text()[-2000:]}")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=.1):
                return port, process
        except OSError:
            time.sleep(.05)
    process.terminate()
    process.wait(timeout=5)
    raise RuntimeError(f"Rizzy did not listen: {Path(log_path).read_text()[-2000:]}")


def sample_resources(processes):
    samples = [process_sample(process.pid) for process in processes]
    return {key: sum(sample[key] for sample in samples)
            for key in ("cpu_seconds", "pss_mib", "write_mib")}


def run_one(process_count, concurrency, count, round_number):
    with tempfile.TemporaryDirectory(prefix="rizzy-cluster-bench-") as temp:
        database = Path(temp) / "benchmark.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}",
               "RIZZY_STORAGE_DIR": str(Path(temp) / "storage"),
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded.stdout).group(1)
        ports, processes = [], []
        try:
            for index in range(process_count):
                port, process = start(env, Path(temp) / f"server-{index}.log")
                ports.append(port)
                processes.append(process)
            targets = [{"url": f"http://127.0.0.1:{port}",
                        "account": account, "token": token} for port in ports]
            preflight(targets[0])
            board, _ = expect(targets[0], "/boards", "POST",
                              {"board": {"name": "Cluster benchmark"}}, 201)
            try:
                before = sample_resources(processes)
                started = time.monotonic()

                def do(index):
                    try:
                        return workflow(targets[index % process_count], board["id"], index), None
                    except Exception as error:
                        return None, f"{type(error).__name__}: {str(error)[:180]}"

                with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
                    outcomes = list(pool.map(do, range(count)))
                elapsed = time.monotonic() - started
                after = sample_resources(processes)
                errors = [error for _, error in outcomes if error]
                assert not errors, errors[:10]
                completed = [result for result, _ in outcomes]
                numbers = [result["number"] for result in completed]
                assert len(numbers) == len(set(numbers)) == count
                import sqlite3
                with sqlite3.connect(database) as db:
                    cards = db.execute("SELECT COUNT(*) FROM cards WHERE board_id=? AND closed=1",
                                       (board["id"],)).fetchone()[0]
                    events = db.execute("SELECT COUNT(*) FROM events WHERE board_id=?",
                                        (board["id"],)).fetchone()[0]
                    comments = db.execute("SELECT COUNT(*) FROM comments WHERE card_id IN "
                                          "(SELECT id FROM cards WHERE board_id=?)",
                                          (board["id"],)).fetchone()[0]
                assert (cards, events, comments) == (count, 4 * count, 3 * count), (cards, events, comments)
                durations = [sum(result["timings"].values()) for result in completed]
                return {
                    "round": round_number, "processes": process_count,
                    "concurrency": concurrency, "workflows": count,
                    "seconds": round(elapsed, 2),
                    "workflows_per_second": round(count / elapsed, 1),
                    "p50_workflow_ms": pct(durations, .5),
                    "p95_workflow_ms": pct(durations, .95),
                    "p95_steps_ms": {step: pct([item["timings"][step] for item in completed], .95)
                                     for step in STEPS},
                    "http_errors": 0, "verified_cards": cards,
                    "verified_events": events, "verified_comments": comments,
                    "server_resources": {
                        "cpu_seconds": round(after["cpu_seconds"] - before["cpu_seconds"], 2),
                        "pss_mib_after": round(after["pss_mib"], 1),
                        "write_mib": round(after["write_mib"] - before["write_mib"], 1),
                    },
                }
            finally:
                expect(targets[0], f"/boards/{board['id']}", "DELETE", status=204)
        except Exception:
            for index in range(len(processes)):
                log = Path(temp) / f"server-{index}.log"
                print(f"server {index}: {log.read_text()[-1500:]}", file=sys.stderr)
            raise
        finally:
            for process in processes:
                process.terminate()
            for process in processes:
                process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--workflows", type=int, default=1000)
    parser.add_argument("--concurrency", default="8,32")
    parser.add_argument("--processes", default="1,2,4")
    parser.add_argument("--rounds", type=int, default=2)
    args = parser.parse_args()
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    result = {"workload": "create, edit title and description, comment, close, read",
              "database": "fresh SQLite database per run", "runs": []}
    process_counts = [int(value) for value in args.processes.split(",")]
    concurrencies = [int(value) for value in args.concurrency.split(",")]
    for round_number in range(1, args.rounds + 1):
        for concurrency in concurrencies:
            order = process_counts if round_number % 2 else list(reversed(process_counts))
            for process_count in order:
                sample = run_one(process_count, concurrency, args.workflows, round_number)
                result["runs"].append(sample)
                print(f"round {round_number} p={process_count} c={concurrency}: "
                      f"{sample['workflows_per_second']} workflows/s, "
                      f"p95 {sample['p95_workflow_ms']} ms", file=sys.stderr, flush=True)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
