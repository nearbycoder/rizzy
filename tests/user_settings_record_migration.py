#!/usr/bin/env python3
"""Backfill settings metadata for users created by older Rizzy builds."""
import os
import sqlite3
import tempfile

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        owner = next(user for user in api(target, "/users") if user["role"] == "owner")
        api(target, "/notifications/settings", "PUT", {
            "user_settings": {"bundle_email_frequency": "weekly"}}, expected=204)
        api(target, "/my/timezone", "PUT", {"timezone_name": "America/Chicago"}, expected=204)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            created_at = db.execute("SELECT created_at FROM users WHERE id=?", (owner["id"],)).fetchone()[0]
            for trigger in ("user_settings_user_insert", "user_settings_frequency_update",
                            "user_settings_timezone_insert", "user_settings_timezone_update",
                            "user_settings_timezone_delete"):
                db.execute(f"DROP TRIGGER {trigger}")
            db.execute("DROP TABLE user_settings_records")

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        stable_id = None
        for _ in range(2):
            _, restarted = start(env, binary=BINARY)
            try:
                with sqlite3.connect(target["db"]) as db:
                    row = db.execute(
                        "SELECT r.id,r.created_at,r.updated_at,n.bundle_email_frequency,t.timezone_name "
                        "FROM user_settings_records r JOIN notification_settings n ON n.user_id=r.user_id "
                        "LEFT JOIN user_timezones t ON t.user_id=r.user_id WHERE r.user_id=?",
                        (owner["id"],)).fetchone()
                    assert row and row[1:] == (
                        created_at, created_at, "weekly", "America/Chicago")
                    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
                    if stable_id is not None:
                        assert row[0] == stable_id
                    stable_id = row[0]
            finally:
                stop(restarted)
        print("User settings migration passed: values preserved and metadata stable")


if __name__ == "__main__":
    main()
