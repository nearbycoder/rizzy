#!/usr/bin/env python3
"""Compare queued avatar purges and drain old original and derived blobs."""

import base64
import hashlib
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from avatar_parity import (PNG, assert_derived_removed, assert_original_removed,  # noqa: E402
                           preprocessed_avatar)
from card_image_parity import call  # noqa: E402
from sustained_mixed import (RUST, SOURCE, prepare_rust, prepare_source,  # noqa: E402
                             rust_env, start_server, stop_server)


def upload_avatar(target, user_id):
    prefix = f"/{target['account']}"
    blob = {"filename": "avatar.png", "byte_size": len(PNG),
            "checksum": base64.b64encode(hashlib.md5(PNG).digest()).decode(),
            "content_type": "image/png"}
    status, _, raw = call(target, prefix + "/rails/active_storage/direct_uploads", "POST",
                          {"blob": blob})
    assert status == 200, (status, raw[:100])
    upload = json.loads(raw)
    status, _, raw = call(target, upload["direct_upload"]["url"], "PUT", PNG,
                          bearer=False, headers=upload["direct_upload"]["headers"])
    assert status == 204, (status, raw[:100])
    status, _, raw = call(target, prefix + f"/users/{user_id}.json", "PATCH",
                          {"user": {"avatar": upload["signed_id"]}})
    assert status == 200, (status, raw[:100])
    derived = preprocessed_avatar(target, upload, "png")
    return upload, derived


def old_blob_state(target, upload):
    source = target["name"] == "fizzy"
    blobs = "active_storage_blobs" if source else "storage_blobs"
    variants = "active_storage_variant_records" if source else "storage_variant_records"
    attachments = "active_storage_attachments" if source else "storage_attachments"
    key = "`key`" if source else "storage_key"
    if target.get("mysql_database"):
        from sustained_mixed_mysql import mysql_query
        assert upload["key"].isalnum()
        value = mysql_query(target["mysql_client"], target["mysql_socket"],
                            target["mysql_database"],
                            f"SELECT (SELECT COUNT(*) FROM {blobs} WHERE {key}='{upload['key']}'),"
                            f"(SELECT COUNT(*) FROM {variants} v JOIN {blobs} b ON b.id=v.blob_id WHERE b.{key}='{upload['key']}'),"
                            f"(SELECT COUNT(*) FROM {attachments} a JOIN {blobs} b ON b.id=a.blob_id WHERE b.{key}='{upload['key']}')")
        return tuple(map(int, value.split("\t")))
    with sqlite3.connect(target["database"]) as db:
        row = db.execute(f"SELECT id FROM {blobs} WHERE {key}=?", (upload["key"],)).fetchone()
        if not row:
            return (0, 0, 0)
        blob_id = row[0]
        return (1,
                db.execute(f"SELECT COUNT(*) FROM {variants} WHERE blob_id=?", (blob_id,)).fetchone()[0],
                db.execute(f"SELECT COUNT(*) FROM {attachments} WHERE blob_id=?", (blob_id,)).fetchone()[0])


def derived_rows(target, derived):
    source = target["name"] == "fizzy"
    blobs = "active_storage_blobs" if source else "storage_blobs"
    key = "`key`" if source else "storage_key"
    if target.get("mysql_database"):
        from sustained_mixed_mysql import mysql_query
        assert derived[2].isalnum()
        return int(mysql_query(target["mysql_client"], target["mysql_socket"],
                               target["mysql_database"],
                               f"SELECT COUNT(*) FROM {blobs} WHERE {key}='{derived[2]}'"))
    with sqlite3.connect(target["database"]) as db:
        return db.execute(f"SELECT COUNT(*) FROM {blobs} WHERE {key}=?", (derived[2],)).fetchone()[0]


def pending_jobs(target):
    if target.get("mysql_database"):
        from sustained_mixed_mysql import mysql_query
        if target["name"] == "fizzy":
            return int(mysql_query(target["mysql_client"], target["mysql_socket"],
                                   target["queue_database"],
                                   "SELECT COUNT(*) FROM solid_queue_jobs WHERE class_name='ActiveStorage::PurgeJob'"))
        return int(mysql_query(target["mysql_client"], target["mysql_socket"],
                               target["mysql_database"],
                               "SELECT COUNT(*) FROM storage_purge_jobs WHERE state='pending'"))
    if target["name"] == "fizzy":
        with sqlite3.connect(target["queue_database"]) as db:
            return db.execute("SELECT COUNT(*) FROM solid_queue_jobs WHERE class_name='ActiveStorage::PurgeJob'").fetchone()[0]
    with sqlite3.connect(target["database"]) as db:
        return db.execute("SELECT COUNT(*) FROM storage_purge_jobs WHERE state='pending'").fetchone()[0]


def drain(target, key):
    if target["name"] == "fizzy":
        assert key.isalnum()
        script = f'ActiveStorage::PurgeJob.perform_now(ActiveStorage::Blob.find_by!(key: "{key}"))'
        result = subprocess.run(["bin/rails", "runner", script], cwd=SOURCE,
                                env=target["env"], capture_output=True, text=True)
        assert result.returncode == 0, result.stderr[-1500:]
    else:
        result = subprocess.run([RUST, "purge-stored-blobs", "--one"], cwd=ROOT,
                                env=target["env"], capture_output=True, text=True)
        assert result.returncode == 0 and "Purged 1 stored blobs" in result.stdout, (
            result.returncode, result.stdout, result.stderr[-1500:])


def run(target):
    prefix = f"/{target['account']}"
    status, _, raw = call(target, prefix + "/users")
    assert status == 200, (status, raw[:100])
    user = next(row for row in json.loads(raw) if row["role"] == "owner")
    first, first_derived = upload_avatar(target, user["id"])
    second, second_derived = upload_avatar(target, user["id"])
    assert old_blob_state(target, first) == (1, 1, 0), target["name"]
    assert old_blob_state(target, second) == (1, 1, 1), target["name"]
    assert pending_jobs(target) == 1, target["name"]
    if target.get("storage"):
        assert (target["storage"] / first["key"]).exists()
        assert (target["storage"] / first_derived[2]).exists()
    drain(target, first["key"])
    assert old_blob_state(target, first) == (0, 0, 0), target["name"]
    assert old_blob_state(target, second) == (1, 1, 1), target["name"]
    assert_original_removed(target, first)
    assert derived_rows(target, first_derived) == 1, target["name"]
    assert pending_jobs(target) == (2 if target["name"] == "fizzy" else 1), target["name"]
    drain(target, first_derived[2])
    assert derived_rows(target, first_derived) == 0, target["name"]
    assert_derived_removed(target, first_derived)
    if target["name"] == "rizzy":
        assert pending_jobs(target) == 0
    status, _, raw = call(target, prefix + f"/users/{user['id']}/avatar.json", "DELETE")
    assert status == 204, (status, raw[:100])
    assert old_blob_state(target, second) == (1, 1, 0), target["name"]
    assert pending_jobs(target) == (3 if target["name"] == "fizzy" else 1), (target["name"], pending_jobs(target))
    drain(target, second["key"])
    assert old_blob_state(target, second) == (0, 0, 0), target["name"]
    assert_original_removed(target, second)
    assert derived_rows(target, second_derived) == 1, target["name"]
    if target["name"] == "rizzy":
        assert pending_jobs(target) == 1
    drain(target, second_derived[2])
    assert derived_rows(target, second_derived) == 0, target["name"]
    assert_derived_removed(target, second_derived)
    if target["name"] == "rizzy":
        assert pending_jobs(target) == 0
    return {"old_blob_after_replace": (1, 1, 0), "old_blob_after_drain": (0, 0, 0),
            "deleted_blob_before_drain": (1, 1, 0), "deleted_blob_after_drain": (0, 0, 0)}


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    results = {}
    with tempfile.TemporaryDirectory(prefix="rizzy-avatar-purge-") as temp:
        root = Path(temp)
        servers = []
        try:
            for name, prepare in (("fizzy", prepare_source), ("rizzy", prepare_rust)):
                directory = root / name
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
                print(f"{name}: deferred avatar purge and drain passed", flush=True)
        finally:
            for process in reversed(servers):
                stop_server(process)
    assert results["fizzy"] == results["rizzy"], results


if __name__ == "__main__":
    main()
