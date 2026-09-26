#!/usr/bin/env python3
"""Compare parity-checked Fizzy and Rizzy JSON reads on Oracle MySQL."""

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

from compare import run_rust
from card_workflow import api
from sustained_mixed import (ROOT, RUST, SOURCE, comparable_json, first_difference, fixture,
                             preflight, prepare_rust, prepare_source, start_server,
                             stop_server)
from sustained_mixed_mysql import (create_databases, drop_databases, observed_counts,
                                   rust_env, source_env)

LOAD = ROOT / "target/release/rizzy_load"


def revision(path):
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=path,
                                   text=True).strip()


def dirty(path):
    return bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=path))


def run_pair(args, cpus, generator_cpus, levels, number):
    prefix = "rzread_" + uuid.uuid4().hex[:12]
    source_dbs = {role: prefix + "_f" + suffix for role, suffix in
                  (("", ""), ("CABLE_", "_c"), ("CACHE_", "_k"), ("QUEUE_", "_q"))}
    rust_db = prefix + "_r"
    databases = [*source_dbs.values(), rust_db]
    processes = {}
    logs = {}
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-read-") as temporary:
        root = Path(temporary)
        try:
            create_databases(args.mysql_client, args.mysql_socket, databases)
            targets, samples, paths = {}, {}, {}
            for name in ("fizzy", "rizzy"):
                directory = root / name
                directory.mkdir()
                env, credentials = (
                    prepare_source(directory, args.server_cpus,
                                   source_env(directory, args.server_cpus, args.mysql_host,
                                              args.mysql_port, source_dbs))
                    if name == "fizzy" else
                    prepare_rust(directory, rust_env(directory, args.mysql_host,
                                                     args.mysql_port, rust_db)))
                process, url = start_server(name, directory, env, cpus)
                processes[name] = process
                logs[name] = directory / f"{name}.log"
                target = {"url": url, "account": str(credentials["account"]),
                          "token": credentials["token"], "pid": process.pid}
                if name == "rizzy" and args.rizzy_processes > 1:
                    target["urls"] = [url]
                    target["pids"] = [process.pid]
                    for index in range(2, args.rizzy_processes + 1):
                        worker_directory = root / f"rizzy-{index}"
                        worker_directory.mkdir()
                        worker_process, worker_url = start_server(
                            "rizzy", worker_directory, env, cpus)
                        key = f"rizzy-{index}"
                        processes[key] = worker_process
                        logs[key] = worker_directory / "rizzy.log"
                        target["urls"].append(worker_url)
                        target["pids"].append(worker_process.pid)
                targets[name] = target
                preflight(target)
                _, paths[name], samples[name] = fixture(target, args.cards)
            if difference := first_difference(samples["fizzy"], samples["rizzy"]):
                raise AssertionError("JSON fixture: " + difference)
            content = {}
            for name in ("fizzy", "rizzy"):
                responses = [api(targets[name], path)[1] for path in paths[name]]
                content[name] = comparable_json(responses, targets[name])
            if difference := first_difference(content["fizzy"], content["rizzy"]):
                raise AssertionError("JSON content: " + difference)
            for url in targets["rizzy"].get("urls", [])[1:]:
                replica = {**targets["rizzy"], "url": url}
                responses = [api(replica, path)[1] for path in paths["rizzy"]]
                replica_content = comparable_json(responses, replica)
                if difference := first_difference(content["rizzy"], replica_content):
                    raise AssertionError("Rust replica JSON: " + difference)
            content_hash = hashlib.sha256(json.dumps(content["fizzy"], sort_keys=True)
                                          .encode()).hexdigest()
            expected = {"cards": args.cards, "events": args.cards, "comments": 0}
            baseline = {
                "fizzy": observed_counts(args.mysql_client, args.mysql_socket, source_dbs[""]),
                "rizzy": observed_counts(args.mysql_client, args.mysql_socket, rust_db),
            }
            assert baseline == {"fizzy": expected, "rizzy": expected}, baseline
            round_result = {"number": number, "order": [], "sample_parity": samples["fizzy"],
                            "canonical_json_sha256": content_hash,
                            "baseline_counts": baseline, "runs": [], "comparable": True}
            for concurrency in levels:
                order = (("fizzy", "rizzy") if number % 2 else ("rizzy", "fizzy"))
                round_result["order"].append({"concurrency": concurrency, "targets": order})
                for name in order:
                    sample = run_rust(targets[name], paths[name], concurrency,
                                      args.seconds, str(LOAD))
                    round_result["runs"].append({"target": name, **sample})
                    if sample["errors"]:
                        round_result["comparable"] = False
                    if name == "rizzy" and args.rizzy_processes > 1:
                        requests = sample["server_requests"]
                        if min(requests) == 0 or max(requests) - min(requests) > len(paths[name]):
                            round_result["comparable"] = False
                    print(f"round {number} {name} c={concurrency}: "
                          f"{sample['rps']} req/s, p95 {sample['p95_ms']} ms, "
                          f"errors={sample['errors']}", file=sys.stderr, flush=True)
            after = {
                "fizzy": observed_counts(args.mysql_client, args.mysql_socket, source_dbs[""]),
                "rizzy": observed_counts(args.mysql_client, args.mysql_socket, rust_db),
            }
            round_result["final_counts"] = after
            if after != baseline:
                round_result["comparable"] = False
            round_result["generator_cpus"] = generator_cpus
            return round_result
        except Exception:
            for name, path in logs.items():
                if path.exists():
                    print(f"{name} log tail: {path.read_text()[-1800:]}", file=sys.stderr)
            raise
        finally:
            for process in processes.values():
                stop_server(process)
            drop_databases(args.mysql_client, args.mysql_socket, databases)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=1000)
    parser.add_argument("--seconds", type=float, default=10)
    parser.add_argument("--concurrency", default="8,32,128")
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--server-cpus", type=int, default=4)
    parser.add_argument("--rizzy-processes", type=int, default=1)
    parser.add_argument("--mysql-client", default="/tmp/rizzy-mysql8/bin/mysql")
    parser.add_argument("--mysql-socket", default="/tmp/rizzy-mysql8/mysql.sock")
    parser.add_argument("--mysql-host", default="127.0.0.1")
    parser.add_argument("--mysql-port", type=int, default=33080)
    parser.add_argument("--output")
    args = parser.parse_args()
    levels = [int(value) for value in args.concurrency.split(",")]
    if min(args.cards, args.seconds, args.rounds, args.server_cpus,
           args.rizzy_processes, *levels) <= 0:
        parser.error("cards, seconds, rounds, CPU count, and concurrency must be positive")
    available = sorted(os.sched_getaffinity(0))
    if len(available) <= args.server_cpus:
        parser.error("leave at least one CPU for the load generator")
    cpus = available[:args.server_cpus]
    generator_cpus = available[args.server_cpus:]
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy",
                    "--bin", "rizzy_load"], cwd=ROOT, check=True)
    result = {"database": "Oracle MySQL", "source_revision": revision(SOURCE),
              "rizzy_revision": revision(ROOT), "source_dirty": dirty(SOURCE),
              "rizzy_dirty": dirty(ROOT), "rizzy_binary_sha256": hashlib.sha256(
                  RUST.read_bytes()).hexdigest(), "load_binary_sha256": hashlib.sha256(
                      LOAD.read_bytes()).hexdigest(), "initial_cards": args.cards,
              "workload": "Four JSON GET routes: board, card, scoped cards, scoped activities",
              "seconds_per_target_level": args.seconds, "concurrency": levels,
              "server_cpus": cpus, "source_puma_workers": args.server_cpus,
              "rizzy_processes": args.rizzy_processes,
              "generator_cpus": generator_cpus, "rounds": []}
    os.sched_setaffinity(0, generator_cpus)
    for number in range(1, args.rounds + 1):
        result["rounds"].append(run_pair(args, cpus, generator_cpus, levels, number))
    result["status"] = ("complete" if all(round_["comparable"] for round_ in result["rounds"])
                        else "has_errors")
    output = json.dumps(result, indent=2) + "\n"
    if args.output:
        path = (ROOT / args.output).resolve()
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(output)
        print(path)
    else:
        print(output)
    if result["status"] != "complete":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
