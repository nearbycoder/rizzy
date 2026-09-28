#!/usr/bin/env python3
"""Repair old request ID width and a missing board guard FK on MySQL."""

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
    database = "rzupgrade_" + uuid.uuid4().hex[:12]
    mysql(f"CREATE DATABASE `{database}` CHARACTER SET utf8mb4 COLLATE utf8mb4_bin; "
          f"GRANT ALL PRIVILEGES ON `{database}`.* TO '{username}'@'127.0.0.1';")
    env = {**os.environ,
           "RIZZY_DATABASE_URL": urlunparse(parsed._replace(path="/" + database)),
           "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
    try:
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True).stdout
        slug = re.search(r"Account: (\d+)", seeded).group(1)
        account_id = mysql("SELECT id FROM accounts WHERE external_account_id=" + slug,
                           database)
        mysql("ALTER TABLE storage_entries MODIFY COLUMN request_id VARCHAR(64) NULL; "
              "ALTER TABLE card_bulk_touches DROP FOREIGN KEY fk_card_bulk_touches_0; "
              "DROP TABLE cancellation_mail_jobs;",
              database)
        for _ in range(2):
            subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env,
                           capture_output=True, text=True, check=True)
            capacity = mysql(
                "SELECT character_maximum_length FROM information_schema.columns "
                "WHERE table_schema=DATABASE() AND table_name='storage_entries' "
                "AND column_name='request_id'", database)
            foreign_key = mysql(
                "SELECT COUNT(*) FROM information_schema.referential_constraints "
                "WHERE constraint_schema=DATABASE() "
                "AND constraint_name='fk_card_bulk_touches_0'", database)
            mail_fk = mysql(
                "SELECT COUNT(*) FROM information_schema.referential_constraints "
                "WHERE constraint_schema=DATABASE() "
                "AND constraint_name='fk_cancellation_mail_jobs_0'", database)
            assert capacity == "255" and foreign_key == "1" and mail_fk == "1", (
                capacity, foreign_key, mail_fk)
            assert mysql("SELECT id FROM accounts WHERE external_account_id=" + slug,
                         database) == account_id
        long_id = "X" * 255
        mysql("INSERT INTO storage_entries(id,account_id,created_at,delta,operation,request_id) "
              f"VALUES('trace-probe','{account_id}','2026-01-01T00:00:00.000Z',1,'attach','{long_id}');",
              database)
        assert mysql("SELECT CHAR_LENGTH(request_id) FROM storage_entries "
                     "WHERE id='trace-probe'", database) == "255"
        print("PASS MySQL request ID, board guard FK, and cancellation mail table upgrade")
    finally:
        mysql(f"DROP DATABASE `{database}`")


if __name__ == "__main__":
    main()
