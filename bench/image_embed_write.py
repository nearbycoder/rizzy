#!/usr/bin/env python3
"""Compare synchronous Action Text image-embed writes on paired SQLite apps."""

import argparse
import hashlib
import json
import math
import os
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests"))
from card_image_parity import IMAGE  # noqa: E402
from avatar_parity import PNG  # noqa: E402
from embed_parity import upload  # noqa: E402
from card_workflow import expect  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402

IMAGES = {
    "jpeg": (IMAGE, "moon.jpg", "image/jpeg"),
    "png": (PNG, "avatar.png", "image/png"),
    "gif": ((ROOT / "tests/fixtures/animated.gif").read_bytes(), "animated.gif", "image/gif"),
    "webp": ((ROOT / "tests/fixtures/animated.webp").read_bytes(), "animated.webp", "image/webp"),
}


def revision(path):
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=path, text=True).strip()


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)]


def prepare(target, image, count):
    board, _ = expect(target, "/boards", "POST", {"board": {"name": "Image embed benchmark"}}, 201)
    requests = []
    for index in range(count):
        card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                         {"card": {"title": f"Image {index:03d}"}}, 201)
        blob = upload(target, *image)
        description = (f'<p><action-text-attachment sgid="{blob["attachable_sgid"]}">'
                       '</action-text-attachment></p>')
        requests.append((card["number"], description))
    return requests


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image-format", choices=IMAGES, default="jpeg")
    parser.add_argument("--samples", type=int, default=20)
    parser.add_argument("--warmup", type=int, default=3)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.samples < 2 or args.warmup < 1:
        parser.error("at least two samples and one warmup are required")

    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"], cwd=ROOT, check=True)
    affinity = sorted(os.sched_getaffinity(0))[:4]
    image = IMAGES[args.image_format]
    count = args.samples + args.warmup
    results = {name: [] for name in ("fizzy", "rizzy")}
    with tempfile.TemporaryDirectory(prefix="rizzy-image-embed-write-") as temporary:
        processes = []
        targets = {}
        requests = {}
        directories = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                process, url = start_server(name, directory, env, affinity)
                processes.append(process)
                targets[name] = {"name": name, "url": url,
                                 "account": str(credentials["account"]),
                                 "token": credentials["token"]}
                directories[name] = directory
                requests[name] = prepare(targets[name], image, count)

            for index in range(count):
                order = ("fizzy", "rizzy") if index % 2 == 0 else ("rizzy", "fizzy")
                for name in order:
                    number, description = requests[name][index]
                    started = time.perf_counter_ns()
                    card, _ = expect(targets[name], f"/cards/{number}", "PUT",
                                     {"card": {"description": description}}, 200)
                    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
                    assert card["has_attachments"] and "/representations/" in card["description_html"]
                    if index >= args.warmup:
                        results[name].append(elapsed_ms)

            counts = {}
            for name, directory in directories.items():
                database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
                table = ("active_storage_variant_records" if name == "fizzy"
                         else "storage_variant_records")
                with sqlite3.connect(database) as connection:
                    counts[name] = connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
                assert counts[name] == 2 * count, (name, counts[name], count)
        finally:
            for process in reversed(processes):
                stop_server(process)

    report = {
        "source_revision": revision(ROOT / "upstream"),
        "rizzy_revision": revision(ROOT),
        "rizzy_binary_sha256": hashlib.sha256((ROOT / "target/release/rizzy").read_bytes()).hexdigest(),
        "dirty_checkout": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT)),
        "image_format": args.image_format,
        "image_bytes": len(image[0]),
        "samples": args.samples,
        "warmup": args.warmup,
        "server_affinity": affinity,
        "variant_record_counts": counts,
        "latency_ms": {
            name: {"median": round(statistics.median(values), 3),
                   "p95": round(percentile(values, .95), 3),
                   "mean": round(statistics.mean(values), 3),
                   "samples": [round(value, 3) for value in values]}
            for name, values in results.items()
        },
    }
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
