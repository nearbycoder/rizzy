#!/usr/bin/env python3
"""Import a real Fizzy account ZIP into a disposable Rizzy database."""
import argparse
import io
import json
import os
import re
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import zipfile
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def call(url, token, path, method="GET", data=None, content_type=None):
    headers = {"Authorization": f"Bearer {token}", "Accept": "application/json"}
    if content_type:
        headers["Content-Type"] = content_type
    request = urllib.request.Request(url + path, headers=headers, data=data, method=method)
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            body = response.read()
            return response.status, json.loads(body) if body else None
    except urllib.error.HTTPError as error:
        body = error.read()
        return error.code, json.loads(body) if body and body.startswith(b"{") else body


def upload(url, token, archive):
    boundary = "rizzy-account-import-test"
    body = (f"--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"account.zip\"\r\n"
            "Content-Type: application/zip\r\n\r\n").encode() + archive + f"\r\n--{boundary}--\r\n".encode()
    status, result = call(url, token, "/account/imports", "POST", body,
                          f"multipart/form-data; boundary={boundary}")
    assert status == 201 and result["status"] == "pending", (status, result)
    return result


def cross_account_reference(archive, foreign_user):
    result = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(archive)) as source, zipfile.ZipFile(result, "w") as target:
        card_path = next(name for name in source.namelist() if name.startswith("data/cards/"))
        for name in source.namelist():
            data = source.read(name)
            if name == card_path:
                card = json.loads(data)
                card["creator_id"] = foreign_user
                data = json.dumps(card).encode()
            target.writestr(name, data)
    return result.getvalue()


def duplicate_publication_key(archive):
    result = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(archive)) as source, zipfile.ZipFile(result, "w") as target:
        publication_path = next((name for name in source.namelist()
                                 if name.startswith("data/board_publications/")), None)
        if publication_path is None:
            return None
        for name in source.namelist():
            target.writestr(name, source.read(name))
        publication = json.loads(source.read(publication_path))
        publication["id"] = "duplicate-publication-key-probe"
        target.writestr(f"data/board_publications/{publication['id']}.json",
                        json.dumps(publication))
    return result.getvalue()


def await_result(url, token, created):
    path = f"/{created['account']}/account/imports/{created['id']}"
    for _ in range(300):
        status, result = call(url, token, path)
        assert status == 200, (status, result)
        if result["status"] in ("completed", "failed"):
            return result
        time.sleep(.1)
    raise AssertionError("import did not finish")


def run(archive_path):
    archive = archive_path.read_bytes()
    with zipfile.ZipFile(archive_path) as z:
        account = json.loads(z.read("data/account.json"))
        users = [json.loads(z.read(name)) for name in z.namelist() if name.startswith("data/users/")]
        owner = next(user for user in users if user["role"] == "owner")
        counts = {group: sum(name.startswith(f"data/{group}/") for name in z.namelist())
                  for group in ("boards", "columns", "tags", "cards", "comments", "steps",
                                "assignments", "taggings", "closures", "card_goldnesses",
                                "card_not_nows", "card_activity_spikes", "watches", "pins",
                                "reactions", "mentions", "events", "notifications",
                                "notification_bundles", "webhooks", "board_publications",
                                "active_storage_blobs", "active_storage_variant_records",
                                "active_storage_attachments")}
        blob_metadata = [json.loads(z.read(name)) for name in z.namelist()
                         if name.startswith("data/active_storage_blobs/")]
        bundles = [json.loads(z.read(name)) for name in z.namelist()
                   if name.startswith("data/notification_bundles/")]
        rich_texts = [json.loads(z.read(name)) for name in z.namelist()
                      if name.startswith("data/action_text_rich_texts/")]
        user_gid = f'gid="gid://fizzy/User/{owner["id"]}"'
        has_user_mention = any(user_gid in rich.get("body", "") for rich in rich_texts)
        original_bytes = {blob["id"]: z.read(f"storage/{blob['key']}") for blob in blob_metadata}
        dated_cards = [json.loads(z.read(name)) for name in z.namelist()
                       if name.startswith("data/cards/") and json.loads(z.read(name)).get("due_on")]
        postponed_records = [json.loads(z.read(name)) for name in z.namelist()
                             if name.startswith("data/card_not_nows/")]
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "rizzy.sqlite3"
        storage = Path(temp) / "storage"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}",
               "RIZZY_IMPORT_DIR": str(Path(temp) / "imports"),
               "RIZZY_STORAGE_DIR": str(storage),
               "RIZZY_SEED_EMAIL": owner["email_address"],
               "RIZZY_SEED_NAME": owner["name"],
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
        seed = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True,
                              text=True, check=True).stdout
        seed_account = int(re.search(r"Account: (\d+)", seed).group(1))
        token = re.search(r"Bearer token: (\w+)", seed).group(1)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        url = f"http://127.0.0.1:{port}"
        server = subprocess.Popen([BINARY], cwd=ROOT, env={**env, "RIZZY_BIND": f"127.0.0.1:{port}"},
                                  stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            for _ in range(100):
                try:
                    with urllib.request.urlopen(url + "/up", timeout=1):
                        break
                except OSError:
                    time.sleep(.05)
            else:
                raise RuntimeError("Rizzy server did not start")
            created = upload(url, token, archive)
            result = await_result(url, token, created)
            assert result["status"] == "completed", result
            with sqlite3.connect(database) as db:
                imported_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?",
                                         (created["account"],)).fetchone()[0]
                assert db.execute("SELECT name FROM accounts WHERE id=?", (imported_id,)).fetchone()[0] == account["name"]
                queries = {
                    "comments": "SELECT COUNT(*) FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.account_id=?",
                    "steps": "SELECT COUNT(*) FROM steps s JOIN cards c ON c.id=s.card_id WHERE c.account_id=?",
                    "taggings": "SELECT COUNT(*) FROM taggings t JOIN cards c ON c.id=t.card_id WHERE c.account_id=?",
                    "closures": "SELECT COUNT(*) FROM closures z JOIN cards c ON c.id=z.card_id WHERE c.account_id=?",
                    "card_goldnesses": "SELECT COUNT(*) FROM cards WHERE account_id=? AND golden=1",
                    "card_not_nows": "SELECT COUNT(*) FROM card_not_nows WHERE account_id=?",
                    "watches": "SELECT COUNT(*) FROM watches w JOIN cards c ON c.id=w.card_id WHERE c.account_id=?",
                    "board_publications": "SELECT COUNT(*) FROM boards WHERE account_id=? AND publication_key IS NOT NULL",
                    "active_storage_blobs": "SELECT COUNT(*) FROM storage_blobs WHERE account_id=?",
                    "active_storage_variant_records": "SELECT COUNT(*) FROM storage_variant_records WHERE account_id=?",
                    "active_storage_attachments": "SELECT COUNT(*) FROM storage_attachments WHERE account_id=?",
                }
                for group, expected in counts.items():
                    table = {"active_storage_blobs": "storage_blobs", "active_storage_attachments": "storage_attachments"}.get(group, group)
                    query = queries.get(group, f"SELECT COUNT(*) FROM {table} WHERE account_id=?")
                    actual = db.execute(query, (imported_id,)).fetchone()[0]
                    assert actual == expected, (group, actual, expected)
                assert db.execute("SELECT COUNT(*) FROM users WHERE account_id=? AND role='owner' AND id=?",
                                  (imported_id, owner["id"])).fetchone()[0] == 1
                assert db.execute("SELECT COUNT(*) FROM cards WHERE account_id=? AND description!=''",
                                  (imported_id,)).fetchone()[0] == counts["cards"]
                assert db.execute("SELECT COUNT(*) FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.account_id=? AND m.body!=''",
                                  (imported_id,)).fetchone()[0] == counts["comments"]
                for blob_id, source_bytes in original_bytes.items():
                    key = db.execute("SELECT storage_key FROM storage_blobs WHERE id=? AND account_id=?",
                                     (blob_id, imported_id)).fetchone()[0]
                    assert (storage / key).read_bytes() == source_bytes
                for blob in blob_metadata:
                    actual = json.loads(db.execute("SELECT metadata FROM storage_blobs WHERE id=?",
                                                   (blob["id"],)).fetchone()[0])
                    assert actual == blob["metadata"]
                for card in dated_cards:
                    due_on = db.execute("SELECT due_on FROM cards WHERE id=?", (card["id"],)).fetchone()[0]
                    assert due_on == card["due_on"]
                for not_now in postponed_records:
                    actual = db.execute("SELECT card_id,user_id,created_at,updated_at FROM card_not_nows WHERE id=?",
                                        (not_now["id"],)).fetchone()
                    assert actual == (not_now["card_id"], not_now.get("user_id"),
                                      not_now["created_at"], not_now["updated_at"])
                for bundle in bundles:
                    status_value = db.execute("SELECT status FROM notification_bundles WHERE id=?", (bundle["id"],)).fetchone()[0]
                    expected = {"pending": 0, "processing": 1, "delivered": 2}.get(bundle["status"], bundle["status"])
                    assert status_value == expected
                if original_bytes:
                    description = db.execute("SELECT description FROM cards WHERE account_id=? LIMIT 1",
                                             (imported_id,)).fetchone()[0]
                    assert 'sgid="' in description and ' gid="' not in description
                    if has_user_mention:
                        user_sgid = db.execute("SELECT sgid FROM user_attachables WHERE user_id=?", (owner["id"],)).fetchone()[0]
                        assert f'sgid="{user_sgid}"' in description
                    card_number = db.execute("SELECT number FROM cards WHERE account_id=? LIMIT 1",
                                             (imported_id,)).fetchone()[0]
                    status, card = call(url, token, f"/{created['account']}/cards/{card_number}")
                    assert status == 200 and card["has_attachments"] and card["image_url"]
                    if has_user_mention:
                        assert owner["name"].split()[0] in card["description"]
                        assert 'class="mention"' in card["description_html"]
            status, boards = call(url, token, f"/{created['account']}/boards")
            assert status == 200 and len(boards) == counts["boards"], (status, boards)
            if counts["notifications"]:
                status, notices = call(url, token, f"/{created['account']}/notifications")
                assert status == 200 and any(notice["source_type"] == "mention"
                                             and notice["title"] == f"{owner['name'].split()[0]} @mentioned you"
                                             and "Hello world" in notice["body"] for notice in notices), notices
                if has_user_mention:
                    assert any(owner["name"].split()[0] in notice["body"]
                               for notice in notices if notice["source_type"] == "mention"), notices
            status, export = call(url, token, f"/{created['account']}/account/exports", "POST")
            assert status == 201 and export["status"] == "pending", (status, export)
            for _ in range(300):
                status, export_state = call(url, token, f"/{created['account']}/account/exports/{export['id']}")
                assert status == 200
                if export_state["status"] in ("completed", "failed"):
                    break
                time.sleep(.1)
            assert export_state["status"] == "completed", export_state
            download = urllib.request.Request(export_state["download_url"],
                                              headers={"Authorization": f"Bearer {token}", "Accept": "application/zip"})
            with urllib.request.urlopen(download, timeout=60) as response:
                reexport_bytes = response.read()
            if output := os.environ.get("RIZZY_IMPORT_REEXPORT_OUTPUT"):
                Path(output).write_bytes(reexport_bytes)
            with zipfile.ZipFile(io.BytesIO(reexport_bytes)) as z:
                names = z.namelist()
                for group, expected in counts.items():
                    actual = sum(name.startswith(f"data/{group}/") for name in names)
                    assert actual == expected, ("reexport", group, actual, expected)
                for blob_id, source_bytes in original_bytes.items():
                    blob = json.loads(z.read(f"data/active_storage_blobs/{blob_id}.json"))
                    assert z.read(f"storage/{blob['key']}") == source_bytes
                for source_blob in blob_metadata:
                    blob = json.loads(z.read(f"data/active_storage_blobs/{source_blob['id']}.json"))
                    assert blob["metadata"] == source_blob["metadata"]
                for source_card in dated_cards:
                    card = json.loads(z.read(f"data/cards/{source_card['id']}.json"))
                    assert card["due_on"] == source_card["due_on"]
                for source_not_now in postponed_records:
                    restored = json.loads(z.read(f"data/card_not_nows/{source_not_now['id']}.json"))
                    assert {key: restored[key] for key in ("card_id", "user_id", "created_at", "updated_at")} == {
                        key: source_not_now[key] for key in ("card_id", "user_id", "created_at", "updated_at")}
                for source_bundle in bundles:
                    bundle = json.loads(z.read(f"data/notification_bundles/{source_bundle['id']}.json"))
                    expected = source_bundle["status"] if isinstance(source_bundle["status"], str) else ("pending", "processing", "delivered")[source_bundle["status"]]
                    assert bundle["status"] == expected
                if has_user_mention:
                    assert any(user_gid in json.loads(z.read(name)).get("body", "")
                               for name in names if name.startswith("data/action_text_rich_texts/"))
            duplicate = upload(url, token, archive)
            conflict = await_result(url, token, duplicate)
            assert conflict["status"] == "failed" and conflict["failure_reason"] == "conflict", conflict
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM cards").fetchone()[0] == counts["cards"]
                foreign_user = db.execute("SELECT u.id FROM users u JOIN accounts a ON a.id=u.account_id WHERE a.external_account_id=? AND u.role='owner'",
                                          (seed_account,)).fetchone()[0]
            malicious = upload(url, token, cross_account_reference(archive, foreign_user))
            rejected = await_result(url, token, malicious)
            assert rejected["status"] == "failed" and rejected["failure_reason"] == "invalid_export", rejected
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM cards").fetchone()[0] == counts["cards"]
            if tampered := duplicate_publication_key(archive):
                duplicate_key = upload(url, token, tampered)
                denied = await_result(url, token, duplicate_key)
                assert denied["status"] == "failed" and denied["failure_reason"] == "conflict", denied
            invalid = upload(url, token, b"not a ZIP")
            bad_result = await_result(url, token, invalid)
            assert bad_result["status"] == "failed" and bad_result["failure_reason"] == "invalid_export", bad_result
            server.terminate()
            server.wait(timeout=5)
            with sqlite3.connect(database) as db:
                stale = (datetime.now(timezone.utc) - timedelta(minutes=31)).strftime("%Y-%m-%dT%H:%M:%S.000Z")
                db.execute("UPDATE imports SET status='processing',failure_reason=NULL,updated_at=? WHERE id=?",
                           (stale, invalid["id"]))
            server = subprocess.Popen([BINARY], cwd=ROOT, env={**env, "RIZZY_BIND": f"127.0.0.1:{port}"},
                                      stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            for _ in range(100):
                try:
                    with urllib.request.urlopen(url + "/up", timeout=1):
                        break
                except OSError:
                    time.sleep(.05)
            else:
                raise RuntimeError("Rizzy server did not restart")
            recovered = await_result(url, token, invalid)
            assert recovered["status"] == "failed" and recovered["failure_reason"] == "invalid_export", recovered
            server.terminate()
            server.wait(timeout=5)
            with sqlite3.connect(database) as db:
                expired = (datetime.now(timezone.utc) - timedelta(days=8)).strftime("%Y-%m-%dT%H:%M:%S.000Z")
                db.execute("UPDATE imports SET created_at=? WHERE id=?", (expired, invalid["id"]))
            server = subprocess.Popen([BINARY], cwd=ROOT, env={**env, "RIZZY_BIND": f"127.0.0.1:{port}"},
                                      stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            for _ in range(100):
                with sqlite3.connect(database) as db:
                    exists = db.execute("SELECT COUNT(*) FROM imports WHERE id=?", (invalid["id"],)).fetchone()[0]
                if not exists:
                    break
                time.sleep(.05)
            assert not exists and not (Path(env["RIZZY_IMPORT_DIR"]) / f"{invalid['id']}.zip").exists()
            print(f"Account import passed: {counts}; duplicate IDs rejected atomically")
        finally:
            server.terminate()
            server.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("archive", nargs="?", type=Path, help="Fizzy account export ZIP; omitted to generate a Rizzy export fixture")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as temp:
        if args.archive:
            archive_path = args.archive
        else:
            archive_path = Path(temp) / "account.zip"
            subprocess.run([sys.executable, str(ROOT / "tests" / "account_transfer.py")],
                           cwd=ROOT, check=True,
                           env={**os.environ, "RIZZY_TRANSFER_TEST_OUTPUT": str(archive_path)})
        run(archive_path)


if __name__ == "__main__":
    main()
