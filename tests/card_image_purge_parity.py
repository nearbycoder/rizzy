#!/usr/bin/env python3
"""Compare deferred card-image cleanup with the pinned Fizzy app."""

import base64
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from avatar_purge_parity import drain, old_blob_state, pending_jobs  # noqa: E402
from card_image_parity import IMAGE, call  # noqa: E402
from sustained_mixed import (ROOT as BENCH_ROOT, prepare_rust, prepare_source,
                             rust_env, start_server, stop_server)  # noqa: E402


def checked(target, path, method="GET", payload=None, expected=200):
    status, _, raw = call(target, path, method, payload)
    assert status == expected, (target["name"], path, status, raw[:250])
    return json.loads(raw) if raw else None


def upload(target):
    prefix = f"/{target['account']}"
    blob = {"filename": "moon.jpg", "byte_size": len(IMAGE),
            "checksum": base64.b64encode(hashlib.md5(IMAGE).digest()).decode(),
            "content_type": "image/jpeg"}
    result = checked(target, prefix + "/rails/active_storage/direct_uploads",
                     "POST", {"blob": blob})
    status, _, raw = call(target, result["direct_upload"]["url"], "PUT", IMAGE,
                          bearer=False, headers=result["direct_upload"]["headers"])
    assert status == 204, (target["name"], status, raw[:100])
    return result


def assert_before_drain(target, blob, expected_jobs):
    assert old_blob_state(target, blob) == (1, 0, 0), (target["name"], blob["key"])
    assert pending_jobs(target) == expected_jobs, (target["name"], pending_jobs(target))
    if target.get("storage"):
        assert (target["storage"] / blob["key"]).read_bytes() == IMAGE


def drain_and_check(target, blob):
    drain(target, blob["key"])
    assert old_blob_state(target, blob) == (0, 0, 0), target["name"]
    if target.get("storage"):
        assert not (target["storage"] / blob["key"]).exists()


def run(target):
    prefix = f"/{target['account']}"
    board = checked(target, prefix + "/boards", "POST",
                    {"board": {"name": "Card image purge"}}, 201)
    card = checked(target, prefix + f"/boards/{board['id']}/cards", "POST",
                   {"card": {"title": "Image purge"}}, 201)
    path = prefix + f"/cards/{card['number']}.json"
    first = upload(target)
    checked(target, path, "PATCH", {"card": {"image": first["signed_id"]}})
    second = upload(target)
    checked(target, path, "PATCH", {"card": {"image": second["signed_id"]}})
    assert_before_drain(target, first, 1)
    assert old_blob_state(target, second) == (1, 0, 1), target["name"]
    drain_and_check(target, first)

    checked(target, prefix + f"/cards/{card['number']}/image.json", "DELETE",
            expected=204)
    assert_before_drain(target, second, 2 if target["name"] == "fizzy" else 1)
    drain_and_check(target, second)

    doomed = checked(target, prefix + f"/boards/{board['id']}/cards", "POST",
                     {"card": {"title": "Delete with card"}}, 201)
    third = upload(target)
    checked(target, prefix + f"/cards/{doomed['number']}.json", "PATCH",
            {"card": {"image": third["signed_id"]}})
    checked(target, prefix + f"/cards/{doomed['number']}", "DELETE", expected=204)
    assert_before_drain(target, third, 3 if target["name"] == "fizzy" else 1)
    drain_and_check(target, third)

    other = checked(target, prefix + f"/boards/{board['id']}/cards", "POST",
                    {"card": {"title": "Delete with board"}}, 201)
    fourth = upload(target)
    checked(target, prefix + f"/cards/{other['number']}.json", "PATCH",
            {"card": {"image": fourth["signed_id"]}})
    checked(target, prefix + f"/boards/{board['id']}", "DELETE", expected=204)
    assert_before_drain(target, fourth, 4 if target["name"] == "fizzy" else 1)
    drain_and_check(target, fourth)
    assert pending_jobs(target) == (4 if target["name"] == "fizzy" else 0)
    return {"replacement": "deferred", "explicit_delete": "deferred",
            "card_delete": "deferred", "board_delete": "deferred",
            "drain": "removed bytes"}


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=BENCH_ROOT, check=True)
    results = {}
    with tempfile.TemporaryDirectory(prefix="rizzy-card-image-purge-") as temp:
        servers = []
        try:
            for name, prepare in (("fizzy", prepare_source), ("rizzy", prepare_rust)):
                directory = Path(temp) / name
                directory.mkdir()
                env, seed = (prepare(directory, 2) if name == "fizzy" else
                             prepare(directory, {**rust_env(directory),
                                                 "RIZZY_DISABLE_STORAGE_PURGE": "1"}))
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                servers.append(process)
                target = {"name": name, "url": url, "env": env,
                          "database": directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3"),
                          **seed}
                if name == "fizzy":
                    target["queue_database"] = directory / "queue_db.sqlite3"
                else:
                    target["storage"] = directory / "storage"
                results[name] = run(target)
                print(f"{name}: deferred card-image purge passed", flush=True)
        finally:
            for process in reversed(servers):
                stop_server(process)
    assert results["fizzy"] == results["rizzy"], results


if __name__ == "__main__":
    main()
