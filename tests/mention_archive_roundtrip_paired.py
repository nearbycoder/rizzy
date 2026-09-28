#!/usr/bin/env python3
"""Round-trip card User mentions and file embeds through both account importers."""

import argparse
import hashlib
import http.cookiejar
import io
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from account_export_field_inventory import (FILE, drain_source_mention_job,  # noqa: E402
                                            export, fixture)
from account_import import await_result, upload  # noqa: E402
from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import (SOURCE, prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def blob_hashes(archive):
    with zipfile.ZipFile(io.BytesIO(archive)) as zipped:
        hashes = {}
        for path in zipped.namelist():
            if not path.startswith("data/active_storage_blobs/") or not path.endswith(".json"):
                continue
            blob = json.loads(zipped.read(path))
            hashes[blob["id"]] = hashlib.sha256(zipped.read(f"storage/{blob['key']}")).hexdigest()
        return hashes


def attachment_blobs(archive):
    with zipfile.ZipFile(io.BytesIO(archive)) as zipped:
        attachments = {}
        for path in zipped.namelist():
            if not path.startswith("data/active_storage_attachments/") or not path.endswith(".json"):
                continue
            attachment = json.loads(zipped.read(path))
            attachments[attachment["id"]] = attachment["blob_id"]
        return attachments


def verify_rust_import(target, created, expected_sha256, expected_blobs,
                       expected_attachments, expected_variants, image_filename):
    result = await_result(target["url"], target["token"], created)
    assert result["status"] == "completed", result
    with sqlite3.connect(target["db"]) as db:
        account = db.execute("SELECT id FROM accounts WHERE external_account_id=?",
                             (created["account"],)).fetchone()
        assert account, created
        account_id = account[0]
        card = db.execute("SELECT id,number,description FROM cards WHERE account_id=? AND title=?",
                          (account_id, "Record fields")).fetchone()
        assert card, account_id
        assert "gid://fizzy/" not in card[2], card[2]
        user = db.execute("SELECT user_id,sgid FROM user_attachables WHERE account_id=? AND user_id IN "
                          "(SELECT mentionee_id FROM mentions WHERE account_id=? AND source_type='Card' AND source_id=?)",
                          (account_id, account_id, card[0])).fetchone()
        assert user and user[1] in card[2], (user, card[2])
        assert db.execute("SELECT COUNT(*) FROM mentions WHERE account_id=? AND source_type='Card' AND source_id=?",
                          (account_id, card[0])).fetchone()[0] == 1
        blob = db.execute("SELECT storage_key,attachable_sgid FROM storage_blobs WHERE account_id=? AND filename=?",
                          (account_id, "notes.txt")).fetchone()
        assert blob and blob[1] in card[2], (blob, card[2])
        assert db.execute("SELECT COUNT(*) FROM storage_attachments WHERE account_id=? AND blob_id IN "
                          "(SELECT id FROM storage_blobs WHERE account_id=? AND filename=?)",
                          (account_id, account_id, "notes.txt")).fetchone()[0] == 1
        if image_filename:
            image = db.execute("SELECT id,attachable_sgid FROM storage_blobs WHERE account_id=? AND filename=?",
                               (account_id, image_filename)).fetchone()
            assert image and image[1] in card[2], (image, card[2])
            assert db.execute("SELECT COUNT(*) FROM storage_attachments WHERE account_id=? AND blob_id=?",
                              (account_id, image[0])).fetchone()[0] == 1
        variant_count = db.execute("SELECT COUNT(*) FROM storage_variant_records WHERE account_id=?",
                                   (account_id,)).fetchone()[0]
        assert variant_count == expected_variants, (variant_count, expected_variants)
        imported_blobs = dict(db.execute("SELECT id,storage_key FROM storage_blobs WHERE account_id=?",
                                         (account_id,)).fetchall())
        assert set(imported_blobs) == set(expected_blobs), (set(imported_blobs), set(expected_blobs))
        imported_attachments = dict(db.execute("SELECT id,blob_id FROM storage_attachments WHERE account_id=?",
                                               (account_id,)).fetchall())
        assert imported_attachments == expected_attachments, (imported_attachments, expected_attachments)
    for blob_id, expected_hash in expected_blobs.items():
        actual_hash = hashlib.sha256((Path(target["storage"]) / imported_blobs[blob_id]).read_bytes()).hexdigest()
        assert actual_hash == expected_hash, (blob_id, actual_hash, expected_hash)
    actual_sha256 = hashlib.sha256((Path(target["storage"]) / blob[0]).read_bytes()).hexdigest()
    assert actual_sha256 == expected_sha256, (actual_sha256, expected_sha256)
    target["opener"] = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
    login(target)
    status, _, rendered = request(target, f"/{created['account']}/cards/{card[1]}",
                                  accept="text/html")
    assert status == 200, (status, rendered[-1500:])
    assert b'<span class="mention">' in rendered and b"notes.txt" in rendered, rendered[-1500:]
    if image_filename:
        assert image_filename.encode() in rendered, rendered[-1500:]
    return {"cards": 1, "mentions": 1, "attachments": 2 + bool(image_filename),
            "file_sha256": actual_sha256, "blobs": len(expected_blobs),
            "variants": variant_count, "storage_attachments": len(imported_attachments),
            "image": bool(image_filename)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image-format", choices=("jpeg", "png", "gif", "webp", "svg", "bmp"))
    args = parser.parse_args()
    image_filenames = {"jpeg": "moon.jpg", "png": "avatar.png", "gif": "animated.gif",
                       "webp": "animated.webp", "svg": "avatar.svg", "bmp": "tiny.bmp"}
    image_filename = image_filenames.get(args.image_format)
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mention-archive-paired-") as temporary:
        directory = Path(temporary)
        processes = []
        targets = {}
        archives = {}
        variants = {}
        source_environment = None
        try:
            for name in ("fizzy", "rizzy"):
                app_directory = directory / name
                app_directory.mkdir()
                if name == "fizzy":
                    env, credentials = prepare_source(app_directory, 2)
                    source_environment = env
                else:
                    env = rust_env(app_directory)
                    env["RIZZY_SECRET_KEY_BASE"] = source_environment["SECRET_KEY_BASE"]
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                    env, credentials = prepare_rust(app_directory, env)
                process, url = start_server(name, app_directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url, "account": str(credentials["account"]),
                          "token": credentials["token"],
                          "db": str(app_directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                          "storage": env.get("RIZZY_STORAGE_DIR"), "email": "owner@example.com"}
                targets[name] = target
                fixture(target, source_environment, bool(image_filename), args.image_format or "jpeg")
                if name == "fizzy":
                    drain_source_mention_job(target, source_environment)
                exported = export(target, env)
                archives[name] = exported[7]
                variants[name] = len(exported[6])

            expected_sha256 = hashlib.sha256(FILE).hexdigest()
            rust_archive_path = directory / "rizzy-account.zip"
            rust_archive_path.write_bytes(archives["rizzy"])
            source_import_env = {**source_environment,
                                 "RIZZY_ACCOUNT_ARCHIVE": str(rust_archive_path),
                                 "EXPECTED_FILE_SHA256": expected_sha256,
                                 "EXPECTED_BLOB_SHA256": json.dumps(blob_hashes(archives["rizzy"])),
                                 "EXPECTED_ATTACHMENTS": json.dumps(attachment_blobs(archives["rizzy"])),
                                 "EXPECTED_VARIANT_COUNT": str(variants["rizzy"])}
            if image_filename:
                source_import_env["EXPECTED_IMAGE_FILENAME"] = image_filename
            source_import = subprocess.run(
                ["bin/rails", "runner", str(ROOT / "tests/import_mention_archive_with_fizzy.rb")],
                cwd=SOURCE, env=source_import_env, capture_output=True, text=True)
            assert source_import.returncode == 0, source_import.stderr[-3000:]
            source_result = json.loads(source_import.stdout.splitlines()[-1])

            created = upload(targets["rizzy"]["url"], targets["rizzy"]["token"], archives["fizzy"])
            rust_result = verify_rust_import(targets["rizzy"], created, expected_sha256,
                                             blob_hashes(archives["fizzy"]),
                                             attachment_blobs(archives["fizzy"]), variants["fizzy"],
                                             image_filename)
            assert source_result == rust_result, (source_result, rust_result)
            print(f"PASS cross-imported User mentions, files, and {args.image_format or 'no'} image preview")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
