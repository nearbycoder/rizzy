#!/usr/bin/env python3
"""Parity-checked Fizzy/Rizzy mixed workload on one disposable MySQL server.

Requires a running local MySQL server and a socket-authenticated root account.
All benchmark schemas have unique rzbench_ names and are dropped at the end.
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

from sustained_mixed import (ROOT, RUST, SOURCE, RUBY, first_difference, fixture,
                             preflight, prepare_rust, prepare_source, run_load,
                             start_server, stop_server)


def mysql_admin(client, socket, statement):
    env = {**os.environ, "LD_LIBRARY_PATH": str(Path(client).resolve().parents[1] / "lib")}
    return subprocess.run([client, "--no-defaults", "--batch", "--skip-column-names",
                           f"--socket={socket}", "-uroot",
                           f"--execute={statement}"], env=env, check=True,
                          capture_output=True, text=True).stdout


def mysql_query(client, socket, database, statement):
    return mysql_admin(client, socket, f"USE `{database}`; {statement}").strip()


def create_databases(client, socket, names):
    for name in names:
        mysql_admin(client, socket,
                    f"CREATE DATABASE `{name}` CHARACTER SET utf8mb4 COLLATE utf8mb4_bin; "
                    f"GRANT ALL PRIVILEGES ON `{name}`.* TO 'rizzy'@'127.0.0.1';")


def drop_databases(client, socket, names):
    for name in names:
        mysql_admin(client, socket, f"DROP DATABASE IF EXISTS `{name}`")


def source_env(directory, workers, host, port, databases):
    env = {**os.environ,
           "PATH": str(RUBY / "bin") + ":" + os.environ.get("PATH", ""),
           "LD_LIBRARY_PATH": str(RUBY / "lib"),
           "BUNDLE_PATH": "/home/nearby/.local/rizzy-bundle",
           "RAILS_ENV": "production", "DATABASE_ADAPTER": "mysql",
           "MYSQL_HOST": host, "MYSQL_PORT": str(port),
           "MYSQL_USER": "rizzy", "MYSQL_PASSWORD": "rizzy",
           "DISABLE_SSL": "true", "SOLID_QUEUE_IN_PUMA": "false",
           "DISABLE_DATABASE_ENVIRONMENT_CHECK": "1",
           "SECRET_KEY_BASE": "0123456789abcdef" * 4,
           "WEB_CONCURRENCY": str(workers),
           "PIDFILE": str(directory / "puma.pid")}
    for role, database in databases.items():
        env[role + "DATABASE_URL"] = f"trilogy://rizzy:rizzy@{host}:{port}/{database}"
    return env


def rust_env(directory, host, port, database):
    return {**os.environ,
            "RIZZY_DATABASE_URL": f"mysql://rizzy:rizzy@{host}:{port}/{database}",
            "RIZZY_STORAGE_DIR": str(directory / "storage"),
            "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
            "RIZZY_DISABLE_PUSH_DELIVERY": "1"}


def verify_counts(client, socket, database, initial_cards, units):
    statement = ("SELECT (SELECT COUNT(*) FROM cards WHERE board_id=b.id), "
                 "(SELECT COUNT(*) FROM events WHERE board_id=b.id), "
                 "(SELECT COUNT(*) FROM comments WHERE card_id IN "
                 "(SELECT id FROM cards WHERE board_id=b.id)) "
                 "FROM boards b WHERE b.name='Mixed workload'")
    actual = [int(value) for value in mysql_query(client, socket, database,
                                                  statement).split("\t")]
    expected = [initial_cards + units, initial_cards + 4 * units, 3 * units]
    assert actual == expected, (database, actual, expected)
    return dict(zip(("cards", "events", "comments"), actual))


def run_pair(args, server_cpus, order):
    prefix = "rzbench_" + uuid.uuid4().hex[:12]
    source_dbs = {role: prefix + "_f" + suffix for role, suffix in
                  (("", ""), ("CABLE_", "_c"), ("CACHE_", "_k"), ("QUEUE_", "_q"))}
    rust_db = prefix + "_r"
    databases = [*source_dbs.values(), rust_db]
    processes = {}
    active_name = "setup"
    created = False
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-bench-") as temp:
        root = Path(temp)
        try:
            created = True
            create_databases(args.mysql_client, args.mysql_socket, databases)
            targets, paths, samples, boards, shapes = {}, {}, {}, {}, {}
            for name in ("fizzy", "rizzy"):
                active_name = name + " setup"
                directory = root / name
                directory.mkdir()
                env, credentials = (
                    prepare_source(directory, args.server_cpus,
                                   source_env(directory, args.server_cpus, args.mysql_host,
                                              args.mysql_port, source_dbs))
                    if name == "fizzy" else
                    prepare_rust(directory, rust_env(directory, args.mysql_host,
                                                     args.mysql_port, rust_db)))
                process, url = start_server(name, directory, env, server_cpus)
                processes[name] = process
                target = {"url": url, "account": str(credentials["account"]),
                          "token": credentials["token"], "pid": process.pid}
                targets[name] = target
                shapes[name] = preflight(target)
                boards[name], paths[name], samples[name] = fixture(target, args.cards)
            if difference := first_difference(shapes["fizzy"], shapes["rizzy"]):
                raise AssertionError("workflow shapes: " + difference)
            if difference := first_difference(samples["fizzy"], samples["rizzy"]):
                raise AssertionError("fixture: " + difference)
            result = {"sample_parity": samples["fizzy"], "runs": {}}
            for name in order:
                active_name = name + " load"
                result["runs"][name] = run_load(targets[name], boards[name], paths[name],
                                                 args.seconds, args.clients)
                assert not result["runs"][name]["errors"], (name, result["runs"][name]["errors"])
                database = source_dbs[""] if name == "fizzy" else rust_db
                result["runs"][name]["verified_counts"] = verify_counts(
                    args.mysql_client, args.mysql_socket, database, args.cards,
                    result["runs"][name]["units_completed"])
                print(f"{name}: {result['runs'][name]['units_per_second']} units/s, "
                      f"{len(result['runs'][name]['errors'])} errors",
                      file=sys.stderr, flush=True)
            return result
        except Exception:
            print(f"benchmark failed during {active_name}", file=sys.stderr)
            failure_dir = Path(tempfile.mkdtemp(prefix="rizzy-mysql-bench-failure-"))
            for name in processes:
                log = root / name / (name + ".log")
                if log.exists():
                    preserved = failure_dir / f"{name}.log"
                    preserved.write_bytes(log.read_bytes())
                    print(f"{name} full log: {preserved}", file=sys.stderr)
                    lines = log.read_text().splitlines()
                    errors = [line for line in lines if "ERROR" in line or "500 Internal" in line
                              or "Exception" in line]
                    print(f"{name} error lines: {errors[-20:]}", file=sys.stderr)
                    print(f"{name} log tail: {'\n'.join(lines[-25:])}", file=sys.stderr)
            raise
        finally:
            for process in processes.values():
                stop_server(process)
            if created:
                drop_databases(args.mysql_client, args.mysql_socket, databases)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, default=30)
    parser.add_argument("--clients", type=int, default=16)
    parser.add_argument("--cards", type=int, default=100)
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
    result = {"database": "MySQL", "source_revision": subprocess.check_output(
                  ["git", "rev-parse", "HEAD"], cwd=SOURCE, text=True).strip(),
              "source_dirty": bool(subprocess.check_output(
                  ["git", "status", "--porcelain"], cwd=SOURCE)),
              "rizzy_revision": subprocess.check_output(
                  ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "rizzy_dirty": bool(subprocess.check_output(
                  ["git", "status", "--porcelain"], cwd=ROOT)),
              "rizzy_binary_sha256": binary_sha256,
              "workload": "four JSON reads plus create/edit/comment/close/read per unit",
              "initial_cards": args.cards, "seconds_per_target": args.seconds,
              "clients": args.clients, "source_puma_workers": args.server_cpus,
              "server_cpus": server_cpus,
              "load_generator_cpus": sorted(os.sched_getaffinity(0)), "rounds": []}
    for number in range(args.rounds):
        order = ("fizzy", "rizzy") if number % 2 == 0 else ("rizzy", "fizzy")
        result["rounds"].append({"number": number + 1, "order": order,
                                 **run_pair(args, server_cpus, order)})
    output = json.dumps(result, indent=2) + "\n"
    if args.output:
        path = (ROOT / args.output).resolve()
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(output)
        print(path)
    else:
        print(output)


if __name__ == "__main__":
    main()
