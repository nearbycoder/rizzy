#!/usr/bin/env python3
"""Paired MySQL board-move load with response and side-effect parity gates."""

import argparse
import concurrent.futures
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

from card_workflow import api, expect
from sustained_mixed import (ROOT, RUST, comparable_json, first_difference,
                             prepare_rust, prepare_source, start_server, stop_server)
from sustained_mixed_mysql import (create_databases, drop_databases, mysql_admin,
                                   mysql_query, rust_env, source_env)


def git_output(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def percentile(values, pct):
    if not values:
        return None
    ordered = sorted(values)
    return round(ordered[min(len(ordered) - 1, int((len(ordered) - 1) * pct))], 2)


def fixture(target, clients):
    boards = [expect(target, "/boards", "POST", {"board": {"name": f"Move bench {part}"}}, 201)[0]
              for part in ("source", "destination")]
    cards = []
    for index in range(clients):
        card, _ = expect(target, f"/boards/{boards[0]['id']}/cards", "POST",
                         {"card": {"title": f"Move bench card {index:03d}"}}, 201)
        cards.append(card)
    sample, _ = expect(target, f"/cards/{cards[0]['number']}/board", "PATCH",
                       {"board_id": boards[1]["id"]}, 200)
    returned, _ = expect(target, f"/cards/{cards[0]['number']}/board", "PATCH",
                         {"board_id": boards[0]["id"]}, 200)
    assert sample["board"]["id"] == boards[1]["id"]
    assert returned["board"]["id"] == boards[0]["id"]
    return {"boards": boards, "cards": cards, "sample": comparable_json(sample, target),
            "warmup_moves": 2}


def run_load(target, fixture_data, clients, seconds):
    boards = fixture_data["boards"]
    cards = fixture_data["cards"]
    deadline = time.monotonic() + seconds

    def worker(index):
        client_target = target.get("client_target", lambda _: target)(index)
        number = cards[index]["number"]
        successful = 0
        latencies = []
        errors = []
        while time.monotonic() < deadline:
            destination = boards[(successful + 1) % 2]["id"]
            started = time.monotonic()
            try:
                status, value, _ = api(client_target, f"/cards/{number}/board", "PATCH",
                                       {"board_id": destination})
                if status != 200 or value["board"]["id"] != destination:
                    raise AssertionError((status, value.get("board")))
                successful += 1
                latencies.append((time.monotonic() - started) * 1000)
            except Exception as error:
                errors.append(f"card {index}: {str(error)[:220]}")
                break
        return successful, latencies, errors

    started = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=clients) as pool:
        outcomes = list(pool.map(worker, range(clients)))
    elapsed = time.monotonic() - started
    counts = [outcome[0] for outcome in outcomes]
    latencies = [latency for _, values, _ in outcomes for latency in values]
    errors = [error for _, _, values in outcomes for error in values]
    result = {"moves": sum(counts), "moves_by_client": counts, "errors": errors[:20],
            "elapsed_seconds": round(elapsed, 3),
            "moves_per_second": round(sum(counts) / elapsed, 2),
            "p50_ms": percentile(latencies, .50), "p95_ms": percentile(latencies, .95),
            "p99_ms": percentile(latencies, .99)}
    if "urls" in target:
        result["moves_by_process"] = [sum(count for index, count in enumerate(counts)
                                          if index % len(target["urls"]) == process)
                                      for process in range(len(target["urls"]))]
    return result


def source_queue_state(client, socket, database):
    ready, claimed, failed = map(int, mysql_query(
        client, socket, database,
        "SELECT (SELECT COUNT(*) FROM solid_queue_ready_executions), "
        "(SELECT COUNT(*) FROM solid_queue_claimed_executions), "
        "(SELECT COUNT(*) FROM solid_queue_failed_executions)").split("\t"))
    return {"ready": ready, "claimed": claimed, "failed": failed}


def wait_source_queue(client, socket, database, timeout=20):
    started = time.monotonic()
    initial = source_queue_state(client, socket, database)
    state = initial
    while state["ready"] or state["claimed"]:
        if time.monotonic() - started >= timeout:
            break
        time.sleep(.2)
        state = source_queue_state(client, socket, database)
    return {"initial": initial, "final": state,
            "drain_seconds": round(time.monotonic() - started, 2),
            "drained": not state["ready"] and not state["claimed"]}


def source_failed_jobs(client, socket, database):
    rows = mysql_query(
        client, socket, database,
        "SELECT j.class_name,HEX(LEFT(f.error,800)) FROM solid_queue_failed_executions f "
        "JOIN solid_queue_jobs j ON j.id=f.job_id ORDER BY f.created_at DESC LIMIT 10")
    return [{"class": line.split("\t", 1)[0],
             "error": bytes.fromhex(line.split("\t", 1)[1]).decode(errors="replace")}
            for line in rows.splitlines() if "\t" in line]


def verify(client, socket, database, fixture_data, sample, source):
    cards = fixture_data["cards"]
    boards = fixture_data["boards"]
    expected = [boards[count % 2]["id"] for count in sample["moves_by_client"]]
    if source:
        expected = [f"{int(board_id, 36):032X}" for board_id in expected]
    numbers = ",".join(str(card["number"]) for card in cards)
    board_field = "HEX(board_id)" if source else "board_id"
    rows = mysql_query(client, socket, database,
                       f"SELECT number,{board_field} FROM cards WHERE number IN ({numbers}) ORDER BY number")
    actual = [line.split("\t")[1] for line in rows.splitlines()]
    event_count, comment_count = map(int, mysql_query(
        client, socket, database,
        "SELECT (SELECT COUNT(*) FROM events WHERE action='card_board_changed'), "
        "(SELECT COUNT(*) FROM comments)").split("\t"))
    total_moves = fixture_data["warmup_moves"] + sample["moves"]
    errors = []
    if actual != expected:
        errors.append("final card boards differ from completed move counts")
    if event_count != total_moves or comment_count != total_moves:
        errors.append(f"events/comments {event_count}/{comment_count} expected {total_moves}")
    sample["verified_counts"] = {"cards": len(actual), "move_events": event_count,
                                 "system_comments": comment_count}
    sample["valid"] = not sample["errors"] and not errors
    sample["verification_errors"] = errors


def run_pair(args, order, server_cpus, client_cpus):
    prefix = "rzmove_" + uuid.uuid4().hex[:12]
    source_dbs = {role: prefix + suffix for role, suffix in
                  (("", "_f"), ("CABLE_", "_fc"), ("CACHE_", "_fk"), ("QUEUE_", "_fq"))}
    rust_db = prefix + "_r"
    databases = [*source_dbs.values(), rust_db]
    processes = []
    with tempfile.TemporaryDirectory(prefix="rizzy-card-move-mysql-") as temp:
        try:
            create_databases(args.mysql_client, args.mysql_socket, databases)
            targets, fixtures, logs = {}, {}, {}
            for name in ("fizzy", "rizzy"):
                directory = Path(temp) / name
                directory.mkdir()
                if name == "fizzy":
                    env = source_env(directory, 3, args.mysql_host,
                                     args.mysql_port, source_dbs)
                    env["SOLID_QUEUE_IN_PUMA"] = "true"
                    env, credentials = prepare_source(directory, 3, env)
                else:
                    env = rust_env(directory, args.mysql_host, args.mysql_port, rust_db)
                    env, credentials = prepare_rust(directory, env)
                process, url = start_server(name, directory, env, server_cpus)
                processes.append(process)
                targets[name] = {"url": url, "account": str(credentials["account"]),
                                 "token": credentials["token"]}
                logs[name] = directory / f"{name}.log"
                if name == "rizzy":
                    targets[name]["urls"] = [url]
                    for index in range(1, 4):
                        worker_directory = Path(temp) / f"rizzy-{index}"
                        worker_directory.mkdir()
                        worker, worker_url = start_server("rizzy", worker_directory,
                                                          env, server_cpus)
                        processes.append(worker)
                        targets[name]["urls"].append(worker_url)
                        logs[f"rizzy-{index}"] = worker_directory / "rizzy.log"
                fixtures[name] = fixture(targets[name], args.clients)
            if difference := first_difference(fixtures["fizzy"]["sample"],
                                              fixtures["rizzy"]["sample"]):
                raise AssertionError("move response mismatch: " + difference)
            source_warmup_queue = wait_source_queue(
                args.mysql_client, args.mysql_socket, source_dbs["QUEUE_"])

            if client_cpus:
                os.sched_setaffinity(0, client_cpus)
            result = {"sample_parity": True, "source_warmup_queue": source_warmup_queue,
                      "order": list(order), "phases": {},
                      "paired_comparable": True}
            for name in order:
                target = targets[name]
                if name == "rizzy":
                    # Divide clients evenly across the four equivalent processes.
                    urls = target["urls"]
                    def client_target(index):
                        return {**target, "url": urls[index % len(urls)]}
                else:
                    def client_target(index):
                        return target
                sample = run_load({**target, "client_target": client_target},
                                  fixtures[name], args.clients, args.seconds)
                verify(args.mysql_client, args.mysql_socket,
                       source_dbs[""] if name == "fizzy" else rust_db,
                       fixtures[name], sample, name == "fizzy")
                if name == "fizzy":
                    sample["queue"] = wait_source_queue(
                        args.mysql_client, args.mysql_socket, source_dbs["QUEUE_"])
                    if sample["queue"]["drained"]:
                        sample["moves_per_second_including_queue_drain"] = round(
                            sample["moves"] / (sample["elapsed_seconds"] +
                                               sample["queue"]["drain_seconds"]), 2)
                result["phases"][name] = sample
                if not sample["valid"]:
                    result["paired_comparable"] = False
            result["paired_http_comparable"] = result["paired_comparable"]
            source_queue = result["phases"].get("fizzy", {}).get("queue")
            if (not source_warmup_queue["drained"] or
                    source_warmup_queue["final"]["failed"] or
                    not source_queue or not source_queue["drained"] or
                    source_queue["final"]["failed"]):
                result["paired_comparable"] = False
                if source_queue and source_queue["final"]["failed"]:
                    result["source_failed_jobs"] = source_failed_jobs(
                        args.mysql_client, args.mysql_socket, source_dbs["QUEUE_"])
            if not result["paired_comparable"]:
                result["diagnostics"] = {}
                for name, log in logs.items():
                    if log.exists():
                        result["diagnostics"][name] = [line for line in log.read_text().splitlines()
                                                        if "ERROR" in line or "1213" in line][-10:]
            return result
        except Exception as error:
            if isinstance(error, subprocess.CalledProcessError):
                print(f"MySQL command failed: {error.stderr}", file=sys.stderr)
            for name, log in locals().get("logs", {}).items():
                if log.exists():
                    lines = log.read_text().splitlines()
                    print(f"{name} log tail: {lines[-12:]}", file=sys.stderr)
            raise
        finally:
            for process in reversed(processes):
                stop_server(process)
            drop_databases(args.mysql_client, args.mysql_socket, databases)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--mysql-client", default="/tmp/rizzy-mysql8/bin/mysql")
    parser.add_argument("--mysql-socket", default="/tmp/rizzy-mysql8/mysql.sock")
    parser.add_argument("--mysql-host", default="127.0.0.1")
    parser.add_argument("--mysql-port", type=int, default=33080)
    parser.add_argument("--clients", type=int, default=8)
    parser.add_argument("--seconds", type=float, default=10)
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if not Path(args.mysql_socket).is_socket():
        raise SystemExit("Start the disposable MySQL server first")
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    affinity = sorted(os.sched_getaffinity(0))
    server_cpus = affinity[:4]
    client_cpus = affinity[4:8]
    original_affinity = set(affinity)
    old_trust = mysql_admin(args.mysql_client, args.mysql_socket,
                            "SELECT @@GLOBAL.log_bin_trust_function_creators").strip()
    old_max_connections = mysql_admin(args.mysql_client, args.mysql_socket,
                                      "SELECT @@GLOBAL.max_connections").strip()
    mysql_admin(args.mysql_client, args.mysql_socket,
                "SET GLOBAL log_bin_trust_function_creators=1")
    mysql_admin(args.mysql_client, args.mysql_socket,
                "SET GLOBAL max_connections=400")
    try:
        result = {"source_revision": git_output("-C", "upstream", "rev-parse", "HEAD"),
                  "rizzy_revision": git_output("rev-parse", "HEAD"),
                  "rizzy_dirty": bool(git_output("status", "--porcelain")),
                  "rizzy_binary_sha256": hashlib.sha256(RUST.read_bytes()).hexdigest(),
                  "server_cpus": server_cpus, "client_cpus": client_cpus,
                  "mysql_version": mysql_admin(args.mysql_client, args.mysql_socket,
                                                 "SELECT VERSION()").strip(),
                  "mysql_max_connections": 400,
                  "clients": args.clients, "seconds": args.seconds, "rounds": []}
        for index in range(args.rounds):
            order = ("fizzy", "rizzy") if index % 2 == 0 else ("rizzy", "fizzy")
            os.sched_setaffinity(0, original_affinity)
            round_result = run_pair(args, order, server_cpus, client_cpus)
            result["rounds"].append(round_result)
            print(f"round {index + 1}: " + ", ".join(
                f"{name}={phase['moves_per_second']}/s errors={len(phase['errors'])} "
                f"valid={phase['valid']}" for name, phase in round_result["phases"].items()),
                file=sys.stderr, flush=True)
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(result, indent=2) + "\n")
    finally:
        os.sched_setaffinity(0, original_affinity)
        mysql_admin(args.mysql_client, args.mysql_socket,
                    f"SET GLOBAL log_bin_trust_function_creators={old_trust}")
        mysql_admin(args.mysql_client, args.mysql_socket,
                    f"SET GLOBAL max_connections={old_max_connections}")


if __name__ == "__main__":
    main()
