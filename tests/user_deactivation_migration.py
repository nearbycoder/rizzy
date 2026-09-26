#!/usr/bin/env python3
"""Migrate an existing user table, then remove and rejoin a member."""
import hashlib
import os
import sqlite3
import subprocess
import tempfile
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from join import call


def main():
    subprocess.run(["cargo", "build", "--locked", "--quiet"], cwd=ROOT, check=True)
    schema = (ROOT / "schema.sql").read_text()
    old_schema = schema.replace("identity_id TEXT REFERENCES identities(id),", "identity_id TEXT NOT NULL REFERENCES identities(id),", 1)
    assert old_schema != schema
    user_times = "  created_at TEXT NOT NULL,\n  updated_at TEXT NOT NULL DEFAULT '',\n  UNIQUE(account_id, identity_id)"
    assert user_times in old_schema
    old_schema = old_schema.replace(user_times, "  created_at TEXT NOT NULL,\n  UNIQUE(account_id, identity_id)", 1)
    with tempfile.TemporaryDirectory() as temp:
        db_path = Path(temp) / "existing.sqlite3"
        stamp = "2026-01-01T00:00:00Z"
        with sqlite3.connect(db_path) as db:
            db.executescript(old_schema)
            db.execute("INSERT INTO accounts(id,external_account_id,name,created_at) VALUES('account',1,'Existing',?)", (stamp,))
            for identity, email in (("owner-identity", "owner@example.test"), ("member-identity", "member@example.test")):
                db.execute("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)", (identity, email, stamp))
            for user, identity, role in (("owner", "owner-identity", "owner"), ("old-member", "member-identity", "member")):
                db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES(?,'account',?,?,?,?)", (user, identity, user, role, stamp))
            db.execute("INSERT INTO boards(id,account_id,creator_id,name,created_at,updated_at) VALUES('board','account','owner','Everyone',?,?)", (stamp, stamp))
            db.execute("INSERT INTO accesses(board_id,user_id) VALUES('board','old-member')")
            db.execute("INSERT INTO accesses(board_id,user_id) VALUES('board','owner')")
            for identity, token in (("owner-identity", "owner-token"), ("member-identity", "member-token")):
                db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES(?,?,?,?,?,?)", (token, identity, hashlib.sha256(token.encode()).hexdigest(), "Migration", "write", stamp))
            old_sgid = db.execute("SELECT sgid FROM user_attachables WHERE user_id='old-member'").fetchone()[0]
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{db_path}"}
        port, server = start(env)
        try:
            with sqlite3.connect(db_path) as db:
                column = next(row for row in db.execute("PRAGMA table_info(users)") if row[1] == "identity_id")
                assert column[3] == 0, column
                assert db.execute("SELECT created_at,updated_at FROM users WHERE id='old-member'").fetchone() == (stamp, stamp)
                assert db.execute("SELECT sgid FROM user_attachables WHERE user_id='old-member'").fetchone()[0] == old_sgid
                assert db.execute("PRAGMA foreign_key_check").fetchall() == []
            invite, _ = call(port, "/1/account/join_code", bearer="owner-token")
            call(port, "/1/users/old-member", "DELETE", bearer="owner-token", status=204)
            with sqlite3.connect(db_path) as db:
                removed = db.execute("SELECT active,identity_id,updated_at FROM users WHERE id='old-member'").fetchone()
                assert removed[:2] == (0, None) and removed[2] > stamp
            joined, _ = call(port, f"/1/join/{invite['code']}", "POST", {"email_address": "member@example.test"}, bearer="member-token")
            assert joined == {"joined": True, "requires_name": True, "account_slug": "/1"}, joined
            with sqlite3.connect(db_path) as db:
                new = db.execute("SELECT id,role FROM users WHERE account_id='account' AND identity_id='member-identity' AND active=1").fetchone()
                assert new and new[0] != "old-member" and new[1] == "member"
                assert db.execute("SELECT updated_at FROM users WHERE id=?", (new[0],)).fetchone()[0]
                assert db.execute("SELECT COUNT(*) FROM accesses WHERE board_id='board' AND user_id=?", (new[0],)).fetchone()[0] == 1
                assert db.execute("SELECT COUNT(*) FROM user_attachables WHERE user_id=?", (new[0],)).fetchone()[0] == 1
                assert db.execute("PRAGMA foreign_key_check").fetchall() == []
        finally:
            stop(server)
        port, server = start(env)
        try:
            with sqlite3.connect(db_path) as db:
                assert db.execute("SELECT COUNT(*) FROM users WHERE identity_id='member-identity' AND active=1").fetchone()[0] == 1
                assert db.execute("PRAGMA foreign_key_check").fetchall() == []
        finally:
            stop(server)
    print("User migration preserved foreign keys and attachment IDs; removed member rejoined with a new user ID")


if __name__ == "__main__":
    main()
