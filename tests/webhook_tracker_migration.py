#!/usr/bin/env python3
"""Upgrade legacy webhook failure columns into independent tracker rows."""

import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-webhook-tracker-upgrade-") as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST", {"board": {"name": "Legacy tracker"}}, 201)
        webhook = api(target, f"/boards/{board['id']}/webhooks", "POST", {
            "webhook": {"name": "Legacy", "url": "https://example.invalid/hook",
                        "subscribed_actions": []}}, 201)
        stop(process)
        old_failure = "2025-01-01T00:00:00.000Z"
        with sqlite3.connect(target["db"]) as db:
            db.execute("DROP TABLE webhook_delinquency_trackers")
            db.execute("UPDATE webhooks SET consecutive_failures_count=9,first_failure_at=? "
                       "WHERE id=?", (old_failure, webhook["id"]))
            old = db.execute("SELECT account_id,created_at,updated_at FROM webhooks WHERE id=?",
                             (webhook["id"],)).fetchone()

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        for attempt in range(2):
            _, restarted = start(env, binary=BINARY)
            try:
                with sqlite3.connect(target["db"]) as db:
                    rows = db.execute(
                        "SELECT id,account_id,webhook_id,consecutive_failures_count,"
                        "first_failure_at,created_at,updated_at "
                        "FROM webhook_delinquency_trackers WHERE webhook_id=?",
                        (webhook["id"],)).fetchall()
                    assert rows == [(webhook["id"], old[0], webhook["id"], 9,
                                     old_failure, old[1], old[2])], rows
                    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
            finally:
                stop(restarted)
        print("PASS legacy webhook tracker backfill and idempotent restart")


if __name__ == "__main__":
    main()
