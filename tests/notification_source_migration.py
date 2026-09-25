#!/usr/bin/env python3
"""Migrate an old Event-only notification schema and render Mention sources."""
import hashlib
import json
import os
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"
STAMP = "2026-09-25T10:00:00.000Z"


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    schema = (ROOT / "schema.sql").read_text()
    marker = "source_id TEXT NOT NULL,\n  source_type TEXT NOT NULL DEFAULT 'Event',"
    old_schema = schema.replace(marker, "source_id TEXT NOT NULL REFERENCES events(id) ON DELETE CASCADE,\n  source_type TEXT NOT NULL DEFAULT 'Event',")
    assert old_schema != schema
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "old-notifications.sqlite3"
        db = sqlite3.connect(database)
        db.executescript(old_schema)
        db.execute("INSERT INTO accounts(id,external_account_id,name,created_at) VALUES('account',1,'Migration',?)", (STAMP,))
        db.execute("INSERT INTO account_join_codes(id,account_id,code,created_at,updated_at) VALUES('join','account','migrate-code',?,?)", (STAMP, STAMP))
        db.execute("INSERT INTO identities(id,email_address,created_at) VALUES('identity','owner@example.com',?)", (STAMP,))
        db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('owner','account','identity','Owner','owner',?)", (STAMP,))
        token = "notification-migration-token"
        db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES('token','identity',?,'Migration','write',?)",
                   (hashlib.sha256(token.encode()).hexdigest(), STAMP))
        db.execute("INSERT INTO boards(id,account_id,creator_id,name,created_at,updated_at) VALUES('board','account','owner','Board',?,?)", (STAMP, STAMP))
        db.execute("INSERT INTO accesses(board_id,user_id) VALUES('board','owner')")
        for number, card_id, description in [(1, "card-event", "Event body"), (2, "card-mention", "Hello mention")]:
            db.execute("INSERT INTO cards(id,account_id,board_id,creator_id,number,title,description,created_at,updated_at,last_active_at) VALUES(?,'account','board','owner',?,? ,?,?,?,?)",
                       (card_id, number, f"Card {number}", description, STAMP, STAMP, STAMP))
        db.execute("INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,created_at) VALUES('event','account','board','card-event','owner','Card','card-event','card_published',?)", (STAMP,))
        db.execute("INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,created_at,updated_at) VALUES('old-notice','account','owner','owner','card-event','event','Event',?,?)", (STAMP, STAMP))
        db.commit()
        assert any(row[3] == "source_id" for row in db.execute("PRAGMA foreign_key_list(notifications)"))
        db.close()

        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}",
               "RIZZY_BIND": f"127.0.0.1:{port}", "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
        server = subprocess.Popen([BINARY], cwd=ROOT, env=env,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            for _ in range(100):
                try:
                    with urllib.request.urlopen(f"http://127.0.0.1:{port}/up", timeout=1):
                        break
                except OSError:
                    time.sleep(.05)
            else:
                raise RuntimeError(server.stderr.read().decode()[:1000])
            db = sqlite3.connect(database)
            assert db.execute("SELECT COUNT(*) FROM notifications WHERE id='old-notice'").fetchone()[0] == 1
            assert not any(row[3] == "source_id" for row in db.execute("PRAGMA foreign_key_list(notifications)"))
            db.execute("INSERT INTO mentions(id,account_id,mentionee_id,mentioner_id,source_type,source_id,created_at,updated_at) VALUES('mention','account','owner','owner','Card','card-mention',?,?)", (STAMP, STAMP))
            db.execute("INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,created_at,updated_at) VALUES('mention-notice','account','owner','owner','card-mention','mention','Mention',?,?)", (STAMP, STAMP))
            db.commit()
            request = urllib.request.Request(f"http://127.0.0.1:{port}/1/notifications",
                                             headers={"Authorization": f"Bearer {token}", "Accept": "application/json"})
            with urllib.request.urlopen(request, timeout=5) as response:
                notices = json.load(response)
            mention = next(notice for notice in notices if notice["id"] == "mention-notice")
            assert mention["source_type"] == "mention" and mention["title"] == "Owner @mentioned you"
            assert mention["body"] == "Hello mention"
            db.execute("DELETE FROM events WHERE id='event'")
            db.commit()
            assert db.execute("SELECT COUNT(*) FROM notifications WHERE id='old-notice'").fetchone()[0] == 0
            assert db.execute("SELECT COUNT(*) FROM notifications WHERE id='mention-notice'").fetchone()[0] == 1
            db.execute("DELETE FROM cards WHERE id='card-mention'")
            db.commit()
            assert db.execute("SELECT COUNT(*) FROM mentions WHERE id='mention'").fetchone()[0] == 0
            assert db.execute("SELECT COUNT(*) FROM notifications WHERE id='mention-notice'").fetchone()[0] == 0
            assert not db.execute("PRAGMA foreign_key_check").fetchall()
            db.close()
            print("Notification migration passed: Event row kept, Mention rendered, both source deletions cleaned up")
        finally:
            server.terminate()
            server.wait(timeout=5)


if __name__ == "__main__":
    main()
