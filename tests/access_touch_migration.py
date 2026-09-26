#!/usr/bin/env python3
"""Check Access user/board touches and upgrade the old no-op update trigger."""
import os
import sqlite3
import tempfile
import time
from datetime import datetime, timezone

from auth import BINARY, start, stop
from search_html_parity import api, rust_target


def times(db, board_id, user_id):
    return db.execute("SELECT u.updated_at,b.updated_at,r.updated_at FROM users u JOIN boards b ON b.id=? "
                      "JOIN access_records r ON r.board_id=b.id AND r.user_id=u.id WHERE u.id=?",
                      (board_id, user_id)).fetchone()


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        board = api(target, "/boards", "POST", {"board": {"name": "Access touch migration"}}, 201)
        stop(process)

        with sqlite3.connect(target["db"]) as db:
            db.execute("DROP TRIGGER access_records_update")
            db.execute("CREATE TRIGGER access_records_update AFTER UPDATE OF involvement,accessed_at ON accesses BEGIN "
                       "UPDATE access_records SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') "
                       "WHERE board_id=NEW.board_id AND user_id=NEW.user_id; END")

        env = {**os.environ, "RIZZY_DATABASE_URL": "sqlite://" + target["db"]}
        _, restarted = start(env, binary=BINARY)
        try:
            with sqlite3.connect(target["db"]) as db:
                db.execute("PRAGMA foreign_keys=ON")
                definition = db.execute("SELECT sql FROM sqlite_master WHERE name='access_records_update'").fetchone()[0]
                assert "WHEN NEW.involvement IS NOT OLD.involvement" in definition
                account_id = db.execute("SELECT account_id FROM boards WHERE id=?", (board["id"],)).fetchone()[0]
                stamp = "2026-01-01T00:00:00.000Z"
                db.execute("INSERT INTO identities(id,email_address,created_at) VALUES('access-touch-identity','access-touch@example.test',?)", (stamp,))
                db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) "
                           "VALUES('access-touch-user',?,'access-touch-identity','Access Touch','member',?)",
                           (account_id, stamp))
                user_id = "access-touch-user"
                user_before = db.execute("SELECT updated_at FROM users WHERE id=?", (user_id,)).fetchone()[0]
                board_before = db.execute("SELECT updated_at FROM boards WHERE id=?", (board["id"],)).fetchone()[0]
                assert user_before == stamp

                time.sleep(.02)
                db.execute("INSERT INTO accesses(board_id,user_id,involvement) VALUES(?,?,'access_only')",
                           (board["id"], user_id))
                created = times(db, board["id"], user_id)
                assert created[0] > user_before and created[1] > board_before

                time.sleep(.02)
                db.execute("UPDATE accesses SET involvement='watching' WHERE board_id=? AND user_id=?",
                           (board["id"], user_id))
                changed = times(db, board["id"], user_id)
                assert all(new > old for new, old in zip(changed, created))

                time.sleep(.02)
                db.execute("UPDATE accesses SET involvement='watching' WHERE board_id=? AND user_id=?",
                           (board["id"], user_id))
                assert times(db, board["id"], user_id) == changed

                time.sleep(.02)
                accessed_at = datetime.now(timezone.utc).isoformat(timespec="milliseconds")
                db.execute("UPDATE accesses SET accessed_at=? WHERE board_id=? AND user_id=?",
                           (accessed_at, board["id"], user_id))
                accessed = times(db, board["id"], user_id)
                assert all(new > old for new, old in zip(accessed, changed))

                time.sleep(.02)
                db.execute("UPDATE accesses SET accessed_at=? WHERE board_id=? AND user_id=?",
                           (accessed_at, board["id"], user_id))
                assert times(db, board["id"], user_id) == accessed

                time.sleep(.02)
                db.execute("DELETE FROM accesses WHERE board_id=? AND user_id=?", (board["id"], user_id))
                removed = db.execute("SELECT u.updated_at,b.updated_at FROM users u JOIN boards b ON b.id=? "
                                     "WHERE u.id=?", (board["id"], user_id)).fetchone()
                assert removed[0] > accessed[0] and removed[1] > accessed[1]
                assert db.execute("SELECT 1 FROM access_records WHERE user_id=?", (user_id,)).fetchone() is None
                assert db.execute("PRAGMA foreign_key_check").fetchall() == []
        finally:
            stop(restarted)
    print("Access touch migration passed: create, involvement, access time, no-op, and delete times")


if __name__ == "__main__":
    main()
