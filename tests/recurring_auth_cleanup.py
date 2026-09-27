#!/usr/bin/env python3
"""Check explicit auth cleanup keeps fresh links and transfers."""

import os
import sqlite3
import subprocess
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def stamp(offset):
    return (datetime.now(timezone.utc) + offset).isoformat(
        timespec="milliseconds").replace("+00:00", "Z")


def main():
    subprocess.run(["cargo", "build", "--locked", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-recurring-auth-") as directory:
        database = Path(directory) / "rizzy.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                       capture_output=True, text=True, check=True)
        with sqlite3.connect(database) as db:
            identity = db.execute("SELECT id FROM identities WHERE email_address='owner@example.com'").fetchone()
            if identity is None:
                identity = db.execute("SELECT id FROM identities ORDER BY created_at LIMIT 1").fetchone()
            identity_id = identity[0]
            for name, expires in (("stale", stamp(-timedelta(hours=1))),
                                  ("fresh", stamp(timedelta(hours=1)))):
                db.execute("INSERT INTO magic_links(id,identity_id,code,purpose,expires_at,created_at) "
                           "VALUES(?,?,?,'sign_in',?,?)",
                           (f"magic-{name}", identity_id, name, expires, stamp(-timedelta(hours=2))))
            for name, created in (("stale", stamp(-timedelta(hours=5))),
                                  ("fresh", stamp(-timedelta(hours=3)))):
                db.execute("INSERT INTO identity_transfers(id,identity_id,token,created_at) VALUES(?,?,?,?)",
                           (f"transfer-{name}", identity_id, f"token-{name}", created))
        subprocess.run([BINARY, "cleanup-magic-links"], cwd=ROOT, env=env,
                       capture_output=True, text=True, check=True)
        with sqlite3.connect(database) as db:
            assert db.execute("SELECT id FROM magic_links ORDER BY id").fetchall() == [("magic-fresh",)]
            assert db.execute("SELECT COUNT(*) FROM identity_transfers").fetchone()[0] == 2
        subprocess.run([BINARY, "cleanup-transfers"], cwd=ROOT, env=env,
                       capture_output=True, text=True, check=True)
        with sqlite3.connect(database) as db:
            assert db.execute("SELECT id FROM identity_transfers ORDER BY id").fetchall() == [("transfer-fresh",)]
            assert db.execute("SELECT id FROM magic_links ORDER BY id").fetchall() == [("magic-fresh",)]
    print("Recurring auth cleanup retained fresh links and transfers")


if __name__ == "__main__":
    main()
