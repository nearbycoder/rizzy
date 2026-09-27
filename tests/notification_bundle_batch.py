#!/usr/bin/env python3
"""Drain more than one due-bundle page, including equal end-time ties."""

import os
import re
import sqlite3
import subprocess
import tempfile
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

from auth import BINARY, ROOT


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-notification-batch-") as temporary:
        database = Path(temporary) / "rizzy.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}",
               "RIZZY_STORAGE_DIR": str(Path(temporary) / "storage"),
               "RIZZY_SENDMAIL": "/bin/true"}
        subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, check=True,
                       capture_output=True, text=True)
        ended = (datetime.now(timezone.utc) - timedelta(hours=1)).isoformat(
            timespec="milliseconds").replace("+00:00", "Z")
        started = (datetime.now(timezone.utc) - timedelta(hours=5)).isoformat(
            timespec="milliseconds").replace("+00:00", "Z")
        with sqlite3.connect(database) as db:
            account = db.execute("SELECT id FROM accounts LIMIT 1").fetchone()[0]
            for index in range(205):
                identity = str(uuid.uuid4())
                user = str(uuid.uuid4())
                db.execute("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)",
                           (identity, f"bundle-{index}@example.test", started))
                db.execute("INSERT INTO users(id,account_id,identity_id,name,created_at) "
                           "VALUES(?,?,?,?,?)",
                           (user, account, identity, f"Bundle {index}", started))
                db.execute("INSERT INTO notification_bundles(id,account_id,user_id,starts_at,"
                           "ends_at,status,created_at,updated_at) VALUES(?,?,?,?,?,0,?,?)",
                           (str(uuid.uuid4()), account, user, started, ended,
                            started, started))
        first = subprocess.run([BINARY, "deliver-notifications"], cwd=ROOT, env=env,
                               check=True, capture_output=True, text=True)
        assert first.stdout.strip() == "Delivered 205 notification bundles", first.stdout
        with sqlite3.connect(database) as db:
            assert db.execute("SELECT COUNT(*) FROM notification_bundles WHERE status=2")\
                .fetchone()[0] == 205
            assert db.execute("SELECT COUNT(*) FROM notification_bundles WHERE status=0")\
                .fetchone()[0] == 0
        repeat = subprocess.run([BINARY, "deliver-notifications"], cwd=ROOT, env=env,
                                check=True, capture_output=True, text=True)
        assert repeat.stdout.strip() == "Delivered 0 notification bundles", repeat.stdout
        with sqlite3.connect(database) as db:
            db.execute("UPDATE notification_bundles SET status=0")
        workers = [subprocess.Popen([BINARY, "deliver-notifications"], cwd=ROOT, env=env,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                   for _ in range(2)]
        counts = []
        for worker in workers:
            output, error = worker.communicate(timeout=20)
            assert worker.returncode == 0, error
            counts.append(int(re.search(r"Delivered (\d+) notification bundles", output).group(1)))
        assert sum(counts) == 205, counts
        with sqlite3.connect(database) as db:
            assert db.execute("SELECT COUNT(*) FROM notification_bundles WHERE status=2")\
                .fetchone()[0] == 205
        print("PASS due notification bundles drain across pages, ties, and two workers")


if __name__ == "__main__":
    main()
