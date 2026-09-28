#!/usr/bin/env python3
"""Round-trip card User mentions and file embeds through both account importers."""

import hashlib
import http.cookiejar
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from account_export_field_inventory import (FILE, drain_source_mention_job,  # noqa: E402
                                            export, fixture)
from account_import import await_result, upload  # noqa: E402
from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import (SOURCE, prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def verify_rust_import(target, created, expected_sha256):
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
    actual_sha256 = hashlib.sha256((Path(target["storage"]) / blob[0]).read_bytes()).hexdigest()
    assert actual_sha256 == expected_sha256, (actual_sha256, expected_sha256)
    target["opener"] = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
    login(target)
    status, _, rendered = request(target, f"/{created['account']}/cards/{card[1]}",
                                  accept="text/html")
    assert status == 200, (status, rendered[-1500:])
    assert b'<span class="mention">' in rendered and b"notes.txt" in rendered, rendered[-1500:]
    return {"cards": 1, "mentions": 1, "attachments": 2, "file_sha256": actual_sha256}


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mention-archive-paired-") as temporary:
        directory = Path(temporary)
        processes = []
        targets = {}
        archives = {}
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
                fixture(target, source_environment)
                if name == "fizzy":
                    drain_source_mention_job(target, source_environment)
                archives[name] = export(target, env)[7]

            expected_sha256 = hashlib.sha256(FILE).hexdigest()
            rust_archive_path = directory / "rizzy-account.zip"
            rust_archive_path.write_bytes(archives["rizzy"])
            source_import_env = {**source_environment,
                                 "RIZZY_ACCOUNT_ARCHIVE": str(rust_archive_path),
                                 "EXPECTED_FILE_SHA256": expected_sha256}
            source_import = subprocess.run(
                ["bin/rails", "runner", str(ROOT / "tests/import_mention_archive_with_fizzy.rb")],
                cwd=SOURCE, env=source_import_env, capture_output=True, text=True)
            assert source_import.returncode == 0, source_import.stderr[-3000:]
            source_result = json.loads(source_import.stdout.splitlines()[-1])

            created = upload(targets["rizzy"]["url"], targets["rizzy"]["token"], archives["fizzy"])
            rust_result = verify_rust_import(targets["rizzy"], created, expected_sha256)
            assert source_result == rust_result, (source_result, rust_result)
            print("PASS cross-imported User mentions and file attachments with matching bytes")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
