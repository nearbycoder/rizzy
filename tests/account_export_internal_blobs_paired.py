#!/usr/bin/env python3
"""Compare pending and internal Active Storage account transfer behavior."""

import base64
import hashlib
import io
import json
import os
import sqlite3
import sys
import tempfile
import time
import urllib.request
import uuid
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from account_import import await_result as await_import, upload as upload_import  # noqa: E402
from card_image_parity import call  # noqa: E402
from embed_parity import upload  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             source_env, start_server, stop_server)


def source_id(value):
    return int(value, 36).to_bytes(16, "big")


def public_id(value):
    number = int.from_bytes(value, "big")
    digits = "0123456789abcdefghijklmnopqrstuvwxyz"
    result = ""
    while number:
        number, remainder = divmod(number, 36)
        result = digits[remainder] + result
    return result.rjust(25, "0")


def attach_internal(database, name, blob_id, record_type):
    table = "active_storage_attachments" if name == "fizzy" else "storage_attachments"
    blob_table = "active_storage_blobs" if name == "fizzy" else "storage_blobs"
    stored_blob_id = source_id(blob_id) if name == "fizzy" else blob_id
    with sqlite3.connect(database) as db:
        account_id, created_at = db.execute(
            f"SELECT account_id,created_at FROM {blob_table} WHERE id=?", (stored_blob_id,)
        ).fetchone()
        attachment_id = uuid.uuid4().bytes if name == "fizzy" else str(uuid.uuid4())
        record_id = uuid.uuid4().bytes if name == "fizzy" else str(uuid.uuid4())
        db.execute(
            f"INSERT INTO {table}(id,account_id,blob_id,created_at,name,record_id,record_type) "
            "VALUES(?,?,?,?,?,?,?)",
            (attachment_id, account_id, stored_blob_id, created_at, "file", record_id, record_type),
        )
    return public_id(attachment_id) if name == "fizzy" else attachment_id


def pending_direct_upload(target):
    content = b"unfinished direct upload"
    payload = {"blob": {"filename": "pending.txt", "byte_size": len(content),
                        "checksum": base64.b64encode(hashlib.md5(content).digest()).decode(),
                        "content_type": "text/plain"}}
    status, _, raw = call(target, f"/{target['account']}/rails/active_storage/direct_uploads",
                          "POST", payload)
    assert status == 200, (target["name"], "pending direct upload", status, raw[:160])
    return json.loads(raw)


def archive(target):
    account = target["account"]
    status, _, raw = call(target, f"/{account}/account/exports.json", "POST")
    assert status == 201, (target["name"], "create", status, raw[:160])
    export_id = json.loads(raw)["id"]
    for _ in range(240):
        status, _, raw = call(target, f"/{account}/account/exports/{export_id}.json")
        assert status == 200, (target["name"], "status", status)
        state = json.loads(raw)
        if state["status"] in ("completed", "failed"):
            break
        time.sleep(.25)
    assert state["status"] == "completed", (target["name"], state)
    request = urllib.request.Request(state["download_url"], headers={
        "Authorization": f"Bearer {target['token']}", "Accept": "application/zip"})
    with urllib.request.urlopen(request, timeout=45) as response:
        assert response.status == 200
        data = response.read()
    target["archive_bytes"] = data
    return zipfile.ZipFile(io.BytesIO(data))


def check(target):
    blobs = [upload(target) for _ in range(3)]
    ordinary, exported, imported = blobs
    pending = pending_direct_upload(target)
    target["ordinary_blob_id"] = ordinary["id"]
    target["pending_blob_id"] = pending["id"]
    exported_attachment = attach_internal(target["db"], target["name"], exported["id"], "Export")
    imported_attachment = attach_internal(target["db"], target["name"], imported["id"], "Account::Import")
    with archive(target) as zipped:
        names = set(zipped.namelist())
        assert f"data/active_storage_blobs/{ordinary['id']}.json" in names, target["name"]
        assert f"storage/{ordinary['key']}" in names, target["name"]
        assert f"data/active_storage_blobs/{pending['id']}.json" in names, target["name"]
        assert f"storage/{pending['key']}" not in names, target["name"]
        for blob, attachment in ((exported, exported_attachment), (imported, imported_attachment)):
            assert f"data/active_storage_blobs/{blob['id']}.json" not in names, target["name"]
            assert f"data/active_storage_attachments/{attachment}.json" not in names, target["name"]
            assert f"storage/{blob['key']}" not in names, target["name"]
    return {"ordinary_blob": True, "ordinary_file": True,
            "pending_blob": True, "pending_file": False,
            "excluded_internal_blobs": 2, "excluded_internal_attachments": 2,
            "excluded_internal_files": 2}


def import_source_archive(source, rust, directory):
    with zipfile.ZipFile(io.BytesIO(source["archive_bytes"])) as zipped:
        ordinary_metadata = json.loads(zipped.read(
            f"data/active_storage_blobs/{source['ordinary_blob_id']}.json"))
        pending_metadata = json.loads(zipped.read(
            f"data/active_storage_blobs/{source['pending_blob_id']}.json"))
        ordinary_bytes = zipped.read(f"storage/{ordinary_metadata['key']}")
    created = upload_import(rust["url"], rust["token"], source["archive_bytes"])
    result = await_import(rust["url"], rust["token"], created)
    assert result["status"] == "completed", result
    with sqlite3.connect(rust["db"]) as db:
        account_id = db.execute(
            "SELECT id FROM accounts WHERE external_account_id=?", (created["account"],)
        ).fetchone()[0]
        pending = db.execute(
            "SELECT storage_key,uploaded_at,byte_size,checksum FROM storage_blobs WHERE account_id=? AND id=?",
            (account_id, source["pending_blob_id"]),
        ).fetchone()
        ordinary = db.execute(
            "SELECT storage_key,uploaded_at FROM storage_blobs WHERE account_id=? AND id=?",
            (account_id, source["ordinary_blob_id"]),
        ).fetchone()
    assert pending and pending[1] is None, pending
    assert pending[2:] == (pending_metadata["byte_size"], pending_metadata["checksum"]), pending
    assert ordinary and ordinary[1], ordinary
    storage = directory / "storage"
    assert not (storage / pending[0]).exists()
    assert (storage / ordinary[0]).read_bytes() == ordinary_bytes


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-export-internal-paired-") as temporary:
        processes = []
        results = {}
        targets = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env = source_env(directory, 2)
                    env["SOLID_QUEUE_IN_PUMA"] = "true"
                    env, credentials = prepare_source(directory, 2, env)
                else:
                    env, credentials = prepare_rust(directory, rust_env(directory))
                database = directory / ("db.sqlite3" if name == "fizzy"
                                        else "rizzy.sqlite3")
                if name == "rizzy":
                    with sqlite3.connect(database) as db:
                        db.execute("DROP INDEX storage_attachments_blob_type")
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                if name == "rizzy":
                    with sqlite3.connect(database) as db:
                        indexes = {row[1] for row in db.execute(
                            "PRAGMA index_list(storage_attachments)")}
                    assert "storage_attachments_blob_type" in indexes
                target = {"name": name, "url": url,
                          "account": str(credentials["account"]),
                          "token": credentials["token"], "db": str(database)}
                targets[name] = target
                results[name] = check(target)
            assert results["fizzy"] == results["rizzy"], results
            import_source_archive(targets["fizzy"], targets["rizzy"], Path(temporary) / "rizzy")
            print("Matched Fizzy account ZIP pending/internal blobs and imported pending metadata")
        finally:
            for process in reversed(processes):
                stop_server(process)
            source_database = Path(temporary) / "fizzy" / "db.sqlite3"
            if source_database.exists():
                with sqlite3.connect(source_database) as db:
                    keys = [row[0] for row in db.execute("SELECT `key` FROM active_storage_blobs")]
                storage_root = ROOT / "upstream" / "storage" / "production" / "files"
                for key in keys:
                    if key.isalnum():
                        file = storage_root / key[:2] / key[2:4] / key
                        file.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
