#!/usr/bin/env python3
"""Verify member verification, name entry, and account scoping."""
import hashlib
import os
import re
import sqlite3
import subprocess
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from join import call


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        return None


def post_form(port, path, session, data=None):
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}",
        urllib.parse.urlencode(data or {}).encode(), method="POST",
        headers={"Cookie": "session_token=" + session, "Accept": "text/html", "Content-Type": "application/x-www-form-urlencoded"},
    )
    try:
        urllib.request.build_opener(NoRedirect).open(request, timeout=3)
    except urllib.error.HTTPError as response:
        assert response.code == 302, (path, response.code, response.read())
        return response.headers["Location"]
    raise AssertionError("expected redirect")


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "verification.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        seed = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seed.stdout).group(1)
        with sqlite3.connect(database) as db:
            db.execute("ALTER TABLE users DROP COLUMN verified_at")
        port, server = start(env)
        try:
            with sqlite3.connect(database) as db:
                account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?", (account,)).fetchone()[0]
                assert db.execute("SELECT verified_at FROM users WHERE role='owner'").fetchone()[0]
                db.execute("INSERT INTO identities(id,email_address,created_at) VALUES('verify-identity','verify@example.com','2026-01-01T00:00:00Z')")
                db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('verify-user',?,'verify-identity','verify@example.com','member','2026-01-01T00:00:00Z')", (account_id,))
                db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES('verify-token','verify-identity',?,'Test','write','2026-01-01T00:00:00Z')", (hashlib.sha256(b'verify-token').hexdigest(),))
                db.execute("INSERT INTO sessions(id,identity_id,token_hash,signup_pending,created_at) VALUES('verify-session-row','verify-identity',?,0,'2026-01-01T00:00:00Z')", (hashlib.sha256(b'verify-session').hexdigest(),))
            path = f"/{account}/users/verifications"
            call(port, path + "/new", status=406)
            call(port, path, "POST", status=406)
            page, _ = call(port, path + "/new", cookies=["session_token=verify-session"], accept="text/html")
            assert "<form" in page and path in page
            assert post_form(port, path, "verify-session") == f"/{account}/users/joins/new"
            with sqlite3.connect(database) as db:
                verified_at = db.execute("SELECT verified_at FROM users WHERE id='verify-user'").fetchone()[0]
                assert verified_at
            assert post_form(port, path, "verify-session") == f"/{account}/users/joins/new"
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT verified_at FROM users WHERE id='verify-user'").fetchone()[0] == verified_at
            page, _ = call(port, f"/{account}/users/joins/new", cookies=["session_token=verify-session"], accept="text/html")
            assert 'name="user[name]"' in page
            assert post_form(port, f"/{account}/users/joins", "verify-session", {"user[name]": "Verified Member"}) == "/"
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT name FROM users WHERE id='verify-user'").fetchone()[0] == "Verified Member"
        finally:
            stop(server)
    print("verification and browser join checks passed")


if __name__ == "__main__":
    main()
