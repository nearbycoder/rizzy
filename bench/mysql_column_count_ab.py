#!/usr/bin/env python3
"""Compare matched MySQL board/column read latency between two Rizzy builds."""

import argparse
import hashlib
import json
import os
import statistics
import tempfile
import time
import uuid
from pathlib import Path

import sustained_mixed
from card_workflow import expect
from mysql_column_delete import fixture
from sustained_mixed_mysql import create_databases, drop_databases, mysql_admin, rust_env


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read_samples(target, item, requests):
    board_id = item["board"]["id"]
    paths = {"named_cards": f"/boards/{board_id}/columns/{item['delete_column']}/cards",
             "maybe_board": f"/boards/{board_id}"}
    results = {}
    for label, path in paths.items():
        for _ in range(5):
            expect(target, path)
        samples = []
        for _ in range(requests):
            started = time.perf_counter()
            body, headers = expect(target, path)
            samples.append((time.perf_counter() - started) * 1000)
            assert headers.get("X-Total-Count") is not None, (label, headers)
            if label == "named_cards":
                assert isinstance(body, list) and len(body) == 15
            else:
                assert body["id"] == board_id
        results[label] = {"median_ms": round(statistics.median(samples), 3),
                          "samples_ms": [round(value, 3) for value in samples],
                          "total_count": int(headers["X-Total-Count"])}
    return results


def run_trial(args, trial, cpus):
    prefix = "rzcountab_" + uuid.uuid4().hex[:10]
    databases = {"before": prefix + "_b", "after": prefix + "_a"}
    processes = []
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-count-ab-") as temp:
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
            fixtures = {name: fixture(targets[name], args.cards, trial,
                                      args.mysql_client, args.mysql_socket)
                        for name in ("before", "after")}
            order = ("before", "after") if trial % 2 == 0 else ("after", "before")
            readings = {name: read_samples(targets[name], fixtures[name], args.requests)
                        for name in order}
            for label in ("named_cards", "maybe_board"):
                assert readings["before"][label]["total_count"] == readings["after"][label]["total_count"], readings
            return readings
        finally:
            for process in processes:
                sustained_mixed.stop_server(process)
            drop_databases(args.mysql_client, args.mysql_socket, list(databases.values()))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--before-binary", type=Path, required=True)
    parser.add_argument("--after-binary", type=Path, required=True)
    parser.add_argument("--before-revision", required=True)
    parser.add_argument("--after-revision", required=True)
    parser.add_argument("--cards", type=int, default=10000)
    parser.add_argument("--requests", type=int, default=40)
    parser.add_argument("--trials", type=int, default=3)
    parser.add_argument("--mysql-client", default="/tmp/rizzy-mysql8/bin/mysql")
    parser.add_argument("--mysql-socket", default="/tmp/rizzy-mysql8/mysql.sock")
    parser.add_argument("--mysql-host", default="127.0.0.1")
    parser.add_argument("--mysql-port", type=int, default=33080)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.cards < 3 or args.cards > 100000 or args.requests < 1 or args.trials < 1:
        parser.error("cards must be 3 to 100000; requests and trials must be positive")
    mysql_admin(args.mysql_client, args.mysql_socket, "SELECT 1")
    cpus = sorted(os.sched_getaffinity(0))[:2]
    result = {"database": "Oracle MySQL 8",
              "mysql_version": mysql_admin(args.mysql_client, args.mysql_socket,
                                           "SELECT VERSION()").strip(),
              "fixture": "Fresh database per binary/trial with identical API-created and SQL-cloned card counts; response totals and first-page shapes checked",
              "server_cpus": cpus, "cards": args.cards,
              "requests_per_route_per_trial": args.requests,
              "before": {"revision": args.before_revision,
                         "binary_sha256": digest(args.before_binary)},
              "after": {"revision": args.after_revision,
                        "binary_sha256": digest(args.after_binary)},
              "trials": []}
    for trial in range(args.trials):
        readings = run_trial(args, trial, cpus)
        result["trials"].append(readings)
        print(trial, {name: {label: value["median_ms"] for label, value in routes.items()}
                      for name, routes in readings.items()}, flush=True)
    result["median_of_trial_medians_ms"] = {
        label: {name: round(statistics.median(trial[name][label]["median_ms"]
                                              for trial in result["trials"]), 3)
                for name in ("before", "after")}
        for label in ("named_cards", "maybe_board")}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
