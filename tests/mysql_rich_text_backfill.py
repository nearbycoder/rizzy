#!/usr/bin/env python3
"""Backfill older MySQL rich text rows without duplicating them on restart."""

import os
import re
import subprocess
import uuid
from urllib.parse import unquote, urlparse, urlunparse

from mysql_schema_resume import BINARY, ROOT, mysql


def main():
    parsed = urlparse(os.environ.get("RIZZY_TEST_MYSQL_URL", ""))
    if parsed.scheme != "mysql" or not parsed.username:
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL to a local MySQL test account")
    username = unquote(parsed.username)
    if not re.fullmatch(r"[A-Za-z0-9_]+", username):
        raise SystemExit("MySQL test username must contain only letters, digits, or underscore")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    database = "rzrich_" + uuid.uuid4().hex[:12]
    mysql(f"CREATE DATABASE `{database}` CHARACTER SET utf8mb4 COLLATE utf8mb4_bin; "
          f"GRANT ALL PRIVILEGES ON `{database}`.* TO '{username}'@'127.0.0.1';")
    env = {**os.environ,
           "RIZZY_DATABASE_URL": urlunparse(parsed._replace(path="/" + database)),
           "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
    try:
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True).stdout
        slug = re.search(r"Account: (\d+)", seeded).group(1)
        account_id, owner_id = mysql(
            "SELECT a.id,u.id FROM accounts a JOIN users u ON u.account_id=a.id "
            f"WHERE a.external_account_id={slug} AND u.role='owner' LIMIT 1", database
        ).split("\t")
        timestamp = "2026-01-01T00:00:00.000Z"
        mysql(
            "INSERT INTO boards(id,account_id,creator_id,name,all_access,public_description,created_at,updated_at) "
            f"VALUES('legacy-board','{account_id}','{owner_id}','Legacy',0,'Board body','{timestamp}','{timestamp}');"
            "INSERT INTO cards(id,account_id,board_id,creator_id,number,title,description,created_at,updated_at,last_active_at) "
            f"VALUES('legacy-card','{account_id}','legacy-board','{owner_id}',1,'Legacy card','Card body','{timestamp}','{timestamp}','{timestamp}');"
            "INSERT INTO comments(id,card_id,creator_id,body,created_at) "
            f"VALUES('legacy-comment','legacy-card','{owner_id}','Comment body','{timestamp}');"
            "DELETE FROM app_secrets WHERE name='rich_text_records_backfill_v1';",
            database,
        )
        assert mysql("SELECT COUNT(*) FROM rich_text_records WHERE record_id LIKE 'legacy-%'", database) == "0"
        for _ in range(2):
            subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env,
                           capture_output=True, text=True, check=True)
            records = mysql(
                "SELECT record_type,record_id,name,account_id,body_is_null "
                "FROM rich_text_records WHERE record_id LIKE 'legacy-%' ORDER BY record_type",
                database,
            ).splitlines()
            assert records == [
                f"Board\tlegacy-board\tdescription\t{account_id}\t0",
                f"Card\tlegacy-card\tdescription\t{account_id}\t0",
                f"Comment\tlegacy-comment\tbody\t{account_id}\t0",
            ], records
        assert mysql("SELECT value FROM app_secrets WHERE name='rich_text_records_backfill_v1'",
                     database) == "done"
        print("PASS MySQL rich text backfill and restart idempotency")
    finally:
        mysql(f"DROP DATABASE `{database}`")


if __name__ == "__main__":
    main()
