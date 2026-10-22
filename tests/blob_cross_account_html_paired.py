#!/usr/bin/env python3
"""Compare unattached blob authorization and executable-media responses."""

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from card_image_parity import IMAGE, call  # noqa: E402
from embed_parity import upload  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)

HTML = b'<turbo-frame id="page"><script>throw 1</script></turbo-frame>'
DECLARED_TYPES = (
    "text/html;charset=utf-8",
    "text/html,foo",
    "text/x-html",
    "text/html+json",
    "text/vnd.turbo-stream.html",
    "application/xhtml+xml+xxe",
    "text/htmlé",
    "image/svg+xml",
    "application/xml",
    "text/xml",
    "TEXT/HTML",
)


def newest_account(database):
    with sqlite3.connect(database) as db:
        return str(db.execute("SELECT MAX(external_account_id) FROM accounts").fetchone()[0])


def declared_blob(target, declared, filename):
    blob = upload(target, HTML, filename, "text/plain")
    if target["name"] == "fizzy":
        table = "active_storage_blobs"
        blob_id = int(blob["id"], 36).to_bytes(16, "big")
    else:
        table = "storage_blobs"
        blob_id = blob["id"]
    with sqlite3.connect(target["db"]) as db:
        updated = db.execute(f"UPDATE {table} SET content_type=? WHERE id=?",
                             (declared, blob_id))
        assert updated.rowcount == 1, (target["name"], blob["id"])
    return blob


def check(target):
    account = target["account"]
    for declared in DECLARED_TYPES:
        blob = declared_blob(target, declared, "video-exfil.html")
        path = f"/rails/active_storage/blobs/proxy/{blob['signed_id']}/video-exfil.html"
        status, headers, body = call(target, f"/{account}{path}")
        assert status == 200, (target["name"], declared, status, body[:100])
        assert headers.get_content_type() == "application/octet-stream", (
            target["name"], declared, headers.get("Content-Type"))
        assert headers.get("Content-Disposition", "").startswith("attachment"), (
            target["name"], declared, headers.get("Content-Disposition"))
        assert body == HTML, (target["name"], declared, body[:100])

    blob = declared_blob(target, "text/html;charset=utf-8", "cross-account.html")
    public_blob = upload(target)
    status, _, raw = call(target, f"/{account}/boards", "POST",
                          {"board": {"name": "Published blob"}})
    assert status == 201, (target["name"], status, raw[:150])
    board = json.loads(raw)
    markup = f'<action-text-attachment sgid="{public_blob["attachable_sgid"]}"></action-text-attachment>'
    status, _, raw = call(target, f"/{account}/boards/{board['id']}/cards", "POST",
                          {"card": {"title": "Public embed", "description": markup}})
    assert status == 201, (target["name"], status, raw[:150])
    status, _, raw = call(target, f"/{account}/boards/{board['id']}/publication", "POST")
    assert status == 201, (target["name"], status, raw[:150])
    status, _, body = call(target, "/signup/completion.json", "POST",
                           {"signup": {"full_name": "Second Owner"}})
    assert status == 201, (target["name"], status, body[:150])
    second = newest_account(target["db"])
    assert second != account, (target["name"], second, account)
    path = (f"/{second}/rails/active_storage/blobs/proxy/"
            f"{blob['signed_id']}/cross-account.html")
    status, _, body = call(target, path)
    anonymous_status, _, _ = call(target, path, bearer=False)
    public_path = (f"/{second}/rails/active_storage/blobs/proxy/"
                   f"{public_blob['signed_id']}/moon.jpg")
    public_status, public_headers, public_body = call(target, public_path, bearer=False)
    assert public_status == 200 and public_headers.get_content_type() == "image/jpeg"
    assert public_body == IMAGE, (target["name"], public_status, public_body[:20])
    return status, anonymous_status, body[:100], public_status


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-blob-cross-account-") as temporary:
        processes = []
        try:
            results = {}
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env, credentials = prepare_source(directory, 2)
                    database = directory / "db.sqlite3"
                else:
                    env = rust_env(directory)
                    env["RIZZY_MULTI_TENANT"] = "1"
                    env, credentials = prepare_rust(directory, env)
                    database = directory / "rizzy.sqlite3"
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url, "account": str(credentials["account"]),
                          "token": credentials["token"], "db": str(database)}
                results[name] = check(target)
            assert results["fizzy"][0] == 403, results
            assert results["fizzy"][1] == 302, results
            assert results["fizzy"][3] == 200, results
            assert results["rizzy"] == results["fizzy"], results
            print("PASS dangerous unattached blobs and cross-account authorization", results)
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
