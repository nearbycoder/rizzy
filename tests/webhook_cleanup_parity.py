#!/usr/bin/env python3
"""Compare seven-day webhook delivery cleanup over Fizzy's 500-row batch limit."""

import json
import sqlite3
import subprocess
import sys
import tempfile
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

sys.path[:0] = [str(Path(__file__).resolve().parent),
                str(Path(__file__).resolve().parents[1] / "bench")]
from card_image_parity import call  # noqa: E402
from sustained_mixed import (ROOT, RUST, SOURCE, prepare_rust, prepare_source,
                             start_server, stop_server)  # noqa: E402


def api(target, path, method="GET", payload=None, expected=200):
    status, _, body = call(target, f"/{target['account']}{path}", method, payload)
    assert status == expected, (target["name"], method, path, status, body[:250])
    return json.loads(body) if body else None


def source_runner(target, ruby):
    return subprocess.run(["bin/rails", "runner", ruby], cwd=SOURCE,
                          env=target["env"], capture_output=True,
                          text=True, check=True).stdout


def fixture(target):
    board = api(target, "/boards", "POST", {"board": {"name": "Delivery cleanup"}}, 201)
    hook = api(target, f"/boards/{board['id']}/webhooks", "POST", {
        "webhook": {"name": "Cleanup", "url": "https://example.invalid/hook",
                    "subscribed_actions": ["card_published"]}}, 201)
    api(target, f"/boards/{board['id']}/cards", "POST", {
        "card": {"title": "Keep the event"}}, 201)
    if target["name"] == "fizzy":
        source_runner(target, "Event::WebhookDispatchJob.perform_now("
                      "Event.where(board_id: Webhook.find(" + json.dumps(hook["id"])
                      + ").board_id, action: 'card_published').order(created_at: :desc).first)")
    deliveries = api(target, f"/boards/{board['id']}/webhooks/{hook['id']}/deliveries")
    assert len(deliveries) == 1 and deliveries[0]["state"] == "pending", (
        target["name"], deliveries)
    return hook["id"], deliveries[0]["id"]


def add_history(target, delivery_id):
    old = datetime.now(timezone.utc) - timedelta(days=8)
    recent = datetime.now(timezone.utc) - timedelta(days=6)
    current = datetime.now(timezone.utc)
    stamp = lambda value: (value.strftime("%Y-%m-%d %H:%M:%S.%f")
                           if target["name"] == "fizzy" else
                           value.isoformat(timespec="milliseconds").replace("+00:00", "Z"))
    with sqlite3.connect(target["db"]) as db:
        db.row_factory = sqlite3.Row
        original_id = (int(delivery_id, 36).to_bytes(16, "big")
                       if target["name"] == "fizzy" else delivery_id)
        source = dict(db.execute("SELECT * FROM webhook_deliveries WHERE id=?",
                                 (original_id,)).fetchone())
        columns = list(source)
        statement = (f"INSERT INTO webhook_deliveries ({','.join(columns)}) VALUES ("
                     + ",".join("?" for _ in columns) + ")")
        for index in range(505):
            row = dict(source)
            row["id"] = (uuid.uuid4().bytes if target["name"] == "fizzy"
                         else uuid.uuid4().hex)
            row["created_at"] = stamp(old if index < 503 else recent)
            row["updated_at"] = stamp(current)
            row["state"] = ("completed" if index % 2 else "errored")
            db.execute(statement, [row[column] for column in columns])
        plan = db.execute("EXPLAIN QUERY PLAN SELECT id FROM webhook_deliveries "
                          "WHERE created_at<? ORDER BY created_at,id LIMIT 500",
                          (stamp(recent),)).fetchall()
        if target["name"] == "rizzy":
            assert any("webhook_deliveries_stale" in row[3] for row in plan), plan


def remaining(target, webhook_id):
    stored_id = (int(webhook_id, 36).to_bytes(16, "big")
                 if target["name"] == "fizzy" else webhook_id)
    with sqlite3.connect(target["db"]) as db:
        return db.execute("SELECT COUNT(*) FROM webhook_deliveries WHERE webhook_id=?",
                          (stored_id,)).fetchone()[0]


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-webhook-cleanup-pair-") as temporary:
        for name in ("fizzy", "rizzy"):
            directory = Path(temporary) / name
            directory.mkdir()
            env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                else prepare_rust(directory))
            env = {**env, "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
                   "RIZZY_DISABLE_WEBHOOK_CLEANUP": "1"}
            process, url = start_server(name, directory, env, [0, 1])
            target = {"name": name, "url": url, "env": env,
                      "account": str(credentials["account"]),
                      "token": credentials["token"],
                      "db": directory / ("db.sqlite3" if name == "fizzy"
                                               else "rizzy.sqlite3")}
            try:
                webhook_id, delivery_id = fixture(target)
            finally:
                stop_server(process)
            add_history(target, delivery_id)
            assert remaining(target, webhook_id) == 506
            if name == "fizzy":
                source_runner(target, "Webhook::Delivery.cleanup(batch_size: 500, pause: 0)")
            else:
                result = subprocess.run([str(RUST), "cleanup-webhook-deliveries"],
                                        cwd=ROOT, env=env, check=True, text=True,
                                        capture_output=True)
                assert result.stdout.strip() == "Cleaned 503 webhook deliveries", result.stdout
            assert remaining(target, webhook_id) == 3, (name, remaining(target, webhook_id))
        print("PASS seven-day webhook cleanup parity across the 500-row batch boundary")


if __name__ == "__main__":
    main()
