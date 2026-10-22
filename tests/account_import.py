#!/usr/bin/env python3
"""Import a real Fizzy account ZIP into a disposable Rizzy database."""
import argparse
from collections import defaultdict
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
FRESH_UPDATE_TABLES = {
    "user_settings": "user_settings_records",
    "tags": "tags",
    "taggings": "taggings",
    "pins": "pins",
    "reactions": "reactions",
    "boards": "boards",
    "columns": "columns",
    "board_publications": "board_publications",
    "webhooks": "webhooks",
    "webhook_delinquency_trackers": "webhook_delinquency_trackers",
    "accesses": "access_records",
    "cards": "cards",
    "comments": "comments",
    "steps": "steps",
    "assignments": "assignments",
    "closures": "closures",
    "card_goldnesses": "card_goldnesses",
    "card_not_nows": "card_not_nows",
    "card_activity_spikes": "card_activity_spikes",
    "watches": "watches",
    "mentions": "mentions",
    "filters": "filters",
    "events": "events",
    "notifications": "notifications",
    "notification_bundles": "notification_bundles",
    "webhook_deliveries": "webhook_deliveries",
}


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


def assert_fresh(updated_at, import_started):
    updated = datetime.fromisoformat(updated_at.replace("Z", "+00:00"))
    assert import_started <= updated <= datetime.now(timezone.utc), updated_at


def assert_generic_reexport(restored, source, keys, import_started):
    assert {key: restored[key] for key in keys} == {key: source[key] for key in keys}
    assert_fresh(restored["updated_at"], import_started)


def run(archive_path):
    archive = archive_path.read_bytes()
    with zipfile.ZipFile(archive_path) as z:
        account = json.loads(z.read("data/account.json"))
        users = [json.loads(z.read(name)) for name in z.namelist() if name.startswith("data/users/")]
        owner = next(user for user in users if user["role"] == "owner")
        counts = {group: sum(name.startswith(f"data/{group}/") for name in z.namelist())
                  for group in ("user_settings", "boards", "columns", "tags", "accesses", "entropies", "cards", "comments", "steps",
                                "assignments", "taggings", "closures", "card_goldnesses",
                                "card_not_nows", "card_activity_spikes", "watches", "pins",
                                "reactions", "mentions", "events", "notifications",
                                "notification_bundles", "webhooks", "webhook_delinquency_trackers",
                                "board_publications",
                                "active_storage_blobs", "active_storage_variant_records",
                                "active_storage_attachments")}
        blob_metadata = [json.loads(z.read(name)) for name in z.namelist()
                         if name.startswith("data/active_storage_blobs/")]
        bundles = [json.loads(z.read(name)) for name in z.namelist()
                   if name.startswith("data/notification_bundles/")]
        owner_notices = [json.loads(z.read(name)) for name in z.namelist()
                         if name.startswith("data/notifications/")
                         and json.loads(z.read(name))["user_id"] == owner["id"]]
        rich_texts = [json.loads(z.read(name)) for name in z.namelist()
                      if name.startswith("data/action_text_rich_texts/")]
        described_cards = sum(rich.get("record_type") == "Card" and
                              rich.get("name") == "description" and bool(rich.get("body"))
                              for rich in rich_texts)
        described_comments = sum(rich.get("record_type") == "Comment" and
                                 rich.get("name") == "body" and bool(rich.get("body"))
                                 for rich in rich_texts)
        user_gid = f'gid="gid://fizzy/User/{owner["id"]}"'
        has_user_mention = any(user_gid in (rich.get("body") or "") for rich in rich_texts)
        original_bytes = {blob["id"]: z.read(f"storage/{blob['key']}") for blob in blob_metadata}
        cards_by_id = {card["id"]: card for card in
                       (json.loads(z.read(name)) for name in z.namelist()
                        if name.startswith("data/cards/"))}
        comments_by_id = {comment["id"]: comment for comment in
                          (json.loads(z.read(name)) for name in z.namelist()
                           if name.startswith("data/comments/"))}
        rich_by_id = {rich["id"]: rich for rich in rich_texts}
        expected_board_bytes = defaultdict(int)
        for name in z.namelist():
            if not name.startswith("data/active_storage_attachments/"):
                continue
            attachment = json.loads(z.read(name))
            kind, record_id = attachment["record_type"], attachment["record_id"]
            if kind == "ActionText::RichText":
                rich = rich_by_id[record_id]
                kind, record_id = rich["record_type"], rich["record_id"]
            if kind == "Card":
                board_id = cards_by_id[record_id]["board_id"]
            elif kind == "Comment":
                board_id = cards_by_id[comments_by_id[record_id]["card_id"]]["board_id"]
            elif kind == "Board":
                board_id = record_id
            else:
                continue
            expected_board_bytes[board_id] += len(original_bytes[attachment["blob_id"]])
        dated_cards = [json.loads(z.read(name)) for name in z.namelist()
                       if name.startswith("data/cards/") and json.loads(z.read(name)).get("due_on")]
        golden_records = [json.loads(z.read(name)) for name in z.namelist()
                          if name.startswith("data/card_goldnesses/")]
        closure_records = [json.loads(z.read(name)) for name in z.namelist()
                           if name.startswith("data/closures/")]
        access_records = [json.loads(z.read(name)) for name in z.namelist()
                          if name.startswith("data/accesses/")]
        setting_records = [json.loads(z.read(name)) for name in z.namelist()
                           if name.startswith("data/user_settings/")]
        entropy_records = [json.loads(z.read(name)) for name in z.namelist()
                           if name.startswith("data/entropies/")]
        publication_records = [json.loads(z.read(name)) for name in z.namelist()
                               if name.startswith("data/board_publications/")]
        spike_records = [json.loads(z.read(name)) for name in z.namelist()
                         if name.startswith("data/card_activity_spikes/")]
        watch_records = [json.loads(z.read(name)) for name in z.namelist()
                         if name.startswith("data/watches/")]
        postponed_records = [json.loads(z.read(name)) for name in z.namelist()
                             if name.startswith("data/card_not_nows/")]
        imported_samples = {
            group: next((json.loads(z.read(name)) for name in z.namelist()
                         if name.startswith(f"data/{group}/")), None)
            for group in FRESH_UPDATE_TABLES
        }
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
            import_started = datetime.now(timezone.utc)
            created = upload(url, token, archive)
            result = await_result(url, token, created)
            if result["status"] != "completed":
                server.terminate()
                server.wait(timeout=5)
                raise AssertionError((result, server.stderr.read().decode(errors="replace")[-3000:]))
            assert result["status"] == "completed", result
            with sqlite3.connect(database) as db:
                imported_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?",
                                         (created["account"],)).fetchone()[0]
                imported_account = db.execute(
                    "SELECT name,created_at,updated_at FROM accounts WHERE id=?", (imported_id,)
                ).fetchone()
                assert imported_account[0] == account["name"]
                ledger = dict(db.execute(
                    "SELECT board_id,SUM(delta) FROM storage_entries WHERE account_id=? "
                    "GROUP BY board_id", (imported_id,)).fetchall())
                assert ledger == {board: size for board, size in expected_board_bytes.items()
                                  if size}, (ledger, expected_board_bytes)
                assert db.execute(
                    "SELECT COUNT(*) FROM storage_entries WHERE account_id=? AND operation!='reconcile'",
                    (imported_id,)).fetchone()[0] == 0
                account_total = db.execute(
                    "SELECT bytes_stored,last_entry_id FROM storage_totals "
                    "WHERE owner_type='Account' AND owner_id=?", (imported_id,)).fetchone()
                assert account_total and account_total[0] == sum(expected_board_bytes.values())
                assert bool(account_total[1]) == bool(expected_board_bytes), account_total
                if account["name"] != "Import" or account.get("cards_count", 0) != 0:
                    assert_fresh(imported_account[2], import_started)
                for user in users:
                    actual = db.execute("SELECT created_at,updated_at FROM users WHERE id=? AND account_id=?",
                                        (user["id"], imported_id)).fetchone()
                    assert actual == (user["created_at"], user["updated_at"]), ("user times", user["id"])
                for group, sample in imported_samples.items():
                    if sample is None:
                        continue
                    table = FRESH_UPDATE_TABLES[group]
                    created_at, updated_at = db.execute(
                        f"SELECT created_at,updated_at FROM {table} WHERE id=?",
                        (sample["id"],)).fetchone()
                    assert created_at == sample["created_at"], (group, "creation time")
                    assert_fresh(updated_at, import_started)
                queries = {
                    "accesses": "SELECT COUNT(*) FROM access_records WHERE account_id=?",
                    "user_settings": "SELECT COUNT(*) FROM user_settings_records WHERE account_id=?",
                    "entropies": "SELECT COUNT(*) FROM entropy_records WHERE account_id=?",
                    "comments": "SELECT COUNT(*) FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.account_id=?",
                    "steps": "SELECT COUNT(*) FROM steps s JOIN cards c ON c.id=s.card_id WHERE c.account_id=?",
                    "taggings": "SELECT COUNT(*) FROM taggings t JOIN cards c ON c.id=t.card_id WHERE c.account_id=?",
                    "closures": "SELECT COUNT(*) FROM closures z JOIN cards c ON c.id=z.card_id WHERE c.account_id=?",
                    "card_goldnesses": "SELECT COUNT(*) FROM card_goldnesses WHERE account_id=?",
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
                assert db.execute("SELECT identity_id FROM users WHERE account_id=? AND role='system'",
                                  (imported_id,)).fetchone() == (None,)
                assert db.execute("SELECT COUNT(*) FROM cards WHERE account_id=? AND description!=''",
                                  (imported_id,)).fetchone()[0] == described_cards
                assert db.execute("SELECT COUNT(*) FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.account_id=? AND m.body!=''",
                                  (imported_id,)).fetchone()[0] == described_comments
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
                for gold in golden_records:
                    actual = db.execute("SELECT card_id,created_at,updated_at FROM card_goldnesses WHERE id=?",
                                        (gold["id"],)).fetchone()
                    assert actual[:2] == (gold["card_id"], gold["created_at"])
                    assert_fresh(actual[2], import_started)
                for closure in closure_records:
                    actual = db.execute("SELECT card_id,user_id,created_at,updated_at FROM closures WHERE id=?",
                                        (closure["id"],)).fetchone()
                    assert actual[:3] == (closure["card_id"], closure.get("user_id"), closure["created_at"])
                    assert_fresh(actual[3], import_started)
                for access in access_records:
                    actual = db.execute(
                        "SELECT r.board_id,r.user_id,x.involvement,x.accessed_at,r.created_at,r.updated_at "
                        "FROM access_records r JOIN accesses x ON x.board_id=r.board_id AND x.user_id=r.user_id "
                        "WHERE r.id=?", (access["id"],)).fetchone()
                    assert actual[:5] == (access["board_id"], access["user_id"], access["involvement"],
                                          access.get("accessed_at"), access["created_at"])
                    assert_fresh(actual[5], import_started)
                for setting in setting_records:
                    actual = db.execute(
                        "SELECT r.user_id,n.bundle_email_frequency,t.timezone_name,r.created_at,r.updated_at "
                        "FROM user_settings_records r JOIN notification_settings n ON n.user_id=r.user_id "
                        "LEFT JOIN user_timezones t ON t.user_id=r.user_id WHERE r.id=?",
                        (setting["id"],)).fetchone()
                    assert actual[:4] == (setting["user_id"], setting["bundle_email_frequency"],
                                          setting.get("timezone_name"), setting["created_at"])
                    assert_fresh(actual[4], import_started)
                restored_entropies = {}
                for entropy in entropy_records:
                    container_id = imported_id if entropy["container_type"] == "Account" else entropy["container_id"]
                    actual = db.execute(
                        "SELECT id,account_id,created_at,updated_at FROM entropy_records "
                        "WHERE container_type=? AND container_id=?",
                        (entropy["container_type"], container_id)).fetchone()
                    assert actual and actual[1] == imported_id
                    restored_entropies[(entropy["container_type"], container_id)] = actual
                    if entropy["container_type"] == "Account":
                        assert actual[0] != entropy["id"]
                    else:
                        assert (actual[0], actual[2], actual[3]) == (
                            entropy["id"], entropy["created_at"], entropy["updated_at"])
                    if entropy["container_type"] == "Account":
                        days = db.execute("SELECT auto_postpone_period_in_days FROM accounts WHERE id=?",
                                          (imported_id,)).fetchone()[0]
                    else:
                        days = db.execute("SELECT auto_postpone_period_in_days FROM boards WHERE id=?",
                                          (container_id,)).fetchone()[0]
                    assert days * 86400 == entropy["auto_postpone_period"]
                for publication in publication_records:
                    actual = db.execute("SELECT board_id,key,created_at,updated_at FROM board_publications WHERE id=?",
                                        (publication["id"],)).fetchone()
                    assert actual[:3] == (publication["board_id"], publication["key"], publication["created_at"])
                    assert_fresh(actual[3], import_started)
                for spike in spike_records:
                    actual = db.execute("SELECT card_id,created_at,updated_at FROM card_activity_spikes WHERE id=?",
                                        (spike["id"],)).fetchone()
                    assert actual[:2] == (spike["card_id"], spike["created_at"])
                    assert_fresh(actual[2], import_started)
                imported_watches = {
                    row[0]: row[1:] for row in db.execute(
                        "SELECT w.id,w.card_id,w.user_id,w.watching,w.created_at,w.updated_at "
                        "FROM watches w JOIN cards c ON c.id=w.card_id WHERE c.account_id=?",
                        (imported_id,))}
                assert len(imported_watches) == len(watch_records)
                for watch in watch_records:
                    actual = imported_watches[watch["id"]]
                    assert actual[:4] == (
                        watch["card_id"], watch["user_id"], int(watch["watching"]),
                        watch["created_at"])
                    assert_fresh(actual[4], import_started)
                imported_rich_texts = {
                    row[0]: row[1:] for row in db.execute(
                        "SELECT id,record_type,record_id,name,created_at,updated_at,body_is_null "
                        "FROM rich_text_records WHERE account_id=?", (imported_id,))}
                assert len(imported_rich_texts) == len(rich_texts)
                for rich in rich_texts:
                    assert imported_rich_texts[rich["id"]] == (
                        rich["record_type"], rich["record_id"], rich["name"],
                        rich["created_at"], rich["updated_at"], int(rich["body"] is None))
                for not_now in postponed_records:
                    actual = db.execute("SELECT card_id,user_id,created_at,updated_at FROM card_not_nows WHERE id=?",
                                        (not_now["id"],)).fetchone()
                    assert actual[:3] == (not_now["card_id"], not_now.get("user_id"), not_now["created_at"])
                    assert_fresh(actual[3], import_started)
                for bundle in bundles:
                    status_value = db.execute("SELECT status FROM notification_bundles WHERE id=?", (bundle["id"],)).fetchone()[0]
                    expected = {"pending": 0, "processing": 1, "delivered": 2}.get(bundle["status"], bundle["status"])
                    assert status_value == expected
                embedded_card = next((rich for rich in rich_texts
                                      if rich.get("record_type") == "Card" and
                                      'gid="gid://fizzy/' in (rich.get("body") or "")), None)
                if embedded_card:
                    description = db.execute("SELECT description FROM cards WHERE id=? AND account_id=?",
                                             (embedded_card["record_id"], imported_id)).fetchone()[0]
                    assert 'sgid="' in description and ' gid="' not in description
                    card_has_user_mention = 'gid="gid://fizzy/User/' in embedded_card["body"]
                    if card_has_user_mention:
                        user_sgid = db.execute("SELECT sgid FROM user_attachables WHERE user_id=?", (owner["id"],)).fetchone()[0]
                        assert f'sgid="{user_sgid}"' in description
                    card_number = db.execute("SELECT number FROM cards WHERE id=?",
                                             (embedded_card["record_id"],)).fetchone()[0]
                    status, card = call(url, token, f"/{created['account']}/cards/{card_number}")
                    assert status == 200 and card["has_attachments"]
                    if card_has_user_mention:
                        assert owner["name"].split()[0] in card["description"]
                        assert 'class="mention"' in card["description_html"]
            board_ids = set()
            visible_board_ids = {access["board_id"] for access in access_records
                                 if access["user_id"] == owner["id"]}
            for page in range(1, counts["boards"] + 2):
                status, boards = call(url, token, f"/{created['account']}/boards?page={page}")
                assert status == 200, (status, page)
                board_ids.update(board["id"] for board in boards)
                if len(board_ids) == len(visible_board_ids) or not boards:
                    break
            assert board_ids == visible_board_ids, (board_ids, visible_board_ids)
            if owner_notices:
                status, notices = call(url, token, f"/{created['account']}/notifications")
                assert status == 200 and len(notices) == len(owner_notices), (status, notices, owner_notices)
                if any(item["source_type"] == "Mention" for item in owner_notices):
                    assert any(notice["source_type"] == "mention"
                                          and notice["title"] == f"{owner['name'].split()[0]} @mentioned you"
                                          and "Hello world" in notice["body"] for notice in notices), notices
                    if has_user_mention:
                        assert any(owner["name"].split()[0] in notice["body"]
                                   for notice in notices if notice["source_type"] == "mention"), notices
            else:
                status, notices = call(url, token, f"/{created['account']}/notifications")
                assert status == 200 and notices == [], (status, notices)
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
                reexported_account = json.loads(z.read("data/account.json"))
                assert reexported_account["updated_at"] == imported_account[2]
                for group, expected in counts.items():
                    actual = sum(name.startswith(f"data/{group}/") for name in names)
                    assert actual == expected, ("reexport", group, actual, expected)
                for user in users:
                    restored = json.loads(z.read(f"data/users/{user['id']}.json"))
                    assert (restored["created_at"], restored["updated_at"]) == (
                        user["created_at"], user["updated_at"])
                for group, sample in imported_samples.items():
                    if sample is None:
                        continue
                    restored = json.loads(z.read(f"data/{group}/{sample['id']}.json"))
                    assert restored["created_at"] == sample["created_at"], (group, "reexport creation time")
                    assert_fresh(restored["updated_at"], import_started)
                for blob_id, source_bytes in original_bytes.items():
                    blob = json.loads(z.read(f"data/active_storage_blobs/{blob_id}.json"))
                    assert z.read(f"storage/{blob['key']}") == source_bytes
                for source_blob in blob_metadata:
                    blob = json.loads(z.read(f"data/active_storage_blobs/{source_blob['id']}.json"))
                    assert blob["metadata"] == source_blob["metadata"]
                for source_card in dated_cards:
                    card = json.loads(z.read(f"data/cards/{source_card['id']}.json"))
                    assert card["due_on"] == source_card["due_on"]
                for source_gold in golden_records:
                    restored = json.loads(z.read(f"data/card_goldnesses/{source_gold['id']}.json"))
                    assert_generic_reexport(restored, source_gold, ("card_id", "created_at"), import_started)
                for source_closure in closure_records:
                    restored = json.loads(z.read(f"data/closures/{source_closure['id']}.json"))
                    assert_generic_reexport(restored, source_closure,
                                            ("card_id", "user_id", "created_at"), import_started)
                for source_access in access_records:
                    restored = json.loads(z.read(f"data/accesses/{source_access['id']}.json"))
                    assert_generic_reexport(restored, source_access,
                                            ("board_id", "user_id", "involvement", "accessed_at", "created_at"),
                                            import_started)
                for source_setting in setting_records:
                    restored = json.loads(z.read(f"data/user_settings/{source_setting['id']}.json"))
                    assert_generic_reexport(restored, source_setting,
                                            ("user_id", "bundle_email_frequency", "timezone_name", "created_at"),
                                            import_started)
                for source_entropy in entropy_records:
                    container_id = imported_id if source_entropy["container_type"] == "Account" else source_entropy["container_id"]
                    record = restored_entropies[(source_entropy["container_type"], container_id)]
                    restored = json.loads(z.read(f"data/entropies/{record[0]}.json"))
                    assert {key: restored[key] for key in ("account_id", "auto_postpone_period", "container_id", "container_type", "created_at", "updated_at")} == {
                        "account_id": imported_id,
                        "auto_postpone_period": source_entropy["auto_postpone_period"],
                        "container_id": container_id,
                        "container_type": source_entropy["container_type"],
                        "created_at": record[2],
                        "updated_at": record[3]}
                for source_publication in publication_records:
                    restored = json.loads(z.read(f"data/board_publications/{source_publication['id']}.json"))
                    assert_generic_reexport(restored, source_publication,
                                            ("board_id", "key", "created_at"), import_started)
                for source_spike in spike_records:
                    restored = json.loads(z.read(f"data/card_activity_spikes/{source_spike['id']}.json"))
                    assert_generic_reexport(restored, source_spike, ("card_id", "created_at"), import_started)
                for source_watch in watch_records:
                    restored = json.loads(z.read(f"data/watches/{source_watch['id']}.json"))
                    assert_generic_reexport(restored, source_watch,
                                            ("card_id", "user_id", "watching", "created_at"), import_started)
                for source_rich in rich_texts:
                    restored = json.loads(z.read(f"data/action_text_rich_texts/{source_rich['id']}.json"))
                    assert {key: restored[key] for key in ("body", "created_at", "name", "record_id", "record_type", "updated_at")} == {
                        key: source_rich[key] for key in ("body", "created_at", "name", "record_id", "record_type", "updated_at")}
                for source_not_now in postponed_records:
                    restored = json.loads(z.read(f"data/card_not_nows/{source_not_now['id']}.json"))
                    assert_generic_reexport(restored, source_not_now,
                                            ("card_id", "user_id", "created_at"), import_started)
                for source_bundle in bundles:
                    bundle = json.loads(z.read(f"data/notification_bundles/{source_bundle['id']}.json"))
                    expected = source_bundle["status"] if isinstance(source_bundle["status"], str) else ("pending", "processing", "delivered")[source_bundle["status"]]
                    assert bundle["status"] == expected
                if has_user_mention:
                    assert any(user_gid in (json.loads(z.read(name)).get("body") or "")
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
            subprocess.run([BINARY, "cleanup-imports"], cwd=ROOT, env=env,
                           capture_output=True, text=True, check=True)
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
        if not args.archive:
            same_name_archive = Path(temp) / "account-named-import.zip"
            with zipfile.ZipFile(archive_path) as source, zipfile.ZipFile(same_name_archive, "w") as target:
                for item in source.infolist():
                    content = source.read(item.filename)
                    if item.filename == "data/account.json":
                        account = json.loads(content)
                        assert account["cards_count"] > 0
                        account["name"] = "Import"
                        content = json.dumps(account).encode()
                    target.writestr(item, content)
            run(same_name_archive)


if __name__ == "__main__":
    main()
