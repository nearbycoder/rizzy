#!/usr/bin/env python3
"""Sustained, parity-checked JSON reads and card workflows on fresh databases."""

import argparse
import concurrent.futures
import hashlib
import itertools
import json
import os
import re
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from pathlib import Path

from card_workflow import api, expect, preflight, workflow
from compare import pct, process_sample, shape


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "upstream"
RUST = ROOT / "target/release/rizzy"
RUBY = Path("/home/nearby/.local/rizzy-ruby")


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def source_env(directory, workers):
    env = {**os.environ,
           "PATH": str(RUBY / "bin") + ":" + os.environ.get("PATH", ""),
           "LD_LIBRARY_PATH": str(RUBY / "lib"),
           "BUNDLE_PATH": "/home/nearby/.local/rizzy-bundle",
           "RAILS_ENV": "production", "DATABASE_ADAPTER": "sqlite",
           "DISABLE_SSL": "true", "SOLID_QUEUE_IN_PUMA": "false",
           "DISABLE_DATABASE_ENVIRONMENT_CHECK": "1",
           "SECRET_KEY_BASE": "0123456789abcdef" * 4,
           "WEB_CONCURRENCY": str(workers),
           "PIDFILE": str(directory / "puma.pid")}
    for role in ("", "CABLE_", "CACHE_", "QUEUE_"):
        env[role + "DATABASE_URL"] = "sqlite3:" + str(directory / (role.lower() + "db.sqlite3"))
    return env


def rust_env(directory):
    return {**os.environ,
            "RIZZY_DATABASE_URL": f"sqlite://{directory / 'rizzy.sqlite3'}",
            "RIZZY_STORAGE_DIR": str(directory / "storage"),
            "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}


def prepare_source(directory, workers, env=None):
    env = env or source_env(directory, workers)
    prepared = subprocess.run(["bin/rails", "db:prepare"], cwd=SOURCE, env=env,
                              capture_output=True, text=True)
    if prepared.returncode:
        raise RuntimeError(f"Fizzy database setup failed: {prepared.stderr[-2000:]}")
    script = '''require "json"
      identity = Identity.create!(email_address: "owner@example.com")
      account = Account.create_with_owner(
        account: { external_account_id: 1, name: "Rizzy" },
        owner: { name: "Owner", identity: identity })
      token = identity.access_tokens.create!(description: "Benchmark", permission: "write")
      puts JSON.generate({ account: account.external_account_id, token: token.token })'''
    result = subprocess.run(["bin/rails", "runner", script], cwd=SOURCE, env=env,
                            capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"Fizzy account setup failed: {result.stderr[-2000:]}")
    return env, json.loads(result.stdout)


def prepare_rust(directory, env=None):
    env = env or rust_env(directory)
    seeded = subprocess.run([RUST, "seed"], cwd=ROOT, env=env,
                            capture_output=True, text=True)
    if seeded.returncode:
        raise RuntimeError(f"Rizzy database setup failed: {seeded.stderr[-2000:]}")
    seeded = seeded.stdout
    return env, {"account": re.search(r"Account: (\d+)", seeded).group(1),
                 "token": re.search(r"Bearer token: (\w+)", seeded).group(1)}


def start_server(name, directory, env, cpus):
    port = free_port()
    address = f"127.0.0.1:{port}"
    command = (["bin/rails", "server", "-b", "127.0.0.1", "-p", str(port)]
               if name == "fizzy" else [RUST])
    cwd = SOURCE if name == "fizzy" else ROOT
    local_env = {**env, "PORT": str(port)} if name == "fizzy" else {
        **env, "RIZZY_BIND": address}
    url = "http://" + address
    if name == "rizzy":
        local_env.setdefault("RIZZY_PASSKEY_ORIGIN", f"http://localhost:{port}")
    log_path = directory / f"{name}.log"
    with log_path.open("w") as log:
        process = subprocess.Popen(command, cwd=cwd, env=local_env,
                                   stdout=log, stderr=subprocess.STDOUT,
                                   start_new_session=True,
                                   preexec_fn=lambda: os.sched_setaffinity(0, cpus))
    for _ in range(200):
        if process.poll() is not None:
            raise RuntimeError(f"{name} exited: {log_path.read_text()[-2000:]}")
        try:
            with urllib.request.urlopen(url + "/up", timeout=.3):
                return process, url
        except OSError:
            time.sleep(.1)
    stop_server(process)
    raise RuntimeError(f"{name} did not start: {log_path.read_text()[-2000:]}")


def stop_server(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)


def fixture(target, cards, board_name="Mixed workload"):
    board, _ = expect(target, "/boards", "POST",
                      {"board": {"name": board_name}}, 201)
    board_id = board["id"]
    first = None
    for index in range(cards):
        card, _ = expect(target, f"/boards/{board_id}/cards", "POST",
                         {"card": {"title": f"Fixture card {index}",
                                   "description": "The same readable card body."}}, 201)
        first = first or card["number"]
    paths = [f"/boards/{board_id}", f"/cards/{first}",
             f"/cards?board_ids%5B%5D={board_id}",
             f"/activities?board_ids%5B%5D={board_id}"]
    samples = []
    for index, path in enumerate([*paths, f"/boards/{board_id}?page=2"]):
        status, value, headers = api(target, path)
        assert status == 200, (path, status)
        samples.append({"shape": shape(value),
                        "total": int(headers["X-Total-Count"]) if "X-Total-Count" in headers else None,
                        "has_next": "Link" in headers,
                        "content": (value["name"] if index in (0, 4) else
                                    value["title"] if index == 1 else
                                    [row["title"] for row in value] if index == 2 else
                                    [row["action"] for row in value])})
    return board_id, paths, samples


def first_difference(left, right, path="sample"):
    if type(left) is not type(right):
        return f"{path}: types {type(left).__name__} and {type(right).__name__}"
    if isinstance(left, dict):
        if left.keys() != right.keys():
            return f"{path}: key difference {sorted(left.keys() ^ right.keys())}"
        for key in left:
            if difference := first_difference(left[key], right[key], f"{path}.{key}"):
                return difference
    elif isinstance(left, list):
        if len(left) != len(right):
            return f"{path}: lengths {len(left)} and {len(right)}"
        for index, (a, b) in enumerate(zip(left, right)):
            if difference := first_difference(a, b, f"{path}[{index}]"):
                return difference
    elif left != right:
        return f"{path}: values {left!r} and {right!r}"
    return None


def comparable_json(value, target):
    """Keep response content while masking fixture-specific IDs, URLs, and times.

    IDs are numbered by first use, so repeated references must still agree within
    each response. Sorting object keys makes that numbering independent of the
    serializer's object field order.
    """
    identifiers = {}
    root = f"{target['url']}/{target['account']}"
    timestamp = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z")
    generated_id = re.compile(r"\b[0-9a-z]{25}\b")

    def replace_id(match):
        identifier = match.group()
        if identifier not in identifiers:
            identifiers[identifier] = f"$ID{len(identifiers)}"
        return identifiers[identifier]

    def normalize(item):
        if isinstance(item, dict):
            return {key: normalize(item[key]) for key in sorted(item)}
        if isinstance(item, list):
            return [normalize(entry) for entry in item]
        if isinstance(item, str):
            if timestamp.fullmatch(item):
                return "$TIME"
            return generated_id.sub(replace_id, item.replace(root, "$ROOT"))
        return item

    return normalize(value)


def read_four(target, paths):
    timings = []
    for path in paths:
        started = time.perf_counter()
        status, value, _ = api(target, path)
        if status != 200:
            raise AssertionError((path, status))
        if path.startswith("/boards/") and value.get("id") is None:
            raise AssertionError("board JSON lacks id")
        if path.startswith("/cards/") and value.get("id") is None:
            raise AssertionError("card JSON lacks id")
        if path.startswith(("/cards?", "/activities?")) and not isinstance(value, list):
            raise AssertionError("collection JSON is not an array")
        timings.append(time.perf_counter() - started)
    return timings


def target_process_sample(target):
    samples = [process_sample(pid) for pid in target.get("pids", [target["pid"]])]
    return {key: sum(sample[key] for sample in samples) for key in samples[0]}


def run_load(target, board_id, paths, seconds, clients):
    before = target_process_sample(target)
    started = time.monotonic()
    deadline = started + seconds
    sequence = itertools.count(1)
    stop = threading.Event()

    def worker(worker_index):
        unit_times, read_times, workflow_times, errors = [], [], [], []
        urls = target.get("urls")
        worker_target = {**target, "url": urls[worker_index % len(urls)]} if urls else target
        boards = board_id if isinstance(board_id, list) else [board_id]
        # Keep each client's board fixed across targets even when they use a
        # different number of server processes.
        board_index = worker_index % len(boards)
        worker_board = boards[board_index]
        worker_paths = paths[board_index] if isinstance(board_id, list) else paths
        while time.monotonic() < deadline and not stop.is_set():
            beginning = time.perf_counter()
            try:
                reads = read_four(worker_target, worker_paths)
                result = workflow(worker_target, worker_board, next(sequence))
                read_times.extend(reads)
                workflow_times.append(sum(result["timings"].values()))
                unit_times.append(time.perf_counter() - beginning)
            except Exception as error:
                errors.append(f"{type(error).__name__}: {str(error)[:180]}")
                stop.set()
        return unit_times, read_times, workflow_times, errors, (
            worker_index % len(urls) if urls else 0), board_index

    with concurrent.futures.ThreadPoolExecutor(max_workers=clients) as pool:
        workers = list(pool.map(worker, range(clients)))
    elapsed = time.monotonic() - started
    after = target_process_sample(target)
    units = [value for worker in workers for value in worker[0]]
    reads = [value for worker in workers for value in worker[1]]
    workflows = [value for worker in workers for value in worker[2]]
    errors = [error for worker in workers for error in worker[3]]
    result = {"seconds_requested": seconds, "seconds_elapsed": round(elapsed, 2),
            "clients": clients, "units_completed": len(units),
            "units_per_second": round(len(units) / elapsed, 2),
            "read_requests": len(reads) + len(workflows),
            "write_requests": 4 * len(workflows),
            "p95_unit_ms": pct(units, .95), "p95_read_ms": pct(reads, .95),
            "p95_workflow_ms": pct(workflows, .95), "errors": errors[:10],
            "server_resources": {
                "processes_end": after["processes"],
                "cpu_seconds": round(after["cpu_seconds"] - before["cpu_seconds"], 2),
                "cpu_cores_average": round((after["cpu_seconds"] - before["cpu_seconds"]) / elapsed, 2),
                "pss_mib_start": round(before["pss_mib"], 1),
                "pss_mib_end": round(after["pss_mib"], 1),
                "write_mib": round(after["write_mib"] - before["write_mib"], 1)}}
    if target.get("urls"):
        result["units_by_process"] = [
            sum(len(worker[0]) for worker in workers if worker[4] == index)
            for index in range(len(target["urls"]))]
    if isinstance(board_id, list):
        result["client_board_indexes"] = [worker[5] for worker in workers]
        result["units_by_board"] = [
            sum(len(worker[0]) for worker in workers if worker[5] == index)
            for index in range(len(board_id))]
    return result


def verify_counts(database, board_id, initial_cards, units):
    with sqlite3.connect(database) as db:
        stored_board_id = db.execute(
            "SELECT id FROM boards WHERE name='Mixed workload'").fetchone()[0]
        actual = {
            "cards": db.execute("SELECT COUNT(*) FROM cards WHERE board_id=?", (stored_board_id,)).fetchone()[0],
            "events": db.execute("SELECT COUNT(*) FROM events WHERE board_id=?", (stored_board_id,)).fetchone()[0],
            "comments": db.execute("SELECT COUNT(*) FROM comments WHERE card_id IN "
                                   "(SELECT id FROM cards WHERE board_id=?)", (stored_board_id,)).fetchone()[0],
        }
        overall = {"boards": db.execute("SELECT COUNT(*) FROM boards").fetchone()[0],
                   "cards": db.execute("SELECT COUNT(*) FROM cards").fetchone()[0],
                   "events": db.execute("SELECT COUNT(*) FROM events").fetchone()[0]}
    expected = {"cards": initial_cards + units,
                "events": initial_cards + 4 * units,
                "comments": 3 * units}
    assert actual == expected, (str(database), board_id, actual, expected, overall)
    return actual


def run_pair(cards, seconds, clients, workers, server_cpus, order):
    with tempfile.TemporaryDirectory(prefix="rizzy-sustained-mixed-") as temp:
        root = Path(temp)
        targets, processes, paths, samples, boards, shapes = {}, {}, {}, {}, {}, {}
        databases = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = root / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, workers) if name == "fizzy"
                                    else prepare_rust(directory))
                process, url = start_server(name, directory, env, server_cpus)
                processes[name] = process
                target = {"url": url, "account": str(credentials["account"]),
                          "token": credentials["token"], "pid": process.pid}
                targets[name] = target
                shapes[name] = preflight(target)
                databases[name] = (directory / "db.sqlite3" if name == "fizzy"
                                   else directory / "rizzy.sqlite3")
                boards[name], paths[name], samples[name] = fixture(target, cards)
            if difference := first_difference(shapes["fizzy"], shapes["rizzy"]):
                raise AssertionError("workflow shapes: " + difference)
            if difference := first_difference(samples["fizzy"], samples["rizzy"]):
                raise AssertionError(difference)
            result = {"sample_parity": samples["fizzy"], "runs": {}}
            for name in order:
                result["runs"][name] = run_load(targets[name], boards[name],
                                                 paths[name], seconds, clients)
                assert not result["runs"][name]["errors"], result["runs"][name]["errors"]
                result["runs"][name]["verified_counts"] = verify_counts(
                    databases[name], boards[name], cards,
                    result["runs"][name]["units_completed"])
                print(f"{name} {clients} clients: {result['runs'][name]['units_per_second']} units/s, "
                      f"{len(result['runs'][name]['errors'])} errors", file=sys.stderr, flush=True)
            return result
        except Exception:
            for name, process in processes.items():
                print(f"{name} log tail: {(root / name / (name + '.log')).read_text()[-1500:]}",
                      file=sys.stderr)
            raise
        finally:
            for process in processes.values():
                stop_server(process)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, default=60)
    parser.add_argument("--clients", type=int, default=32)
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--server-cpus", type=int, default=8)
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
    result = {"source_revision": subprocess.check_output(
                  ["git", "rev-parse", "HEAD"], cwd=SOURCE, text=True).strip(),
              "rizzy_revision": subprocess.check_output(
                  ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "rizzy_binary_sha256": binary_sha256,
              "workload": "four JSON reads plus create/edit/comment/close/read per unit",
              "initial_cards": args.cards, "seconds_per_target": args.seconds,
              "clients": args.clients, "source_puma_workers": args.server_cpus,
              "server_cpus": server_cpus,
              "load_generator_cpus": sorted(os.sched_getaffinity(0)), "rounds": []}
    for number in range(args.rounds):
        order = ("fizzy", "rizzy") if number % 2 == 0 else ("rizzy", "fizzy")
        result["rounds"].append({"number": number + 1, "order": order,
                                 **run_pair(args.cards, args.seconds, args.clients,
                                            args.server_cpus, server_cpus, order)})
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
