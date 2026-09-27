#!/usr/bin/env python3
"""Verify webhook management and account/board authorization."""
import hashlib
import os
import re
import sqlite3
import subprocess
import tempfile
import time
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from join import call


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "webhooks.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}", "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
        owner_token = re.search(r"Bearer token: (\w+)", seeded.stdout).group(1)
        port, server = start(env)
        try:
            board, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Hooks", "all_access": False}}, bearer=owner_token, status=201)
            board_id = board["id"]
            path = f"/{account}/boards/{board_id}/webhooks"
            owner_id = board["creator"]["id"]
            with sqlite3.connect(database) as db:
                account_id = db.execute("SELECT account_id FROM boards WHERE id=?", (board_id,)).fetchone()[0]
                db.execute("INSERT INTO identities(id,email_address,created_at) VALUES('webhook-member-identity','webhook-member@example.com','2026-01-01T00:00:00Z')")
                db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('webhook-member',?,'webhook-member-identity','Webhook Member','member','2026-01-01T00:00:00Z')", (account_id,))
                db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES('webhook-member-token','webhook-member-identity',?,'Test','write','2026-01-01T00:00:00Z')",
                           (hashlib.sha256(b'webhook-member-token').hexdigest(),))
            member_token = "webhook-member-token"
            call(port, path, bearer=member_token, status=404)
            call(port, f"/{account}/boards/{board_id}", "PUT", {"user_ids": [owner_id, "webhook-member"]}, bearer=owner_token)
            call(port, path, bearer=member_token, status=403)
            empty, _ = call(port, path, bearer=owner_token)
            assert empty == []
            call(port, path, "POST", {"webhook": {"name": "", "url": "invalid-url"}}, bearer=owner_token, status=422)
            webhook, headers = call(port, path, "POST", {"webhook": {"name": "Production", "url": "https://example.invalid/hook", "subscribed_actions": ["", "card_closed", "bad", "card_closed", "comment_created"]}}, bearer=owner_token, status=201)
            assert headers["Location"].endswith(f"/{board_id}/webhooks/{webhook['id']}.json")
            assert webhook["name"] == "Production" and webhook["payload_url"] == "https://example.invalid/hook"
            assert webhook["active"] and len(webhook["signing_secret"]) == 24
            assert webhook["subscribed_actions"] == ["card_closed", "comment_created"]
            assert webhook["board"]["id"] == board_id
            detail_path = path + "/" + webhook["id"]
            detail, _ = call(port, detail_path, bearer=owner_token)
            assert detail == webhook
            listing, _ = call(port, path, bearer=owner_token)
            assert listing == [webhook]
            call(port, f"/{account}/boards/{board_id}/cards", "POST", {"card": {"title": "Unsubscribed"}}, bearer=owner_token, status=201)
            deliveries, _ = call(port, detail_path + "/deliveries", bearer=owner_token)
            assert deliveries == []
            updated, _ = call(port, detail_path, "PATCH", {"webhook": {"name": "Updated", "url": "https://different.example/hook", "subscribed_actions": ["card_published"]}}, bearer=owner_token)
            assert updated["name"] == "Updated" and updated["payload_url"] == webhook["payload_url"]
            assert updated["subscribed_actions"] == ["card_published"]
            call(port, detail_path, "PATCH", {"webhook": {"name": ""}}, bearer=owner_token, status=422)
            with sqlite3.connect(database) as db:
                db.execute("UPDATE webhooks SET active=0 WHERE id=?", (webhook["id"],))
            call(port, f"/{account}/boards/{board_id}/cards", "POST", {"card": {"title": "Inactive"}}, bearer=owner_token, status=201)
            deliveries, _ = call(port, detail_path + "/deliveries", bearer=owner_token)
            assert deliveries == []
            activated, _ = call(port, detail_path + "/activation", "POST", bearer=owner_token, status=201)
            assert activated["active"] is True
            call(port, f"/{account}/boards/{board_id}/cards", "POST", {"card": {"title": "Subscribed"}}, bearer=owner_token, status=201)
            deliveries, _ = call(port, detail_path + "/deliveries", bearer=owner_token)
            assert len(deliveries) == 1 and deliveries[0]["state"] == "pending"
            assert deliveries[0]["request"] is None and deliveries[0]["response"] is None
            assert deliveries[0]["event"]["action"] == "card_published"
            call(port, detail_path, "DELETE", bearer=owner_token, status=204)
            call(port, detail_path, bearer=owner_token, status=404)
            blocked, _ = call(port, path, "POST", {"webhook": {"name": "Private target", "url": "http://127.0.0.1:1/hook", "subscribed_actions": ["card_published"]}}, bearer=owner_token, status=201)
            formats = {}
            for name, url, content_type in (
                ("Campfire", "http://127.0.0.1:1/rooms/123/456-room-name/messages", "text/html"),
                ("Basecamp", "http://127.0.0.1:1/123/integrations/webhook/buckets/456/chats/789/lines", "application/x-www-form-urlencoded"),
            ):
                hook, _ = call(port, path, "POST", {"webhook": {"name": name, "url": url,
                    "subscribed_actions": ["card_published"]}}, bearer=owner_token, status=201)
                formats[name] = (hook["id"], content_type)
            call(port, f"/{account}/boards/{board_id}/cards", "POST", {"card": {"title": "Blocked destination"}}, bearer=owner_token, status=201)
            blocked_path = path + "/" + blocked["id"] + "/deliveries"
            queued, _ = call(port, blocked_path, bearer=owner_token)
            assert len(queued) == 1 and queued[0]["state"] == "pending"
            with sqlite3.connect(database) as db:
                db.execute("UPDATE webhook_delinquency_trackers SET consecutive_failures_count=9,first_failure_at='2025-01-01T00:00:00.000Z' WHERE webhook_id=?", (blocked["id"],))
            stop(server)
            worker_env = {key: value for key, value in env.items() if key != "RIZZY_DISABLE_WEBHOOK_DELIVERY"}
            port, server = start(worker_env)
            for _ in range(50):
                delivered, _ = call(port, blocked_path, bearer=owner_token)
                if delivered[0]["state"] == "completed":
                    break
                time.sleep(0.1)
            assert delivered[0]["state"] == "completed", delivered
            assert delivered[0]["response"] == {"code": None, "error": "private_uri"}, delivered
            assert delivered[0]["request"]["headers"]["User-Agent"] == "fizzy/1.0.0 Webhook"
            assert "X-Webhook-Signature" not in delivered[0]["request"]["headers"]
            for name, (hook_id, content_type) in formats.items():
                format_path = path + "/" + hook_id + "/deliveries"
                for _ in range(50):
                    formatted, _ = call(port, format_path, bearer=owner_token)
                    if formatted[0]["state"] == "completed":
                        break
                    time.sleep(0.1)
                assert formatted[0]["state"] == "completed", (name, formatted)
                assert formatted[0]["response"] == {"code": None, "error": "private_uri"}, (name, formatted)
                assert formatted[0]["request"]["headers"]["Content-Type"] == content_type, (name, formatted)
            deactivated, _ = call(port, path + "/" + blocked["id"], bearer=owner_token)
            assert deactivated["active"] is False
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT consecutive_failures_count FROM webhook_delinquency_trackers WHERE webhook_id=?", (blocked["id"],)).fetchone()[0] == 10
            print("Webhook management and delivery passed: CRUD, formats, subscriptions, queue recovery, private IP protection, and delinquency")
        finally:
            stop(server)


if __name__ == "__main__":
    main()
