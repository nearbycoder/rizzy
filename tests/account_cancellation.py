#!/usr/bin/env python3
"""Exercise cancellation access, 30-day retention, and scoped incineration."""
import hashlib
import json
import os
import re
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path

from auth import BINARY, ROOT, start, stop


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, newurl):
        return None


OPENER = urllib.request.build_opener(NoRedirect)


def request(port, path, method="GET", *, token=None, cookie=None, html=False):
    headers = {"Accept": "text/html" if html else "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if cookie:
        headers["Cookie"] = f"session_token={cookie}"
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", headers=headers,
                                 data=b"" if method == "POST" else None, method=method)
    try:
        response = OPENER.open(req, timeout=5)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


def seed(env, email):
    result = subprocess.run([BINARY, "seed"], cwd=ROOT, env={**env, "RIZZY_SEED_EMAIL": email},
                            capture_output=True, text=True, check=True)
    return int(re.search(r"Account: (\d+)", result.stdout).group(1)), re.search(r"Bearer token: (\w+)", result.stdout).group(1)


def stamp(days=0):
    return (datetime.now(timezone.utc) - timedelta(days=days)).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def main():
    subprocess.run(["cargo", "build", "--locked", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        db_path = root / "rizzy.sqlite3"
        storage = root / "storage"
        exports = root / "exports"
        storage.mkdir()
        exports.mkdir()
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{db_path}",
               "RIZZY_STORAGE_DIR": str(storage), "RIZZY_EXPORT_DIR": str(exports),
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
        first, token = seed(env, "first@example.test")
        second, other_token = seed(env, "second@example.test")
        assert (first, second) == (1, 2)
        cookie = "owner-cancellation-session"
        member_cookie = "member-cancellation-session"
        with sqlite3.connect(db_path) as db:
            ids = {slug: db.execute("SELECT a.id,u.id,i.id FROM accounts a JOIN users u ON u.account_id=a.id JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner'", (slug,)).fetchone()
                   for slug in (first, second)}
            account, owner, identity = ids[first]
            other_account, other_owner, _ = ids[second]
            now = stamp()
            db.execute("INSERT INTO sessions(id,identity_id,token_hash,created_at) VALUES(?,?,?,?)",
                       ("cancellation-session", identity, hashlib.sha256(cookie.encode()).hexdigest(), now))
            db.execute("INSERT INTO identities(id,email_address,created_at) VALUES('member-identity','member@example.test',?)", (now,))
            db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('member-user',?,'member-identity','Member','member',?)", (account, now))
            db.execute("INSERT INTO sessions(id,identity_id,token_hash,created_at) VALUES('member-session','member-identity',?,?)",
                       (hashlib.sha256(member_cookie.encode()).hexdigest(), now))
            for slug, account_id, creator in ((first, account, owner), (second, other_account, other_owner)):
                db.execute("INSERT INTO boards(id,account_id,creator_id,name,created_at,updated_at) VALUES(?,?,?,?,?,?)",
                           (f"board-{slug}", account_id, creator, f"Board {slug}", now, now))
                db.execute("INSERT INTO accesses(board_id,user_id) VALUES(?,?)", (f"board-{slug}", creator))
                db.execute("INSERT INTO cards(id,account_id,board_id,creator_id,number,title,created_at,updated_at,last_active_at) VALUES(?,?,?,?,?,?,?,?,?)",
                           (f"card-{slug}", account_id, f"board-{slug}", creator, 1, f"Card {slug}", now, now, now))
            db.execute("INSERT INTO comments(id,card_id,creator_id,body,created_at) VALUES('comment-1','card-1',?,'A comment',?)", (owner, now))
            db.execute("INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,created_at) VALUES('event-1',?,'board-1','card-1',?,'Card','card-1','created',?)", (account, owner, now))
            db.execute("INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,created_at,updated_at) VALUES('notice-1',?,?,?,'card-1','event-1',?,?)", (account, owner, owner, now, now))
            db.execute("INSERT INTO storage_blobs(id,account_id,storage_key,filename,content_type,byte_size,checksum,signed_id,attachable_sgid,upload_token_hash,upload_expires_at,created_at) VALUES('blob-1',?,'blob-key','file.txt','text/plain',4,'checksum','signed-blob','sgid-blob','upload-hash',?,?)", (account, stamp(-1), now))
            db.execute("INSERT INTO exports(id,account_id,user_id,kind,status,created_at,updated_at) VALUES('export-1',?,?,'account','completed',?,?)", (account, owner, now, now))
            assert db.execute("PRAGMA foreign_key_check").fetchall() == []
        (storage / "blob-key").write_bytes(b"file")
        (exports / "export-1.zip").write_bytes(b"archive")

        port, server = start(env)
        try:
            status, _, page = request(port, "/1/account/settings", cookie=cookie, html=True)
            assert status == 200 and b"Danger zone" not in page
            status, headers, _ = request(port, "/1/account/cancellation", "POST", cookie=cookie, html=True)
            assert status == 302 and headers["Location"] == "/session/menu"
            with sqlite3.connect(db_path) as db:
                assert db.execute("SELECT COUNT(*) FROM account_cancellations").fetchone()[0] == 0
                assert db.execute("SELECT COUNT(*) FROM cancellation_mail_jobs").fetchone()[0] == 0
        finally:
            stop(server)

        env["RIZZY_MULTI_TENANT"] = "1"
        port, server = start(env)
        try:
            status, _, page = request(port, "/1/account/settings", cookie=cookie, html=True)
            assert status == 200 and b"Danger zone" in page and b"Delete my account" in page
            status, _, member_page = request(port, "/1/account/settings", cookie=member_cookie, html=True)
            assert status == 200 and b"Danger zone" not in member_page
            assert request(port, "/1/account/cancellation", "POST", cookie=member_cookie, html=True)[0] == 403
            status, headers, _ = request(port, "/1/account/cancellation", "POST", cookie=cookie, html=True)
            assert status == 302 and headers["Location"] == "/session/menu"
            with sqlite3.connect(db_path) as db:
                assert db.execute("SELECT COUNT(*) FROM cancellation_mail_jobs WHERE status=0").fetchone()[0] == 1
            assert request(port, "/1/account/settings", cookie=cookie, html=True)[0] == 302
            assert request(port, "/1/account/settings.json", token=token)[0] == 403
            identity_status, _, identity_body = request(port, "/1/my/identity.json", token=token)
            assert identity_status == 403, (identity_status, identity_body)
            assert request(port, "/2/account/settings.json", token=other_token)[0] == 200
            status, _, page = request(port, "/session/menu", cookie=cookie, html=True)
            assert status == 200 and b"don\xe2\x80\x99t have any Rizzy accounts" in page
            status, _, identity_body = request(port, "/my/identity", token=token)
            assert status == 200 and json.loads(identity_body)["accounts"] == []
            with sqlite3.connect(db_path) as db:
                db.execute("UPDATE account_cancellations SET created_at=?", (stamp(29),))
        finally:
            stop(server)
        port, server = start(env)
        try:
            with sqlite3.connect(db_path) as db:
                assert db.execute("SELECT COUNT(*) FROM accounts WHERE id=?", (account,)).fetchone()[0] == 1
                db.execute("UPDATE account_cancellations SET created_at=?", (stamp(31),))
        finally:
            stop(server)
        port, server = start(env)
        try:
            for _ in range(100):
                with sqlite3.connect(db_path) as db:
                    if db.execute("SELECT COUNT(*) FROM accounts WHERE id=?", (account,)).fetchone()[0] == 0:
                        break
                time.sleep(.05)
            else:
                raise AssertionError("Cancelled account was not incinerated within five seconds")
            with sqlite3.connect(db_path) as db:
                assert db.execute("SELECT COUNT(*) FROM accounts WHERE id=?", (account,)).fetchone()[0] == 0
                assert db.execute("SELECT COUNT(*) FROM accounts WHERE id=?", (other_account,)).fetchone()[0] == 1
                assert db.execute("SELECT COUNT(*) FROM cards WHERE id='card-2'").fetchone()[0] == 1
                assert db.execute("SELECT COUNT(*) FROM cards WHERE id='card-1'").fetchone()[0] == 0
                assert db.execute("SELECT COUNT(*) FROM comments WHERE id='comment-1'").fetchone()[0] == 0
                assert db.execute("SELECT COUNT(*) FROM notifications WHERE id='notice-1'").fetchone()[0] == 0
                assert db.execute("SELECT COUNT(*) FROM cancellation_mail_jobs").fetchone()[0] == 0
                assert db.execute("PRAGMA foreign_key_check").fetchall() == []
            assert not (storage / "blob-key").exists()
            assert not (exports / "export-1.zip").exists()
            assert request(port, "/2/account/settings.json", token=other_token)[0] == 200
        finally:
            stop(server)
    print("Cancellation access, single-tenant guard, day-29 retention, day-31 scoped deletion and file cleanup passed")


if __name__ == "__main__":
    main()
