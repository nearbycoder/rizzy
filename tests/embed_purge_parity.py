#!/usr/bin/env python3
"""Compare staged Action Text embed purges with the pinned Fizzy app."""

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from avatar_purge_parity import drain, old_blob_state, pending_jobs  # noqa: E402
from card_image_parity import IMAGE, call  # noqa: E402
from embed_parity import FILE, upload  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, rust_env, start_server, stop_server  # noqa: E402


def checked(target, path, method="GET", payload=None, expected=200):
    status, _, raw = call(target, f"/{target['account']}{path}", method, payload)
    assert status == expected, (target["name"], path, status, raw[:250])
    return json.loads(raw) if raw else None


def variant_keys(target, blob):
    assert blob["key"].isalnum()
    source = target["name"] == "fizzy"
    blobs = "active_storage_blobs" if source else "storage_blobs"
    variants = "active_storage_variant_records" if source else "storage_variant_records"
    attachments = "active_storage_attachments" if source else "storage_attachments"
    key = "`key`" if source else "storage_key"
    statement = (f"SELECT d.{key} FROM {variants} v JOIN {blobs} b ON b.id=v.blob_id "
                 f"JOIN {attachments} a ON a.record_type='ActiveStorage::VariantRecord' "
                 f"AND a.record_id=v.id JOIN {blobs} d ON d.id=a.blob_id "
                 f"WHERE b.{key}='{blob['key']}' ORDER BY d.{key}")
    if target.get("mysql_database"):
        from sustained_mixed_mysql import mysql_query
        value = mysql_query(target["mysql_client"], target["mysql_socket"],
                            target["mysql_database"], statement)
        return value.splitlines() if value else []
    with sqlite3.connect(target["database"]) as db:
        return [row[0] for row in db.execute(statement)]


def markup(blob):
    return f'<p><action-text-attachment sgid="{blob["attachable_sgid"]}"></action-text-attachment></p>'


def assert_bytes(target, keys, present):
    if target.get("storage"):
        for key in keys:
            assert (target["storage"] / key).exists() == present, (target["name"], key)


def finish_purge(target, blob, derived):
    drain(target, blob["key"])
    assert old_blob_state(target, blob) == (0, 0, 0), target["name"]
    assert_bytes(target, [blob["key"]], False)
    assert_bytes(target, derived, True)
    assert pending_jobs(target) == (1 + len(derived) if target["name"] == "fizzy" else len(derived)), target["name"]
    for key in derived:
        drain(target, key)
    assert_bytes(target, derived, False)
    assert all(old_blob_state(target, {"key": key}) == (0, 0, 0) for key in derived)
    if target["name"] == "rizzy":
        assert pending_jobs(target) == 0


def run(target):
    board = checked(target, "/boards", "POST", {"board": {"name": "Embed purge"}}, 201)
    blob = upload(target, IMAGE, "moon.jpg", "image/jpeg")
    card = checked(target, f"/boards/{board['id']}/cards", "POST",
                   {"card": {"title": "One embed", "description": markup(blob)}}, 201)
    derived = variant_keys(target, blob)
    assert len(derived) == 2, (target["name"], derived)
    assert old_blob_state(target, blob) == (1, 2, 1)
    assert_bytes(target, [blob["key"], *derived], True)
    checked(target, f"/cards/{card['number']}.json", "PATCH",
            {"card": {"description": "Removed embed"}})
    assert old_blob_state(target, blob) == (1, 2, 0), target["name"]
    assert pending_jobs(target) == 1, target["name"]
    assert_bytes(target, [blob["key"], *derived], True)
    finish_purge(target, blob, derived)
    earlier_jobs = pending_jobs(target)

    shared = upload(target, FILE, "notes.txt", "text/plain")
    card = checked(target, f"/boards/{board['id']}/cards", "POST",
                   {"card": {"title": "Shared embed", "description": markup(shared)}}, 201)
    comment = checked(target, f"/cards/{card['number']}/comments", "POST",
                      {"comment": {"body": markup(shared)}}, 201)
    assert old_blob_state(target, shared) == (1, 0, 2)
    checked(target, f"/cards/{card['number']}.json", "PATCH",
            {"card": {"description": "Removed from card"}})
    assert old_blob_state(target, shared) == (1, 0, 1)
    assert pending_jobs(target) == earlier_jobs
    checked(target, f"/cards/{card['number']}/comments/{comment['id']}", "DELETE", expected=204)
    assert old_blob_state(target, shared) == (1, 0, 0)
    assert pending_jobs(target) == earlier_jobs + 1
    # The source's manually performed jobs remain in Solid Queue's job table.
    drain(target, shared["key"])
    assert old_blob_state(target, shared) == (0, 0, 0)
    assert_bytes(target, [shared["key"]], False)

    last = upload(target, FILE, "last.txt", "text/plain")
    checked(target, f"/boards/{board['id']}/cards", "POST",
            {"card": {"title": "Board delete", "description": markup(last)}}, 201)
    checked(target, f"/boards/{board['id']}", "DELETE", expected=204)
    assert old_blob_state(target, last) == (1, 0, 0)
    assert pending_jobs(target) == earlier_jobs + (2 if target["name"] == "fizzy" else 1)
    drain(target, last["key"])
    assert old_blob_state(target, last) == (0, 0, 0)
    assert_bytes(target, [last["key"]], False)
    return {"image_edit": "staged", "shared_file": "last attachment",
            "board_delete": "staged"}


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    results = {}
    with tempfile.TemporaryDirectory(prefix="rizzy-embed-purge-") as temp:
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
                print(f"{name}: staged embed purge passed", flush=True)
        finally:
            for process in reversed(servers):
                stop_server(process)
    assert results["fizzy"] == results["rizzy"], results


if __name__ == "__main__":
    main()
