#!/usr/bin/env python3
"""Compare a parity-checked 100-board JSON page under concurrent reads."""
import argparse
import json
import os
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests"))
from boards_index_order_parity import cleanup, fixture, verify  # noqa: E402
from compare import run_rust  # noqa: E402


def sample(target):
    request = urllib.request.Request(
        f"{target['url']}/{target['account']}/boards.json?page=4",
        headers={"Authorization": f"Bearer {target['token']}",
                 "Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=20) as response:
        body = response.read()
        boards = json.loads(body)
        assert len(boards) == 100
        return {"bytes": len(body), "first_name": boards[0]["name"],
                "last_name": boards[-1]["name"],
                "total_count": int(response.headers["X-Total-Count"])}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, default=10)
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--concurrency", default="8,32,64")
    parser.add_argument("--load-binary", default="target/release/rizzy_load")
    parser.add_argument("--output")
    args = parser.parse_args()
    levels = [int(value) for value in args.concurrency.split(",")]
    if args.seconds <= 0 or args.rounds < 1 or not levels or min(levels) < 1:
        parser.error("seconds, rounds, and concurrency must be positive")
    binary = str((ROOT / args.load_binary).resolve())
    if not Path(binary).is_file():
        parser.error(f"Build the load client first: {binary}")
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        fields = ("URL", "ACCOUNT", "TOKEN", "DB")
        if any(not os.getenv(f"{name}_{field}") for field in fields):
            parser.error(f"Set {name}_URL, {name}_ACCOUNT, {name}_TOKEN, and {name}_DB")
        target = {"name": name.lower(),
                  **{field.lower(): os.environ[f"{name}_{field}"] for field in fields}}
        if os.getenv(f"{name}_PID"):
            target["pid"] = int(os.environ[f"{name}_PID"])
        targets[name.lower()] = target
    created = []
    result = {"fixture_boards": 200, "path": "/boards.json?page=4",
              "seconds_per_sample": args.seconds, "rounds": args.rounds,
              "concurrency": levels, "samples": {}, "runs": []}
    try:
        shapes = {}
        for name, target in targets.items():
            board, new_ids, baseline = fixture(target)
            created.append((target, board, new_ids))
            pages, board_shape = verify(target, baseline)
            shapes[name] = board_shape
            result["samples"][name] = {"pages": pages, **sample(target)}
        assert shapes["fizzy"] == shapes["rizzy"], shapes
        assert result["samples"]["fizzy"]["first_name"] == result["samples"]["rizzy"]["first_name"]
        assert result["samples"]["fizzy"]["last_name"] == result["samples"]["rizzy"]["last_name"]
        result["board_shape"] = shapes["fizzy"]
        for round_number in range(args.rounds):
            for concurrency in levels:
                order = ("fizzy", "rizzy") if round_number % 2 == 0 else ("rizzy", "fizzy")
                for name in order:
                    target = targets[name]
                    measurement = run_rust(target, [result["path"]], concurrency,
                                           args.seconds, binary)
                    assert not measurement["errors"], (name, concurrency, measurement)
                    result["runs"].append({"round": round_number + 1, "target": name,
                                           **measurement})
                    print(f"round {round_number + 1} {name} {concurrency}: "
                          f"{measurement['rps']} requests/s, p95 {measurement['p95_ms']} ms",
                          flush=True)
    finally:
        for target, board, new_ids in reversed(created):
            cleanup(target, board, new_ids)
    if args.output:
        output = (ROOT / args.output).resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(result, indent=2) + "\n")
        print(output)
    else:
        print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
