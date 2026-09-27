#!/usr/bin/env python3
"""Compare webhook timestamps while failed deliveries accrue and deactivate."""

import json
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path[:0] = [str(Path(__file__).resolve().parent),
                str(Path(__file__).resolve().parents[1] / "bench")]
from card_image_parity import call  # noqa: E402
from sustained_mixed import (SOURCE, prepare_rust, prepare_source,  # noqa: E402
                             start_server, stop_server)


def api(target, path, method="GET", payload=None, expected=200):
    status, _, body = call(target, f"/{target['account']}{path}", method, payload)
    assert status == expected, (target["name"], method, path, status, body[:250])
    return json.loads(body) if body else None


def source_runner(target, ruby):
    result = subprocess.run(["bin/rails", "runner", ruby], cwd=SOURCE,
                            env=target["env"], capture_output=True, text=True, check=True)
    return result.stdout


def state(target, webhook_id):
    if target["name"] == "fizzy":
        return json.loads(source_runner(target, (
            'require "json"; w = Webhook.find(' + json.dumps(webhook_id) + '); '
            't = w.delinquency_tracker; puts JSON.generate({'
            '"updated_at" => w.updated_at.utc.iso8601(6), "active" => w.active?, '
            '"failures" => t.consecutive_failures_count, '
            '"first_failure" => t.first_failure_at&.utc&.iso8601(6)})')))
    with sqlite3.connect(target["db"]) as db:
        timestamp, active, failures, first_failure = db.execute(
            "SELECT w.updated_at,w.active,t.consecutive_failures_count,t.first_failure_at "
            "FROM webhooks w JOIN webhook_delinquency_trackers t ON t.webhook_id=w.id "
            "WHERE w.id=?", (webhook_id,)).fetchone()
        return {"updated_at": timestamp, "active": bool(active),
                "failures": failures, "first_failure": first_failure}


def make_delivery(target, board_id, webhook_id, marker):
    api(target, f"/boards/{board_id}/cards", "POST", {"card": {"title": marker}}, 201)
    if target["name"] == "fizzy":
        source_runner(target, 'Event::WebhookDispatchJob.perform_now('
                      'Event.where(board_id: Webhook.find(' + json.dumps(webhook_id)
                      + ').board_id).order(created_at: :desc).first)')
    path = f"/boards/{board_id}/webhooks/{webhook_id}/deliveries"
    for _ in range(100):
        deliveries = api(target, path)
        if len(deliveries) >= (2 if marker.endswith("second") else 1):
            delivery = deliveries[0]
            if target["name"] == "fizzy":
                source_runner(target, 'Webhook::Delivery.find(' + json.dumps(delivery["id"])
                              + ').deliver')
                return
            if delivery["state"] == "completed":
                return
        time.sleep(0.1)
    raise AssertionError((target["name"], "delivery did not complete", deliveries))


def check(target):
    board = api(target, "/boards", "POST", {"board": {"name": "Timestamp tracker"}}, 201)
    board_id = board["id"]
    try:
        webhook = api(target, f"/boards/{board_id}/webhooks", "POST", {
            "webhook": {"name": "Blocked", "url": "http://127.0.0.1:1/hook",
                        "subscribed_actions": ["card_published"]}}, 201)
        webhook_id = webhook["id"]
        original = state(target, webhook_id)
        assert original["active"] and original["failures"] == 0, (target["name"], original)
        make_delivery(target, board_id, webhook_id, "Tracker first")
        first = state(target, webhook_id)
        assert first["active"] and first["failures"] == 1 and first["first_failure"], (
            target["name"], first)
        assert first["updated_at"] == original["updated_at"], (target["name"], original, first)
        if target["name"] == "fizzy":
            source_runner(target, 'Webhook.find(' + json.dumps(webhook_id) +
                          ').delinquency_tracker.update_columns('
                          'consecutive_failures_count: 9, first_failure_at: 2.hours.ago)')
        else:
            with sqlite3.connect(target["db"]) as db:
                db.execute("UPDATE webhook_delinquency_trackers SET consecutive_failures_count=9,"
                           "first_failure_at='2025-01-01T00:00:00.000Z' WHERE webhook_id=?",
                           (webhook_id,))
        make_delivery(target, board_id, webhook_id, "Tracker second")
        second = state(target, webhook_id)
        assert not second["active"] and second["failures"] == 10, (target["name"], second)
        assert second["updated_at"] != original["updated_at"], (target["name"], second)
        return {"first_failure_touched_webhook": first["updated_at"] != original["updated_at"],
                "deactivation_touched_webhook": second["updated_at"] != original["updated_at"],
                "final_active": second["active"], "final_failures": second["failures"]}
    finally:
        api(target, f"/boards/{board_id}", "DELETE", expected=204)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-webhook-tracker-pair-") as temporary:
        targets = {}
        processes = []
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                if name == "rizzy":
                    env = {**env, "RIZZY_DISABLE_WEBHOOK_DELIVERY": "0"}
                process, url = start_server(name, directory, env, [0, 1])
                processes.append(process)
                targets[name] = {"name": name, "url": url, "env": env,
                                 "account": str(credentials["account"]),
                                 "token": credentials["token"],
                                 "db": directory / ("db.sqlite3" if name == "fizzy"
                                                      else "rizzy.sqlite3")}
            source = check(targets["fizzy"])
            rust = check(targets["rizzy"])
            assert source == rust, (source, rust)
            print("PASS webhook failure and deactivation timestamp parity")
        finally:
            for process in processes:
                stop_server(process)


if __name__ == "__main__":
    main()
