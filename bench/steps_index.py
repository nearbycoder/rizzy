#!/usr/bin/env python3
"""Compare matched card step-list reads on disposable Fizzy and Rizzy databases."""
import argparse
import hashlib
import json
import os
import subprocess
import tempfile
from datetime import date
from pathlib import Path

from card_workflow import expect
from compare import run_rust
from sustained_mixed import ROOT, SOURCE, prepare_rust, prepare_source, start_server, stop_server


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def setup(name, directory, cpus, count):
    env, credentials = (prepare_source(directory, 8) if name == "fizzy"
                        else prepare_rust(directory))
    process, url = start_server(name, directory, env, cpus)
    target = {"url": url, "account": str(credentials["account"]),
              "token": credentials["token"], "pid": process.pid}
    try:
        board, _ = expect(target, "/boards", "POST", {"board": {"name": "Step index load"}}, 201)
        card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                         {"card": {"title": "Many small steps"}}, 201)
        path = f"/cards/{card['number']}/steps"
        for index in range(count):
            expect(target, path, "POST", {"step": {
                "content": f"Task {index:03d}: review the plan", "completed": index % 3 == 0
            }}, 201)
        items, _ = expect(target, path)
        sample = [(item["content"], item["completed"]) for item in items]
        for _ in range(5):
            expect(target, path)
        return process, target, path, sample
    except Exception:
        stop_server(process)
        raise


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--steps", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=4)
    parser.add_argument("--clients", default="8,32")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    clients = [int(value) for value in args.clients.split(",")]
    cpus = sorted(os.sched_getaffinity(0))[:8]
    result = {
        "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=SOURCE, text=True).strip(),
        "rizzy_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "rizzy_binary_sha256": sha256(ROOT / "target/release/rizzy"),
        "client_binary_sha256": sha256(ROOT / "target/release/rizzy_load"),
        "steps": args.steps, "server_cpus": len(cpus), "fizzy_workers": 8,
        "request_mode": "full JSON response", "seconds_per_run": args.seconds,
        "clients": clients, "runs": [],
    }
    with tempfile.TemporaryDirectory(prefix="rizzy-steps-index-load-") as temporary:
        processes = []
        targets = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                process, target, path, sample = setup(name, directory, cpus, args.steps)
                processes.append(process)
                targets[name] = (target, path, sample)
            assert targets["fizzy"][2] == targets["rizzy"][2]
            result["sample_parity"] = {"count": len(targets["fizzy"][2]), "content_and_completion": True}
            for concurrency in clients:
                for order in (("fizzy", "rizzy"), ("rizzy", "fizzy")):
                    for name in order:
                        target, path, _ = targets[name]
                        measurement = run_rust(target, [path], concurrency, args.seconds,
                                               ROOT / "target/release/rizzy_load")
                        assert measurement["errors"] == {}, (name, measurement["errors"])
                        result["runs"].append({"app": name, "clients": concurrency,
                                               "round_order": list(order), **measurement})
        finally:
            for process in reversed(processes):
                stop_server(process)
    output = args.output or ROOT / "bench/results" / f"steps-index-{args.steps}-{date.today().isoformat()}.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2) + "\n")
    print(output)


if __name__ == "__main__":
    main()
