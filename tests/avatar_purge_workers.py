#!/usr/bin/env python3
"""Drain staged avatar purge jobs safely across two Rust server processes."""

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from avatar_purge_parity import upload_avatar  # noqa: E402
from card_image_parity import call  # noqa: E402
from sustained_mixed import prepare_rust, rust_env, start_server, stop_server  # noqa: E402


def counts(database):
    with sqlite3.connect(database) as db:
        return tuple(db.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
                     for table in ("storage_purge_jobs", "storage_blobs",
                                   "storage_variant_records", "storage_attachments"))


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-avatar-purge-workers-") as temp:
        directory = Path(temp)
        env, seed = prepare_rust(directory, {**rust_env(directory),
                                             "RIZZY_DISABLE_STORAGE_PURGE": "1"})
        cpus = sorted(os.sched_getaffinity(0))[:2]
        process, url = start_server("rizzy", directory, env, cpus)
        target = {"name": "rizzy", "url": url,
                  "database": directory / "rizzy.sqlite3",
                  "storage": directory / "storage", **seed}
        blobs = []
        try:
            prefix = f"/{seed['account']}"
            status, _, raw = call(target, prefix + "/users")
            assert status == 200, (status, raw[:100])
            user = next(row for row in json.loads(raw) if row["role"] == "owner")
            for _ in range(3):
                blobs.append(upload_avatar(target, user["id"]))
            status, _, raw = call(target, prefix + f"/users/{user['id']}/avatar.json", "DELETE")
            assert status == 204, (status, raw[:100])
            assert counts(directory / "rizzy.sqlite3") == (3, 6, 3, 3)
        finally:
            stop_server(process)
        database = directory / "rizzy.sqlite3"
        with sqlite3.connect(database) as db:
            db.execute("UPDATE storage_purge_jobs SET state='in_progress',claimed_at='2000-01-01T00:00:00.000Z' WHERE blob_id=(SELECT blob_id FROM storage_purge_jobs ORDER BY created_at,blob_id LIMIT 1)")
        worker_env = {key: value for key, value in env.items()
                      if key != "RIZZY_DISABLE_STORAGE_PURGE"}
        workers = []
        try:
            for index in range(2):
                worker_dir = directory / f"worker-{index}"
                worker_dir.mkdir()
                worker, _ = start_server("rizzy", worker_dir, worker_env, cpus)
                workers.append(worker)
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline and counts(database) != (0, 0, 0, 0):
                time.sleep(.1)
            assert counts(database) == (0, 0, 0, 0), counts(database)
            for original, derived in blobs:
                assert not (directory / "storage" / original["key"]).exists()
                assert not (directory / "storage" / derived[2]).exists()
            print("Two Rust workers drained three originals and three derived blobs, including a stale claim")
        finally:
            for worker in reversed(workers):
                stop_server(worker)


if __name__ == "__main__":
    main()
