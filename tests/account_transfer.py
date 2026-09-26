#!/usr/bin/env python3
"""Exercise Fizzy-format account exports with a disposable Rizzy account."""
import io
import hashlib
import json
import os
import re
import socket
import sqlite3
import subprocess
import tempfile
import time
import zipfile
from datetime import datetime, timezone
from pathlib import Path

from card_image_parity import IMAGE, call
from embed_parity import upload

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def api(target, path, method="GET", payload=None, expected=200):
    status, _, raw = call(target, f"/{target['account']}{path}", method, payload)
    assert status == expected, (path, status, raw[:150])
    return json.loads(raw) if raw else None


def mutate(target, path, method="POST", payload=None):
    status, _, raw = call(target, f"/{target['account']}{path}", method, payload)
    assert status in (200, 201, 204), (path, status, raw[:150])
    return json.loads(raw) if raw else None


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        db_path = Path(temp) / "account.sqlite3"
        archive_dir = Path(temp) / "archives"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{db_path}",
               "RIZZY_EXPORT_DIR": str(archive_dir),
               "RIZZY_STORAGE_DIR": str(Path(temp) / "storage"),
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
        seed = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True,
                              text=True, check=True).stdout
        account = re.search(r"Account: (\d+)", seed).group(1)
        token = re.search(r"Bearer token: (\w+)", seed).group(1)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        target = {"url": f"http://127.0.0.1:{port}", "account": account, "token": token}
        server = subprocess.Popen([BINARY], cwd=ROOT,
                                  env={**env, "RIZZY_BIND": f"127.0.0.1:{port}"},
                                  stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            for _ in range(100):
                try:
                    call(target, "/up", bearer=False)
                    break
                except OSError:
                    time.sleep(.05)
            else:
                raise RuntimeError("Rizzy server did not start")
            owner = next(user for user in api(target, "/users") if user["role"] == "owner")
            with sqlite3.connect(db_path) as db:
                settings_record = db.execute(
                    "SELECT r.id,r.account_id,r.created_at,r.updated_at,n.bundle_email_frequency,t.timezone_name "
                    "FROM user_settings_records r JOIN notification_settings n ON n.user_id=r.user_id "
                    "LEFT JOIN user_timezones t ON t.user_id=r.user_id WHERE r.user_id=?",
                    (owner["id"],)).fetchone()
            assert settings_record and settings_record[4] == "every_few_hours"
            board = api(target, "/boards", "POST", {"board": {"name": "Transfer probe"}}, 201)
            with sqlite3.connect(db_path) as db:
                access_record = db.execute(
                    "SELECT r.id,r.account_id,r.created_at,r.updated_at,x.involvement,x.accessed_at "
                    "FROM access_records r JOIN accesses x ON x.board_id=r.board_id AND x.user_id=r.user_id "
                    "WHERE r.board_id=? AND r.user_id=?", (board["id"], owner["id"])).fetchone()
            assert access_record and access_record[4] == "watching"
            image = upload(target)
            embed = upload(target)
            description = (f'<p>Hello <strong>world</strong></p><p><action-text-attachment '
                           f'sgid="{embed["attachable_sgid"]}"></action-text-attachment></p>')
            card = api(target, f"/boards/{board['id']}/cards", "POST", {
                "card": {"title": "Transfer card", "description": description,
                        "image": image["signed_id"]}}, 201)
            postponed_card = api(target, f"/boards/{board['id']}/cards", "POST", {
                "card": {"title": "Postponed transfer card",
                         "description": "<p>Postponed transfer body</p>"}}, 201)
            mutate(target, f"/cards/{postponed_card['number']}/not_now")
            with sqlite3.connect(db_path) as db:
                postponed_record = db.execute(
                    "SELECT id,user_id,created_at FROM card_not_nows WHERE card_id=?",
                    (postponed_card["id"],)).fetchone()
            assert postponed_record and postponed_record[1] == owner["id"]
            step = api(target, f"/cards/{card['number']}/steps", "POST",
                       {"step": {"content": "First step"}}, 201)
            comment = api(target, f"/cards/{card['number']}/comments", "POST",
                          {"comment": {"body": "An observation"}}, 201)
            mutate(target, f"/cards/{card['number']}/taggings", payload={"tag_title": "#transfer"})
            mutate(target, f"/cards/{card['number']}/assignments", payload={"assignee_id": owner["id"]})
            mutate(target, f"/cards/{card['number']}/goldness")
            with sqlite3.connect(db_path) as db:
                gold_record = db.execute(
                    "SELECT id,created_at,updated_at FROM card_goldnesses WHERE card_id=?",
                    (card["id"],)).fetchone()
                watch_record = db.execute(
                    "SELECT id,user_id,watching,created_at,updated_at FROM watches WHERE card_id=? AND user_id=?",
                    (card["id"], owner["id"])).fetchone()
                rich_record = db.execute(
                    "SELECT id,created_at,updated_at FROM rich_text_records WHERE record_type='Card' AND record_id=? AND name='description'",
                    (card["id"],)).fetchone()
            assert gold_record
            assert watch_record
            assert rich_record
            with sqlite3.connect(db_path) as db:
                spike_record = db.execute(
                    "SELECT id,created_at,updated_at FROM card_activity_spikes WHERE card_id=?",
                    (card["id"],)).fetchone()
            assert spike_record
            mutate(target, f"/cards/{card['number']}/closure")
            with sqlite3.connect(db_path) as db:
                closure_record = db.execute(
                    "SELECT id,account_id,user_id,created_at,updated_at FROM closures WHERE card_id=?",
                    (card["id"],)).fetchone()
            assert closure_record and closure_record[2] == owner["id"]
            mutate(target, f"/cards/{card['number']}/reactions", payload={"reaction": {"content": "👍"}})
            mutate(target, f"/boards/{board['id']}/publication")
            with sqlite3.connect(db_path) as db:
                publication_record = db.execute(
                    "SELECT id,account_id,key,created_at,updated_at FROM board_publications WHERE board_id=?",
                    (board["id"],)).fetchone()
            assert publication_record and publication_record[2]
            mutate(target, f"/boards/{board['id']}/webhooks", payload={
                "webhook": {"name": "Transfer hook", "url": "https://example.invalid/rizzy-transfer",
                            "subscribed_actions": []}})
            exported = api(target, "/account/exports", "POST", expected=201)
            assert exported["status"] == "pending" and "download_url" not in exported
            export_id = exported["id"]
            with sqlite3.connect(db_path) as db:
                account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?", (account,)).fetchone()[0]
            assert closure_record[1] == account_id
            assert publication_record[1] == account_id
            assert access_record[1] == account_id
            assert settings_record[1] == account_id
            for _ in range(120):
                state = api(target, f"/account/exports/{export_id}")
                if state["status"] in ("completed", "failed"):
                    break
                time.sleep(.1)
            assert state["status"] == "completed", state
            download_status, download_headers, archive_bytes = call(target, state["download_url"])
            assert download_status == 200 and download_headers.get_content_type() == "application/zip"
            with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
                names = set(archive.namelist())
                for path in ("data/account.json", f"data/boards/{board['id']}.json",
                             f"data/cards/{card['id']}.json", f"data/steps/{step['id']}.json",
                             f"data/comments/{comment['id']}.json",
                             f"data/active_storage_blobs/{image['id']}.json",
                             f"data/active_storage_blobs/{embed['id']}.json",
                             f"storage/{image['key']}", f"storage/{embed['key']}"):
                    assert path in names, path
                for directory in ("tags", "taggings", "assignments", "closures", "card_goldnesses", "card_not_nows",
                                  "watches", "reactions", "board_publications", "webhooks"):
                    assert any(name.startswith(f"data/{directory}/") for name in names), directory
                assert archive.read(f"storage/{image['key']}") == IMAGE
                assert archive.read(f"storage/{embed['key']}") == IMAGE
                assert archive.getinfo(f"storage/{image['key']}").compress_type == zipfile.ZIP_STORED
                account_data = json.loads(archive.read("data/account.json"))
                assert account_data["id"] == account_id and account_data["join_code"]["account_id"] == account_id
                system_user = next(json.loads(archive.read(path)) for path in names
                                   if path.startswith("data/users/")
                                   and json.loads(archive.read(path))["role"] == "system")
                assert system_user["email_address"] is None and system_user["identity_id"] is None
                card_data = json.loads(archive.read(f"data/cards/{card['id']}.json"))
                assert card_data["title"] == "Transfer card" and "description" not in card_data
                gold_data = json.loads(archive.read(f"data/card_goldnesses/{gold_record[0]}.json"))
                assert (gold_data["card_id"], gold_data["created_at"], gold_data["updated_at"]) == (
                    card["id"], gold_record[1], gold_record[2])
                closure_data = json.loads(archive.read(f"data/closures/{closure_record[0]}.json"))
                assert (closure_data["account_id"], closure_data["card_id"], closure_data["user_id"],
                        closure_data["created_at"], closure_data["updated_at"]) == (
                    account_id, card["id"], owner["id"], closure_record[3], closure_record[4])
                access_data = json.loads(archive.read(f"data/accesses/{access_record[0]}.json"))
                assert (access_data["account_id"], access_data["board_id"], access_data["user_id"],
                        access_data["created_at"], access_data["updated_at"], access_data["involvement"],
                        access_data["accessed_at"]) == (
                    account_id, board["id"], owner["id"], access_record[2], access_record[3],
                    access_record[4], access_record[5])
                settings_data = json.loads(archive.read(f"data/user_settings/{settings_record[0]}.json"))
                assert (settings_data["account_id"], settings_data["user_id"],
                        settings_data["created_at"], settings_data["updated_at"],
                        settings_data["bundle_email_frequency"], settings_data["timezone_name"]) == (
                    account_id, owner["id"], settings_record[2], settings_record[3],
                    settings_record[4], settings_record[5])
                publication_data = json.loads(archive.read(
                    f"data/board_publications/{publication_record[0]}.json"))
                assert (publication_data["account_id"], publication_data["board_id"], publication_data["key"],
                        publication_data["created_at"], publication_data["updated_at"]) == (
                    account_id, board["id"], publication_record[2], publication_record[3], publication_record[4])
                spike_data = json.loads(archive.read(f"data/card_activity_spikes/{spike_record[0]}.json"))
                assert (spike_data["card_id"], spike_data["created_at"], spike_data["updated_at"]) == (
                    card["id"], spike_record[1], spike_record[2])
                watch_data = json.loads(archive.read(f"data/watches/{watch_record[0]}.json"))
                assert (watch_data["card_id"], watch_data["user_id"], watch_data["watching"],
                        watch_data["created_at"], watch_data["updated_at"]) == (
                    card["id"], owner["id"], bool(watch_record[2]), watch_record[3], watch_record[4])
                not_now_data = json.loads(archive.read(f"data/card_not_nows/{postponed_record[0]}.json"))
                assert (not_now_data["card_id"], not_now_data["user_id"], not_now_data["created_at"]) == (
                    postponed_card["id"], owner["id"], postponed_record[2])
                rich_texts = [json.loads(archive.read(name)) for name in names
                              if name.startswith("data/action_text_rich_texts/")]
                assert f"data/action_text_rich_texts/{rich_record[0]}.json" in names
                rich_record_data = json.loads(archive.read(f"data/action_text_rich_texts/{rich_record[0]}.json"))
                assert (rich_record_data["created_at"], rich_record_data["updated_at"]) == rich_record[1:]
                assert any(item["record_id"] == card["id"]
                           and f'gid="gid://fizzy/ActiveStorage::Blob/{embed["id"]}"' in item["body"]
                           and embed["attachable_sgid"] not in item["body"]
                           for item in rich_texts)

            # The source permits owner/admin creation but only the requester can view an export.
            with sqlite3.connect(db_path) as db:
                for role in ("admin", "member"):
                    identity_id = f"transfer-{role}-identity"
                    user_id = f"transfer-{role}-user"
                    other_token = f"transfer-{role}-token"
                    db.execute("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)",
                               (identity_id, f"{role}@example.com", datetime.now(timezone.utc).isoformat()))
                    db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES(?,?,?,?,?,?)",
                               (user_id, account_id, identity_id, role.title(), role,
                                datetime.now(timezone.utc).isoformat()))
                    db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) "
                               "VALUES(?,?,?,?,?,?)", (f"transfer-{role}-access", identity_id,
                                                  hashlib.sha256(other_token.encode()).hexdigest(),
                                                  "transfer test", "write", datetime.now(timezone.utc).isoformat()))
            member = {**target, "token": "transfer-member-token"}
            admin = {**target, "token": "transfer-admin-token"}
            api(member, "/account/exports", "POST", expected=403)
            api(member, f"/account/exports/{export_id}", expected=403)
            api(admin, f"/account/exports/{export_id}", expected=404)
            assert call(admin, state["download_url"])[0] == 404
            with sqlite3.connect(db_path) as db:
                for index in range(9):
                    stamp = datetime.now(timezone.utc).isoformat()
                    db.execute("INSERT INTO exports(id,account_id,user_id,kind,status,created_at,updated_at) "
                               "VALUES(?,?,?,'account','failed',?,?)",
                               (f"transfer-limit-{index}", account_id, owner["id"], stamp, stamp))
            api(target, "/account/exports", "POST", expected=429)
            if output := os.environ.get("RIZZY_TRANSFER_TEST_OUTPUT"):
                Path(output).write_bytes(archive_bytes)
            print(f"Account export passed: {len(names)} entries, card, comment, step, and original images")
        finally:
            server.terminate()
            server.wait(timeout=5)


if __name__ == "__main__":
    main()
