#!/usr/bin/env python3
"""Compare one and four Rizzy app processes on fresh MySQL databases."""

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

from sustained_mixed import (ROOT, RUST, first_difference, fixture, preflight,
                             prepare_rust, run_load, start_server, stop_server)
from sustained_mixed_mysql import (create_databases, drop_databases, observed_counts,
                                   rust_env, verify_counts)


def run_round(args, server_cpus, order):
    names = {processes: "rzbench_" + uuid.uuid4().hex[:12]
             for processes in (1, 4)}
    running, logs = {}, {}
    with tempfile.TemporaryDirectory(prefix="rizzy-cluster-mysql-") as temp:
        root = Path(temp)
        try:
            create_databases(args.mysql_client, args.mysql_socket, names.values())
            targets, boards, paths, samples, shapes = {}, {}, {}, {}, {}
            for count in (1, 4):
                directory = root / f"rizzy-{count}-1"
                directory.mkdir()
                env, credentials = prepare_rust(
                    directory, rust_env(directory, args.mysql_host,
                                        args.mysql_port, names[count]))
                urls, pids = [], []
                for worker in range(1, count + 1):
                    worker_dir = root / f"rizzy-{count}-{worker}"
                    worker_dir.mkdir(exist_ok=True)
                    process, url = start_server("rizzy", worker_dir, env, server_cpus)
                    running[(count, worker)] = process
                    logs[(count, worker)] = worker_dir / "rizzy.log"
                    urls.append(url)
                    pids.append(process.pid)
                target = {"url": urls[0], "urls": urls, "pids": pids,
                          "pid": pids[0], "account": str(credentials["account"]),
                          "token": credentials["token"]}
                targets[count] = target
                shapes[count] = preflight(target)
                boards[count], paths[count], samples[count] = fixture(target, args.cards)
            if difference := first_difference(shapes[1], shapes[4]):
                raise AssertionError("preflight: " + difference)
            if difference := first_difference(samples[1], samples[4]):
                raise AssertionError("fixture: " + difference)
            result = {"sample_parity": samples[1], "runs": {}, "paired_comparable": True}
            for count in order:
                sample = run_load(targets[count], boards[count], paths[count],
                                  args.seconds, args.clients)
                result["runs"][str(count)] = sample
                if sample["errors"]:
                    sample["observed_counts"] = observed_counts(
                        args.mysql_client, args.mysql_socket, names[count])
                    sample["valid_for_throughput_comparison"] = False
                    result["paired_comparable"] = False
                else:
                    try:
                        sample["verified_counts"] = verify_counts(
                            args.mysql_client, args.mysql_socket, names[count],
                            args.cards, sample["units_completed"])
                        sample["valid_for_throughput_comparison"] = True
                    except AssertionError as error:
                        sample["count_verification_error"] = str(error)
                        sample["valid_for_throughput_comparison"] = False
                        result["paired_comparable"] = False
                print(f"{count} process(es): {sample['units_per_second']} units/s, "
                      f"{len(sample['errors'])} errors, "
                      f"comparable={sample['valid_for_throughput_comparison']}",
                      file=sys.stderr, flush=True)
            if not result["paired_comparable"]:
                failure_dir = Path(tempfile.mkdtemp(prefix="rizzy-cluster-mysql-failure-"))
                result["diagnostics"] = {}
                for key, log in logs.items():
                    if log.exists():
                        preserved = failure_dir / f"rizzy-{key[0]}-{key[1]}.log"
                        preserved.write_bytes(log.read_bytes())
                        result["diagnostics"][f"{key[0]}-{key[1]}"] = str(preserved)
            return result
        finally:
            for process in running.values():
                stop_server(process)
            drop_databases(args.mysql_client, args.mysql_socket, names.values())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, default=20)
    parser.add_argument("--clients", type=int, default=32)
    parser.add_argument("--cards", type=int, default=500)
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--server-cpus", type=int, default=4)
    parser.add_argument("--mysql-client", default="/tmp/rizzy-mysql8/bin/mysql")
    parser.add_argument("--mysql-socket", default="/tmp/rizzy-mysql8/mysql.sock")
    parser.add_argument("--mysql-host", default="127.0.0.1")
    parser.add_argument("--mysql-port", type=int, default=33080)
    parser.add_argument("--output")
    args = parser.parse_args()
    if min(args.seconds, args.clients, args.cards, args.rounds, args.server_cpus) <= 0:
        parser.error("all numeric arguments must be positive")
    available = sorted(os.sched_getaffinity(0))
    if len(available) <= args.server_cpus:
        parser.error("leave at least one CPU for the load generator")
    server_cpus = available[:args.server_cpus]
    os.sched_setaffinity(0, available[args.server_cpus:])
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with RUST.open("rb") as binary:
        binary_sha256 = hashlib.file_digest(binary, "sha256").hexdigest()
    result = {"database": "MySQL", "rizzy_revision": subprocess.check_output(
                  ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "rizzy_dirty": bool(subprocess.check_output(
                  ["git", "status", "--porcelain"], cwd=ROOT)),
              "rizzy_binary_sha256": binary_sha256,
              "workload": "four JSON reads plus create/edit/comment/close/read per unit",
              "initial_cards": args.cards, "seconds_per_target": args.seconds,
              "clients": args.clients, "server_cpus": server_cpus,
              "load_generator_cpus": sorted(os.sched_getaffinity(0)), "rounds": []}
    for number in range(args.rounds):
        order = (1, 4) if number % 2 == 0 else (4, 1)
        result["rounds"].append({"number": number + 1, "order": order,
                                 **run_round(args, server_cpus, order)})
    result["status"] = ("complete" if all(r["paired_comparable"] for r in result["rounds"])
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
        sys.exit(1)


if __name__ == "__main__":
    main()
