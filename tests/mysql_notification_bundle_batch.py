#!/usr/bin/env python3
"""Drain 205 due bundles on a disposable MySQL 8 database."""

import os
import re
import subprocess
import tempfile
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

from mysql_webhook_cleanup import root_mysql, SOCKET
from mysql_webhooks import BINARY, ROOT


def main():
    if not SOCKET.is_socket():
        raise SystemExit("Start the disposable MySQL 8 server first")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    trust = root_mysql("SELECT @@GLOBAL.log_bin_trust_function_creators;")
    root_mysql("SET GLOBAL log_bin_trust_function_creators=1;")
    suffix = uuid.uuid4().hex[:12]
    database = "rzbundles_" + suffix
    username = "rzbundles_" + suffix
    password = uuid.uuid4().hex
    try:
        root_mysql(f"CREATE DATABASE {database} CHARACTER SET utf8mb4 COLLATE utf8mb4_bin; "
                   f"CREATE USER '{username}'@'127.0.0.1' IDENTIFIED BY '{password}'; "
                   f"GRANT ALL PRIVILEGES ON {database}.* TO '{username}'@'127.0.0.1';")
        url = f"mysql://{username}:{password}@127.0.0.1:33080/{database}"
        with tempfile.TemporaryDirectory(prefix="rizzy-mysql-bundles-") as temporary:
            env = {**os.environ, "RIZZY_DATABASE_URL": url,
                   "RIZZY_STORAGE_DIR": str(Path(temporary) / "storage"),
                   "RIZZY_SENDMAIL": "/bin/true"}
            seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                    check=True, capture_output=True, text=True)
            assert re.search(r"Account: \d+", seeded.stdout), seeded.stdout
            account = root_mysql(f"USE {database}; SELECT id FROM accounts LIMIT 1;")
            ended = (datetime.now(timezone.utc) - timedelta(hours=1)).isoformat(
                timespec="milliseconds").replace("+00:00", "Z")
            started = (datetime.now(timezone.utc) - timedelta(hours=5)).isoformat(
                timespec="milliseconds").replace("+00:00", "Z")
            statements = [f"USE {database};"]
            for index in range(205):
                identity = str(uuid.uuid4())
                user = str(uuid.uuid4())
                bundle = str(uuid.uuid4())
                statements.append(
                    "INSERT INTO identities(id,email_address,created_at) VALUES"
                    f"('{identity}','bundle-{index}@example.test','{started}');")
                statements.append(
                    "INSERT INTO users(id,account_id,identity_id,name,created_at) VALUES"
                    f"('{user}','{account}','{identity}','Bundle {index}','{started}');")
                statements.append(
                    "INSERT INTO notification_bundles(id,account_id,user_id,starts_at,"
                    "ends_at,status,created_at,updated_at) VALUES"
                    f"('{bundle}','{account}','{user}','{started}','{ended}',0,"
                    f"'{started}','{started}');")
            root_mysql(" ".join(statements))

            def run():
                return subprocess.run([BINARY, "deliver-notifications"], cwd=ROOT, env=env,
                                      check=True, capture_output=True, text=True, timeout=30).stdout

            assert run().strip() == "Delivered 205 notification bundles"
            assert root_mysql(f"USE {database}; SELECT COUNT(*) FROM notification_bundles "
                              "WHERE status=2;") == "205"
            assert run().strip() == "Delivered 0 notification bundles"
            root_mysql(f"USE {database}; UPDATE notification_bundles SET status=0;")
            workers = [subprocess.Popen([BINARY, "deliver-notifications"], cwd=ROOT, env=env,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                       for _ in range(2)]
            counts = []
            for worker in workers:
                output, error = worker.communicate(timeout=30)
                assert worker.returncode == 0, error
                counts.append(int(re.search(r"Delivered (\d+) notification bundles", output).group(1)))
            assert sum(counts) == 205, counts
            assert root_mysql(f"USE {database}; SELECT COUNT(*) FROM notification_bundles "
                              "WHERE status=2;") == "205"
            print("PASS MySQL due bundles drain across pages, ties, and two workers")
    finally:
        try:
            root_mysql(f"DROP DATABASE IF EXISTS {database}; "
                       f"DROP USER IF EXISTS '{username}'@'127.0.0.1';")
        finally:
            root_mysql(f"SET GLOBAL log_bin_trust_function_creators={trust};")


if __name__ == "__main__":
    main()
