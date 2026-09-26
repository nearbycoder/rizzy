#!/usr/bin/env python3
"""Compare two Rizzy binaries on verified MySQL column deletions."""

import argparse
import hashlib
import json
import os
import statistics
import subprocess
import tempfile
import uuid
from pathlib import Path

import sustained_mixed
from mysql_column_delete import fixture, measure
from sustained_mixed_mysql import create_databases, drop_databases, mysql_admin, rust_env


def run_trial(args, size, trial, cpus):
    prefix = "rzdeleteab_" + uuid.uuid4().hex[:10]
    databases = {"before": prefix + "_b", "after": prefix + "_a"}
    processes = []
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-delete-ab-") as temp:
        root = Path(temp)
        try:
            create_databases(args.mysql_client, args.mysql_socket, list(databases.values()))
            targets = {}
            for name, binary in (("before", args.before_binary),
                                 ("after", args.after_binary)):
                directory = root / name
                directory.mkdir()
                sustained_mixed.RUST = binary
                env, credentials = sustained_mixed.prepare_rust(
                    directory, rust_env(directory, args.mysql_host,
                                        args.mysql_port, databases[name]))
                process, url = sustained_mixed.start_server("rizzy", directory, env, cpus)
                processes.append(process)
                targets[name] = {"name": "rizzy", "url": url,
                                 "account": str(credentials["account"]),
                                 "token": credentials["token"],
                                 "database": databases[name]}
            fixtures = {name: fixture(targets[name], size, trial,
                                      args.mysql_client, args.mysql_socket)
                        for name in ("before", "after")}
            order = ("before", "after") if trial % 2 == 0 else ("after", "before")
            return {name: measure(targets[name], fixtures[name],
                                  args.mysql_client, args.mysql_socket) for name in order}
        finally:
            for process in processes:
                sustained_mixed.stop_server(process)
            drop_databases(args.mysql_client, args.mysql_socket, list(databases.values()))


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--before-binary", type=Path, required=True)
    parser.add_argument("--after-binary", type=Path, required=True)
    parser.add_argument("--before-revision", required=True)
    parser.add_argument("--after-revision", required=True)
    parser.add_argument("--sizes", nargs="+", type=int, default=[1000, 10000])
    parser.add_argument("--trials", type=int, default=5)
    parser.add_argument("--mysql-client", default="/tmp/rizzy-mysql8/bin/mysql")
    parser.add_argument("--mysql-socket", default="/tmp/rizzy-mysql8/mysql.sock")
    parser.add_argument("--mysql-host", default="127.0.0.1")
    parser.add_argument("--mysql-port", type=int, default=33080)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.trials < 1 or min(args.sizes) < 3 or max(args.sizes) > 100000:
        parser.error("trials must be positive and card counts 3 to 100000")
    mysql_admin(args.mysql_client, args.mysql_socket, "SELECT 1")
    cpus = sorted(os.sched_getaffinity(0))[:2]
    result = {
        "database": "Oracle MySQL 8",
        "mysql_version": mysql_admin(args.mysql_client, args.mysql_socket,
                                     "SELECT VERSION()").strip(),
        "fixture": "Fresh database per binary/trial; SQL-cloned cards; timed API DELETE; all-card timestamps, activity, placement, and other-column state verified",
        "server_cpus": cpus, "rust_processes_per_binary": 1,
        "before": {"revision": args.before_revision,
                   "binary_sha256": digest(args.before_binary)},
        "after": {"revision": args.after_revision,
                  "binary_sha256": digest(args.after_binary)},
        "sizes": {},
    }
    for size in args.sizes:
        samples = {"before": [], "after": []}
        for trial in range(args.trials):
            times = run_trial(args, size, trial, cpus)
            for name, value in times.items():
                samples[name].append(value)
            print(size, trial, times, flush=True)
        result["sizes"][str(size)] = {
            "samples_ms": samples,
            "median_ms": {name: round(statistics.median(values), 3)
                          for name, values in samples.items()},
            "paired_after_before_ratios": [round(a / b, 3) for a, b in
                                           zip(samples["after"], samples["before"])],
        }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
