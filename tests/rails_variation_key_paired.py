#!/usr/bin/env python3
"""Render source-signed Active Storage variations through both apps."""

import argparse
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

from card_image_parity import call  # noqa: E402
from embed_parity import upload  # noqa: E402
from storage_representation_parity import IMAGES  # noqa: E402
from sustained_mixed import (SOURCE, prepare_rust, prepare_source, rust_env,  # noqa: E402
                             source_env, start_server, stop_server)


def source_variations(env):
    code = '''require "json"
      puts JSON.generate({
        limit: ActiveStorage::Variation.encode(loader: { n: -1 }, resize_to_limit: [ 100, 100 ]),
        fill: ActiveStorage::Variation.encode(resize_to_fill: [ 64, 64 ]),
        png: ActiveStorage::Variation.encode(resize_to_limit: [ 100, 100 ], format: :png)
      })'''
    result = subprocess.run(["bin/rails", "runner", code], cwd=SOURCE, env=env,
                            capture_output=True, text=True, check=True)
    return json.loads(result.stdout)


def variants(target, original_key):
    source = target["name"] == "fizzy"
    blobs = "active_storage_blobs" if source else "storage_blobs"
    records = "active_storage_variant_records" if source else "storage_variant_records"
    attachments = "active_storage_attachments" if source else "storage_attachments"
    key = "key" if source else "storage_key"
    with sqlite3.connect(target["db"]) as db:
        rows = db.execute(
            f"SELECT v.variation_digest,derived.byte_size,derived.checksum,derived.metadata,"
            f"derived.content_type,derived.filename "
            f"FROM {blobs} original JOIN {records} v ON v.blob_id=original.id "
            f"JOIN {attachments} a ON a.record_type='ActiveStorage::VariantRecord' "
            f"AND a.record_id=v.id AND a.name='image' "
            f"JOIN {blobs} derived ON derived.id=a.blob_id "
            f"WHERE original.{key}=?", (original_key,)).fetchall()
    return {digest: (size, checksum, json.loads(metadata), content_type, filename)
            for digest, size, checksum, metadata, content_type, filename in rows}


def check(target, keys, image_format):
    original, filename, content_type, magic, _ = IMAGES[image_format]
    account = target["account"]
    status, _, raw = call(target, f"/{account}/boards", "POST",
                          {"board": {"name": "Source variation"}})
    assert status == 201, (target["name"], status, raw[:120])
    board = json.loads(raw)
    blob = upload(target, original, filename, content_type)
    markup = f'<action-text-attachment sgid="{blob["attachable_sgid"]}"></action-text-attachment>'
    status, _, raw = call(target, f"/{account}/boards/{board['id']}/cards", "POST",
                          {"card": {"title": "Variant card", "description": markup}})
    assert status == 201, (target["name"], status, raw[:120])
    before = variants(target, blob["key"])
    responses = {}
    for name, key in keys.items():
        path = (f"/{account}/rails/active_storage/representations/proxy/"
                f"{blob['signed_id']}/{key}/{filename}")
        status, headers, body = call(target, path)
        assert status == 200, (target["name"], name, status, body[:120])
        expected_type = "image/png" if name == "png" else content_type
        assert headers.get_content_type() == expected_type, (
            target["name"], name, headers.get("Content-Type"))
        assert body != original and body.startswith(b"\x89PNG" if name == "png" else magic)
        if dump_dir := os.environ.get("RIZZY_VARIATION_DUMP_DIR"):
            directory = Path(dump_dir)
            directory.mkdir(parents=True, exist_ok=True)
            directory.joinpath(f"{target['name']}-{image_format}-{name}.bin").write_bytes(body)
        responses[name] = {"sha256": hashlib.sha256(body).hexdigest(),
                           "bytes": len(body), "content_type": expected_type}
        tampered = path.replace(key, key[:-1] + ("a" if key[-1] != "a" else "b"))
        assert call(target, tampered)[0] == 404, (target["name"], name, "tampered")
    after = variants(target, blob["key"])
    return {"responses": responses,
            "variants": {digest: value for digest, value in after.items() if digest not in before}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image-format", choices=IMAGES, default="jpeg")
    args = parser.parse_args()
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-rails-variation-") as temporary:
        processes = []
        try:
            source_dir = Path(temporary) / "fizzy"
            source_dir.mkdir()
            source_settings = source_env(source_dir, 2)
            source_settings, source_credentials = prepare_source(source_dir, 2, source_settings)
            keys = source_variations(source_settings)
            results = {}
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                if name == "fizzy":
                    env, credentials = source_settings, source_credentials
                else:
                    directory.mkdir()
                    env = rust_env(directory)
                    env["RIZZY_SECRET_KEY_BASE"] = source_settings["SECRET_KEY_BASE"]
                    env, credentials = prepare_rust(directory, env)
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url, "account": str(credentials["account"]),
                          "token": credentials["token"],
                          "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3"))}
                results[name] = check(target, keys, args.image_format)
            assert results["fizzy"] == results["rizzy"], results
            print(f"PASS {args.image_format} Rails-signed variations match bytes and records")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
