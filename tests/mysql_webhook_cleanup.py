#!/usr/bin/env python3
"""Check webhook cleanup and its index on a disposable MySQL 8 database."""

import os
import re
import subprocess
import tempfile
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

from mysql_core import call
from mysql_webhooks import BINARY, ROOT, start, stop


MYSQL = Path("/tmp/rizzy-mysql8/bin/mysql")
SOCKET = Path("/tmp/rizzy-mysql8/mysql.sock")


def root_mysql(statement):
    env = {**os.environ, "LD_LIBRARY_PATH": str(MYSQL.resolve().parents[1] / "lib")}
    result = subprocess.run([str(MYSQL), "--no-defaults", "--batch",
                             "--skip-column-names", f"--socket={SOCKET}", "-uroot"],
                            input=statement, text=True, env=env,
                            capture_output=True, check=True)
    return result.stdout.strip()


def main():
    if not SOCKET.is_socket():
        raise SystemExit("Start the disposable MySQL 8 server first")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    trust_function_creators = root_mysql("SELECT @@GLOBAL.log_bin_trust_function_creators;")
    root_mysql("SET GLOBAL log_bin_trust_function_creators=1;")
    suffix = uuid.uuid4().hex[:12]
    database = "rzcleanup_" + suffix
    username = "rzcleanup_" + suffix
    password = uuid.uuid4().hex
    try:
        root_mysql(f"CREATE DATABASE {database} CHARACTER SET utf8mb4 COLLATE utf8mb4_bin; "
                   f"CREATE USER '{username}'@'127.0.0.1' IDENTIFIED BY '{password}'; "
                   f"GRANT ALL PRIVILEGES ON {database}.* TO '{username}'@'127.0.0.1';")
        url = f"mysql://{username}:{password}@127.0.0.1:33080/{database}"
        with tempfile.TemporaryDirectory(prefix="rizzy-mysql-webhook-cleanup-") as temporary:
            env = {**os.environ, "RIZZY_DATABASE_URL": url,
                   "RIZZY_STORAGE_DIR": str(Path(temporary) / "storage"),
                   "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
                   "RIZZY_DISABLE_WEBHOOK_CLEANUP": "1"}
            seed_result = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                         capture_output=True, text=True)
            assert seed_result.returncode == 0, seed_result.stderr[-1600:]
            seeded = seed_result.stdout
            account = re.search(r"Account: (\d+)", seeded).group(1)
            token = re.search(r"Bearer token: (\w+)", seeded).group(1)
            base, process, log = start(env, temporary, "fixture")
            try:
                scope = f"/{account}"
                board = call(base, token, scope + "/boards", "POST",
                             {"board": {"name": "MySQL cleanup"}}, 201)
                hook = call(base, token, scope + f"/boards/{board['id']}/webhooks",
                            "POST", {"webhook": {"name": "Cleanup",
                            "url": "https://example.invalid/hook",
                            "subscribed_actions": ["card_published"]}}, 201)
                call(base, token, scope + f"/boards/{board['id']}/cards", "POST",
                     {"card": {"title": "Keep event"}}, 201)
                rows = call(base, token, scope +
                            f"/boards/{board['id']}/webhooks/{hook['id']}/deliveries")
                assert len(rows) == 1 and rows[0]["state"] == "pending", rows
                delivery_id = rows[0]["id"]
            except Exception:
                print(log.read_text()[-1600:])
                raise
            finally:
                stop(process)
            old = (datetime.now(timezone.utc) - timedelta(days=8)).isoformat(
                timespec="milliseconds").replace("+00:00", "Z")
            recent = (datetime.now(timezone.utc) - timedelta(days=6)).isoformat(
                timespec="milliseconds").replace("+00:00", "Z")
            for count, stamp in ((503, old), (2, recent)):
                statement = (
                    "INSERT INTO webhook_deliveries(id,account_id,webhook_id,event_id,state,"
                    "request,response,created_at,updated_at) SELECT UUID(),account_id,"
                    "webhook_id,event_id,'completed',request,response,"
                    f"'{stamp}',updated_at FROM webhook_deliveries WHERE id='{delivery_id}';")
                root_mysql(f"USE {database}; " + statement * count)
            assert root_mysql(f"USE {database}; SELECT COUNT(*) FROM webhook_deliveries;") == "506"
            plan = root_mysql(
                f"USE {database}; EXPLAIN SELECT id FROM webhook_deliveries WHERE "
                f"created_at<'{recent}' ORDER BY created_at,id LIMIT 500;")
            assert "webhook_deliveries_stale" in plan, plan
            run = subprocess.run([BINARY, "cleanup-webhook-deliveries"], cwd=ROOT, env=env,
                                 capture_output=True, text=True, check=True)
            assert run.stdout.strip() == "Cleaned 503 webhook deliveries", run.stdout
            assert root_mysql(f"USE {database}; SELECT COUNT(*) FROM webhook_deliveries;") == "3"
            subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env,
                           capture_output=True, check=True)
            index_count = root_mysql(
                f"USE {database}; SELECT COUNT(*) FROM information_schema.statistics "
                "WHERE table_schema=DATABASE() AND table_name='webhook_deliveries' "
                "AND index_name='webhook_deliveries_stale' AND seq_in_index=1;")
            assert index_count == "1", index_count
            print("PASS MySQL webhook cleanup, stale index, and restart")
    finally:
        root_mysql(f"DROP DATABASE IF EXISTS {database}; "
                   f"DROP USER IF EXISTS '{username}'@'127.0.0.1';")
        root_mysql(f"SET GLOBAL log_bin_trust_function_creators={trust_function_creators};")


if __name__ == "__main__":
    main()
