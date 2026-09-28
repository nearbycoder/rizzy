#!/usr/bin/env python3
"""Compare repeated PNG avatar attachment and first thumbnail retrieval."""

import argparse
import base64
import hashlib
import json
import os
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests"))

from avatar_parity import PNG  # noqa: E402
from card_image_parity import call  # noqa: E402
from sustained_mixed import (RUST, prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def summary(samples):
    ordered = sorted(samples)
    return {"count": len(ordered), "median_ms": round(statistics.median(ordered), 2),
            "p95_ms": round(ordered[int((len(ordered) - 1) * .95)], 2)}


def run(target, replacements):
    prefix = f"/{target['account']}"
    status, _, raw = call(target, prefix + "/users")
    assert status == 200, (status, raw[:100])
    user = next(row for row in json.loads(raw) if row["role"] == "owner")
    path = prefix + f"/users/{user['id']}/avatar"
    metadata = {"filename": "avatar.png", "byte_size": len(PNG),
                "checksum": base64.b64encode(hashlib.md5(PNG).digest()).decode(),
                "content_type": "image/png"}
    patches, first_reads, checksums = [], [], set()
    for _ in range(replacements):
        status, _, raw = call(target, prefix + "/rails/active_storage/direct_uploads", "POST",
                              {"blob": metadata})
        assert status == 200, (status, raw[:100])
        upload = json.loads(raw)
        status, _, raw = call(target, upload["direct_upload"]["url"], "PUT", PNG,
                              bearer=False, headers=upload["direct_upload"]["headers"])
        assert status == 204, (status, raw[:100])
        started = time.perf_counter()
        status, _, raw = call(target, prefix + f"/users/{user['id']}.json", "PATCH",
                              {"user": {"avatar": upload["signed_id"]}})
        patches.append((time.perf_counter() - started) * 1000)
        assert status == 200, (status, raw[:100])
        started = time.perf_counter()
        with urllib.request.urlopen(target["url"] + path, timeout=15) as response:
            thumbnail = response.read()
            assert response.status == 200 and response.headers.get_content_type() == "image/png"
        first_reads.append((time.perf_counter() - started) * 1000)
        assert (int.from_bytes(thumbnail[16:20], "big"),
                int.from_bytes(thumbnail[20:24], "big")) == (256, 256)
        checksums.add(hashlib.sha256(thumbnail).hexdigest())
    assert len(checksums) == 1, checksums
    status, _, raw = call(target, path + ".json", "DELETE")
    assert status == 204, (status, raw[:100])
    return {"attachment": summary(patches), "first_thumbnail": summary(first_reads),
            "thumbnail_sha256": checksums.pop()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--replacements", type=int, default=12)
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    assert args.replacements > 0 and args.rounds > 0
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    dirty = bool(subprocess.check_output(
        ["git", "status", "--porcelain", "--untracked-files=no"],
        cwd=ROOT, text=True).strip())
    cpus = sorted(os.sched_getaffinity(0))[:2]
    results = {"rizzy_commit": revision, "rizzy_dirty": dirty,
               "rizzy_binary_sha256": hashlib.sha256(RUST.read_bytes()).hexdigest(),
               "upstream_commit": "477c943e0506f109e5bc83ae9dadbe519732c045",
               "fixture": {"format": "PNG", "width": 806, "height": 926, "bytes": len(PNG)},
               "server_cpus": len(cpus), "source_workers": 2,
               "replacements_per_app_per_round": args.replacements, "rounds": []}
    with tempfile.TemporaryDirectory(prefix="rizzy-avatar-attach-compare-") as temp:
        root = Path(temp)
        for round_index in range(args.rounds):
            servers, targets = [], {}
            try:
                for name, prepare in (("fizzy", prepare_source), ("rizzy", prepare_rust)):
                    directory = root / f"round-{round_index}" / name
                    directory.mkdir(parents=True)
                    env, seed = (prepare(directory, 2) if name == "fizzy" else
                                 prepare(directory, {**rust_env(directory),
                                                     "RIZZY_DISABLE_STORAGE_PURGE": "1"}))
                    process, url = start_server(name, directory, env, cpus)
                    servers.append(process)
                    targets[name] = {"url": url, **seed}
                order = ("fizzy", "rizzy") if round_index % 2 == 0 else ("rizzy", "fizzy")
                result = {"order": order}
                for name in order:
                    result[name] = run(targets[name], args.replacements)
                    print(f"round {round_index + 1} {name}: {result[name]}", file=sys.stderr, flush=True)
                assert result["fizzy"]["thumbnail_sha256"] == result["rizzy"]["thumbnail_sha256"]
                results["rounds"].append(result)
            finally:
                for process in reversed(servers):
                    stop_server(process)
    output = json.dumps(results, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(output)
    print(output, end="")


if __name__ == "__main__":
    main()
