#!/usr/bin/env python3
"""Backfill legacy webhook counters in an isolated MySQL 8 database."""

import os
import re
import subprocess
import tempfile
import uuid
from urllib.parse import unquote, urlparse, urlunparse

from mysql_core import call
from mysql_schema_resume import mysql
from mysql_webhooks import BINARY, ROOT, start, stop


def main():
    parsed = urlparse(os.environ.get("RIZZY_TEST_MYSQL_URL", ""))
    if parsed.scheme != "mysql" or not parsed.username:
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL to a local MySQL test account")
    username = unquote(parsed.username)
    if not re.fullmatch(r"[A-Za-z0-9_]+", username):
        raise SystemExit("MySQL test username must contain only letters, digits, or underscore")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    database = "rztracker_" + uuid.uuid4().hex[:12]
    mysql(f"CREATE DATABASE `{database}` CHARACTER SET utf8mb4 COLLATE utf8mb4_bin; "
          f"GRANT ALL PRIVILEGES ON `{database}`.* TO '{username}'@'127.0.0.1';")
    url = urlunparse(parsed._replace(path="/" + database))
    env = {**os.environ, "RIZZY_DATABASE_URL": url,
           "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
    try:
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True).stdout
        account = re.search(r"Account: (\d+)", seeded).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded).group(1)
        with tempfile.TemporaryDirectory(prefix="rizzy-mysql-tracker-upgrade-") as temp:
            base, process, _ = start(env, temp, "before-upgrade")
            try:
                board = call(base, token, f"/{account}/boards", "POST",
                             {"board": {"name": "Legacy tracker"}}, 201)
                webhook = call(base, token,
                               f"/{account}/boards/{board['id']}/webhooks", "POST",
                               {"webhook": {"name": "Legacy", "url": "https://example.invalid/hook",
                                            "subscribed_actions": []}}, 201)
                new_tracker_id = mysql(
                    "SELECT id FROM webhook_delinquency_trackers "
                    f"WHERE webhook_id='{webhook['id']}'", database)
                assert new_tracker_id and new_tracker_id != webhook["id"]
            finally:
                stop(process)
        old = mysql(
            "SELECT account_id,created_at,updated_at FROM webhooks "
            f"WHERE id='{webhook['id']}'", database).split("\t")
        failure = "2025-01-01T00:00:00.000Z"
        mysql("DROP TABLE webhook_delinquency_trackers; "
              "UPDATE webhooks SET consecutive_failures_count=9,"
              f"first_failure_at='{failure}' WHERE id='{webhook['id']}';", database)
        for _ in range(2):
            subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env,
                           capture_output=True, text=True, check=True)
            rows = mysql(
                "SELECT id,account_id,webhook_id,consecutive_failures_count,"
                "first_failure_at,created_at,updated_at "
                "FROM webhook_delinquency_trackers "
                f"WHERE webhook_id='{webhook['id']}'", database).splitlines()
            assert rows == ["\t".join((webhook["id"], old[0], webhook["id"], "9",
                                         failure, old[1], old[2]))], rows
        print("PASS MySQL legacy webhook tracker backfill and idempotent restart")
    finally:
        mysql(f"DROP DATABASE `{database}`")


if __name__ == "__main__":
    main()
