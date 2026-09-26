#!/usr/bin/env python3
"""Build the same API fixture in Fizzy and Rizzy, then measure both HTTP servers.

Example:
  FIZZY_URL=http://127.0.0.1:3006 FIZZY_ACCOUNT=123 FIZZY_TOKEN=... \
  RIZZY_URL=http://127.0.0.1:3007 RIZZY_ACCOUNT=1 RIZZY_TOKEN=... \
  python3 bench/compare.py --cards 100 --seconds 10 --workload scoped
"""
import argparse
import atexit
import concurrent.futures
import itertools
import json
import os
import resource
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from collections import Counter
from pathlib import Path


def request(target, path, method="GET", payload=None, with_headers=False):
    body = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(
        f"{target['url']}/{target['account']}{path}", body, method=method,
        headers={"Authorization": f"Bearer {target['token']}",
                 "Accept": "application/json", "Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=15) as response:
        data = response.read()
        parsed = json.loads(data) if data else None
        return (parsed, response.headers) if with_headers else parsed


def shape(value):
    if isinstance(value, dict):
        return {key: shape(item) for key, item in value.items()}
    if isinstance(value, list):
        return [shape(value[0])] if value else []
    return type(value).__name__


def fixture(target, count, on_board=None, board_name=None):
    board = request(target, "/boards", "POST", {"board": {"name": board_name or f"Rizzy benchmark {int(time.time())}"}})
    board_id = board["id"]
    if on_board:
        on_board(board_id)
    card_number = None
    for n in range(count):
        card = request(target, f"/boards/{board_id}/cards", "POST", {"card": {"title": f"Benchmark card {n}", "description": "The same workload on each application."}})
        if n == 0:
            card_number = card["number"]
    return board_id, card_number


def pct(samples, p):
    return round(sorted(samples)[min(len(samples)-1, int((len(samples)-1)*p))]*1000, 2) if samples else None


def run(target, paths, concurrency, seconds):
    deadline = time.monotonic() + seconds
    starts = time.monotonic()
    samples, failures = [], Counter()
    path_requests = Counter()
    sequence = itertools.count()
    lock = threading.Lock()

    def worker(_worker_id):
        local, errs, local_paths = [], Counter(), Counter()
        while time.monotonic() < deadline:
            path_index = next(sequence) % len(paths)
            t = time.perf_counter()
            try:
                request(target, paths[path_index])
                local.append(time.perf_counter()-t)
                local_paths[path_index] += 1
            except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as e:
                errs[str(getattr(e, "code", type(e).__name__))] += 1
        with lock:
            samples.extend(local)
            failures.update(errs)
            path_requests.update(local_paths)

    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
        list(pool.map(worker, range(concurrency)))
    elapsed = time.monotonic()-starts
    return {"concurrency": concurrency, "seconds": round(elapsed, 2),
            "requests": len(samples), "errors": dict(failures),
            "path_requests": [path_requests[index] for index in range(len(paths))],
            "rps": round(len(samples)/elapsed, 1), "p50_ms": pct(samples, .50),
            "p95_ms": pct(samples, .95), "p99_ms": pct(samples, .99)}


def process_sample(root_pid):
    """Read CPU, proportional memory, and I/O for one Linux process tree."""
    if root_pid is None:
        return None
    entries = {}
    for directory in Path("/proc").iterdir():
        if not directory.name.isdigit():
            continue
        try:
            fields = (directory / "stat").read_text().rsplit(")", 1)[1].split()
            entries[int(directory.name)] = (int(fields[1]), int(fields[11]) + int(fields[12]))
        except (OSError, IndexError, ValueError):
            continue
    if root_pid not in entries:
        raise RuntimeError(f"Benchmark server PID {root_pid} is not running")
    pids = {root_pid}
    while True:
        children = {pid for pid, (parent, _) in entries.items() if parent in pids}
        if children <= pids:
            break
        pids |= children
    pss_kib = read_bytes = write_bytes = 0
    for pid in pids:
        try:
            for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines():
                if line.startswith("Pss:"):
                    pss_kib += int(line.split()[1])
                    break
            for line in Path(f"/proc/{pid}/io").read_text().splitlines():
                if line.startswith("read_bytes:"):
                    read_bytes += int(line.split()[1])
                elif line.startswith("write_bytes:"):
                    write_bytes += int(line.split()[1])
        except (OSError, ValueError):
            continue
    ticks = sum(entries[pid][1] for pid in pids if pid in entries)
    return {"processes": len(pids), "cpu_seconds": ticks / os.sysconf("SC_CLK_TCK"),
            "pss_mib": pss_kib / 1024, "read_mib": read_bytes / 1048576,
            "write_mib": write_bytes / 1048576}


def run_rust(target, paths, concurrency, seconds, load_binary):
    env = {**os.environ, "LOAD_URL": target["url"], "LOAD_ACCOUNT": target["account"],
           "LOAD_TOKEN": target["token"], "LOAD_PATHS_JSON": json.dumps(paths),
           "LOAD_CONCURRENCY": str(concurrency), "LOAD_SECONDS": str(seconds)}
    if target.get("urls"):
        env["LOAD_URLS_JSON"] = json.dumps(target["urls"])
    else:
        env.pop("LOAD_URLS_JSON", None)
    server_pids = target.get("pids", [target["pid"]] if target.get("pid") else [])

    def server_sample():
        samples = [process_sample(pid) for pid in server_pids]
        return {key: sum(sample[key] for sample in samples) for key in samples[0]} if samples else None

    before = server_sample()
    child_before = resource.getrusage(resource.RUSAGE_CHILDREN)
    process = subprocess.Popen([load_binary], env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True)
    samples = [before] if before else []
    while process.poll() is None:
        if server_pids:
            samples.append(server_sample())
        time.sleep(0.5)
    output, error = process.communicate()
    if process.returncode:
        raise RuntimeError(f"Rust load generator exited {process.returncode}: {error.strip()}")
    result = json.loads(output)
    child_after = resource.getrusage(resource.RUSAGE_CHILDREN)
    result["generator_cpu_seconds"] = round(
        child_after.ru_utime + child_after.ru_stime - child_before.ru_utime - child_before.ru_stime, 2)
    if server_pids:
        after = server_sample()
        samples.append(after)
        elapsed = result["seconds"]
        result["server_resources"] = {
            "processes_peak": max(item["processes"] for item in samples),
            "pss_mib_start": round(before["pss_mib"], 1),
            "pss_mib_peak": round(max(item["pss_mib"] for item in samples), 1),
            "pss_mib_end": round(after["pss_mib"], 1),
            "cpu_seconds": round(after["cpu_seconds"] - before["cpu_seconds"], 2),
            "cpu_cores_average": round((after["cpu_seconds"] - before["cpu_seconds"]) / elapsed, 2),
            "read_mib": round(after["read_mib"] - before["read_mib"], 2),
            "write_mib": round(after["write_mib"] - before["write_mib"], 2),
        }
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=10)
    parser.add_argument("--rounds", type=int, default=1,
                        help="repeat each level, alternating which server runs first")
    parser.add_argument("--client", choices=("python", "rust"), default="python")
    parser.add_argument("--load-binary", default="target/release/rizzy_load")
    parser.add_argument("--concurrency", default="1,8,32")
    parser.add_argument("--workload", choices=("mixed", "scoped"), default="mixed",
                        help="scoped excludes the account-wide board list")
    args = parser.parse_args()
    if args.cards < 1 or args.seconds <= 0 or args.rounds < 1:
        parser.error("--cards, --seconds, and --rounds must be positive")
    if args.client == "rust" and not Path(args.load_binary).is_file():
        parser.error(f"Build the Rust load client first: cargo build --release --bin rizzy_load")
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        keys = [f"{name}_{field}" for field in ("URL", "ACCOUNT", "TOKEN")]
        absent = [k for k in keys if not os.getenv(k)]
        if len(absent) == len(keys):
            continue
        if absent:
            parser.error("Missing " + ", ".join(absent))
        target = {field.lower(): os.environ[f"{name}_{field}"] for field in ("URL", "ACCOUNT", "TOKEN")}
        if os.getenv(f"{name}_PID"):
            target["pid"] = int(os.environ[f"{name}_PID"])
        targets[name.lower()] = target
    if not targets:
        parser.error("Set at least one complete FIZZY_* or RIZZY_* target")
    results = {"fixture_cards": args.cards, "workload": args.workload,
               "rounds": args.rounds, "client": args.client,
               "host_logical_cpus": os.cpu_count(), "runs": {}}
    paths = {}
    samples = {}
    created_boards = []
    board_name = f"Rizzy benchmark {time.time_ns()}"

    def cleanup():
        for target, board_id in reversed(created_boards):
            try:
                request(target, f"/boards/{board_id}", "DELETE")
            except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError):
                pass

    atexit.register(cleanup)
    for name, target in targets.items():
        board, number = fixture(target, args.cards,
                                lambda board_id: created_boards.append((target, board_id)),
                                board_name)
        filtered_cards = f"/cards?board_ids%5B%5D={board}"
        activities = f"/activities?board_ids%5B%5D={board}"
        paths[name] = [f"/boards/{board}", f"/cards/{number}", filtered_cards, activities]
        if args.workload == "mixed":
            paths[name].insert(0, "/boards")
        # Verify each app actually returns the seeded objects before timing it.
        board_sample = request(target, f"/boards/{board}")
        card_sample = request(target, f"/cards/{number}")
        page_sample, page_headers = request(target, filtered_cards, with_headers=True)
        activity_sample, activity_headers = request(target, activities, with_headers=True)
        assert board_sample["name"].startswith("Rizzy benchmark")
        assert card_sample["title"] == "Benchmark card 0"
        assert len(page_sample) == min(args.cards, 15), (name, "first page count", len(page_sample))
        assert int(page_headers["X-Total-Count"]) == args.cards, (name, "total count", page_headers["X-Total-Count"])
        assert len(activity_sample) == min(args.cards, 15), (name, "activity first page count", len(activity_sample))
        assert int(activity_headers["X-Total-Count"]) == args.cards, (name, "activity total count", activity_headers["X-Total-Count"])
        samples[name] = {"board_shape": shape(board_sample), "card_shape": shape(card_sample),
                         "activity_shape":shape(activity_sample),
                         "board_name": board_sample["name"],
                         "card_semantics": {key: card_sample[key] for key in
                                            ("title", "description", "status", "closed",
                                             "postponed", "golden", "has_attachments", "tags")},
                         "first_page_titles": [card["title"] for card in page_sample],
                         "activity_actions": [event["action"] for event in activity_sample],
                         "activity_descriptions": [event["description"] for event in activity_sample],
                         "card_description_html": card_sample["description_html"],
                         "first_page_count": len(page_sample), "first_page_bytes": len(json.dumps(page_sample))}
        for path in paths[name]:
            for _ in range(20):
                request(target, path)
    if len(targets) == 2:
        left, right = samples["fizzy"], samples["rizzy"]
        results["sample_parity"] = {"board_shape": left["board_shape"] == right["board_shape"],
                                    "card_shape": left["card_shape"] == right["card_shape"],
                                    "activity_shape": left["activity_shape"] == right["activity_shape"],
                                    "board_name": left["board_name"] == right["board_name"],
                                    "card_semantics": left["card_semantics"] == right["card_semantics"],
                                    "first_page_titles": left["first_page_titles"] == right["first_page_titles"],
                                    "activity_actions": left["activity_actions"] == right["activity_actions"],
                                    "activity_descriptions": left["activity_descriptions"] == right["activity_descriptions"],
                                    "description_html": left["card_description_html"] == right["card_description_html"],
                                    "first_page_count": left["first_page_count"] == right["first_page_count"],
                                    "first_page_bytes": {name: sample["first_page_bytes"] for name, sample in samples.items()}}
        assert all(value for key,value in results["sample_parity"].items() if key != "first_page_bytes"), results["sample_parity"]
    levels = [int(value) for value in args.concurrency.split(",")]
    if not levels or any(value < 1 for value in levels):
        parser.error("--concurrency must be a comma-separated list of positive integers")
    for concurrency in levels:
        for round_number in range(1, args.rounds + 1):
            names = list(targets)
            if round_number % 2 == 0:
                names.reverse()
            for name in names:
                result = (run_rust(targets[name], paths[name], concurrency, args.seconds,
                                   args.load_binary) if args.client == "rust" else
                          run(targets[name], paths[name], concurrency, args.seconds))
                result["round"] = round_number
                results["runs"].setdefault(name, []).append(result)
                print(f"{name} c={concurrency} round={round_number}: {result}",
                      file=sys.stderr, flush=True)
    if len(targets) == 2:
        for fizzy, rizzy in zip(results["runs"]["fizzy"], results["runs"]["rizzy"]):
            results.setdefault("comparison", []).append({"concurrency": fizzy["concurrency"],
                "round": fizzy["round"],
                "rps_ratio_rizzy_over_fizzy": round(rizzy["rps"]/fizzy["rps"], 3) if fizzy["rps"] else None,
                "p95_ratio_fizzy_over_rizzy": round(fizzy["p95_ms"]/rizzy["p95_ms"], 3) if rizzy["p95_ms"] else None})
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
