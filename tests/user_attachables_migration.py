#!/usr/bin/env python3
"""Backfill opaque User attachment IDs in an existing Rizzy database."""
import os
import sqlite3
import subprocess
import tempfile
from pathlib import Path

from auth import BINARY, ROOT, start, stop


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    schema = (ROOT / "schema.sql").read_text()
    start_at = schema.index("CREATE TABLE IF NOT EXISTS user_attachables")
    end_at = schema.index("CREATE TABLE IF NOT EXISTS access_tokens", start_at)
    old_schema = schema[:start_at] + schema[end_at:]
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "existing.sqlite3"
        with sqlite3.connect(database) as db:
            db.executescript(old_schema)
            db.execute("INSERT INTO accounts(id,external_account_id,name,created_at) VALUES('account',1,'Existing','2026-01-01T00:00:00Z')")
            db.execute("INSERT INTO identities(id,email_address,created_at) VALUES('identity','owner@example.com','2026-01-01T00:00:00Z')")
            db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('owner','account','identity','Owner','owner','2026-01-01T00:00:00Z')")
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        first = None
        for _ in range(2):
            _, server = start(env)
            try:
                with sqlite3.connect(database) as db:
                    token = db.execute("SELECT sgid FROM user_attachables WHERE user_id='owner'").fetchone()[0]
                    assert len(token) == 64
                    assert db.execute("SELECT COUNT(*) FROM user_attachables").fetchone()[0] == db.execute("SELECT COUNT(*) FROM users").fetchone()[0]
                    if first is not None:
                        assert token == first
                    first = token
            finally:
                stop(server)
    print("Existing User attachment IDs backfilled once and stable across restart")


if __name__ == "__main__":
    main()
