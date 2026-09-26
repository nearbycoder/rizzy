#!/usr/bin/env python3
"""Check default settings and stable metadata across updates."""
import sqlite3
import tempfile
import time

from auth import stop
from search_html_parity import api, rust_target


def record(target, user_id):
    with sqlite3.connect(target["db"]) as db:
        return db.execute(
            "SELECT r.id,r.account_id,r.created_at,r.updated_at,n.bundle_email_frequency,t.timezone_name "
            "FROM user_settings_records r JOIN notification_settings n ON n.user_id=r.user_id "
            "LEFT JOIN user_timezones t ON t.user_id=r.user_id WHERE r.user_id=?",
            (user_id,)).fetchone()


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        try:
            owner = next(user for user in api(target, "/users") if user["role"] == "owner")
            first = record(target, owner["id"])
            assert first and first[2] == first[3] and first[4:] == ("every_few_hours", None)

            time.sleep(.02)
            api(target, "/notifications/settings", "PUT", {
                "user_settings": {"bundle_email_frequency": "daily"}}, expected=204)
            changed = record(target, owner["id"])
            assert changed[:3] == first[:3] and changed[3] > first[3]
            assert changed[4:] == ("daily", None)

            api(target, "/notifications/settings", "PUT", {
                "user_settings": {"bundle_email_frequency": "daily"}}, expected=204)
            assert record(target, owner["id"]) == changed

            time.sleep(.02)
            api(target, "/my/timezone", "PUT", {"timezone_name": "America/Chicago"}, expected=204)
            zoned = record(target, owner["id"])
            assert zoned[:3] == first[:3] and zoned[3] > changed[3]
            assert zoned[4:] == ("daily", "America/Chicago")

            api(target, "/my/timezone", "PUT", {"timezone_name": "America/Chicago"}, expected=204)
            assert record(target, owner["id"]) == zoned
            print("User settings lifecycle passed: default, changes, repeat no-ops, stable ID")
        finally:
            stop(process)


if __name__ == "__main__":
    main()
