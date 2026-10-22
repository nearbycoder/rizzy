#!/usr/bin/env python3
"""Verify email changes require a fresh, one-use confirmation token."""
import base64
import hashlib
import hmac
import json
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import urllib.parse
import uuid
from pathlib import Path

from auth import BINARY, ROOT, cookie, start, stop
from join import call


SECRET = "0123456789abcdef" * 4


def expired_email_token(user_id, old_email, new_email):
    query = urllib.parse.urlencode({
        "old_email_address": old_email,
        "new_email_address": new_email,
        "expires_in": "1800",
    })
    gid = f"gid://fizzy/User/{user_id}?{query}"
    envelope = {"_rails": {"data": gid, "exp": "2025-01-01T00:00:00.000Z",
                            "pur": "change_email_address"}}
    serialized = json.dumps(envelope, separators=(",", ":"), ensure_ascii=False).replace("&", "\\u0026")
    encoded = base64.urlsafe_b64encode(serialized.encode()).decode()
    key = hashlib.pbkdf2_hmac("sha256", SECRET.encode(), b"signed_global_ids", 1000, 64)
    digest = hmac.new(key, encoded.encode(), hashlib.sha1).hexdigest()
    return f"{encoded}--{digest}"


def main():
    mysql_url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if mysql_url and not mysql_url.startswith("mysql:"):
        raise SystemExit("RIZZY_TEST_MYSQL_URL must be a dedicated MySQL database")
    suffix = "-" + uuid.uuid4().hex[:12] if mysql_url else ""
    def email(local):
        return f"{local}{suffix}@example.com"
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "email-change.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": mysql_url or f"sqlite://{database}",
               "RIZZY_SEED_EMAIL": email("owner"), "RIZZY_SECRET_KEY_BASE": SECRET}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
        owner_token = re.search(r"Bearer token: (\w+)", seeded.stdout).group(1)
        port, server = start(env)
        try:
            identity, _ = call(port, "/my/identity", bearer=owner_token)
            owner_id = identity["accounts"][0]["user"]["id"]
            request_path = f"/{account}/users/{owner_id}/email_addresses"
            call(port, request_path, "POST", {"email_address": email("new")}, status=406)
            call(port, f"/{account}/users/other/email_addresses", "POST", {"email_address": email("new")}, bearer=owner_token, status=404)
            call(port, request_path, "POST", {}, bearer=owner_token, status=400)
            invalid, _ = call(port, request_path, "POST", {"email_address": "invalid"}, bearer=owner_token, status=422)
            assert invalid["error"] == "Please enter a valid email address"
            same, _ = call(port, request_path, "POST", {"email_address": email("owner").upper()}, bearer=owner_token, status=422)
            assert same["error"] == "That is already your email address"
            if mysql_url:
                from mysql_core import mysql_scalar
                account_id = mysql_scalar(mysql_url, f"SELECT id FROM accounts WHERE external_account_id={int(account)}")
                other_identity, other_user = uuid.uuid4().hex, uuid.uuid4().hex
                mysql_scalar(mysql_url, f"INSERT INTO identities(id,email_address,created_at) VALUES('{other_identity}','{email('another')}','2026-01-01T00:00:00Z')")
                mysql_scalar(mysql_url, f"INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('{other_user}','{account_id}','{other_identity}','Another','member','2026-01-01T00:00:00Z')")
            else:
                with sqlite3.connect(database) as db:
                    account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?", (int(account),)).fetchone()[0]
                    db.execute("INSERT INTO identities(id,email_address,created_at) VALUES('another-identity',?,'2026-01-01T00:00:00Z')", (email("another"),))
                    db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('another-user',?,'another-identity','Another','member','2026-01-01T00:00:00Z')", (account_id,))
            existing, _ = call(port, request_path, "POST", {"email_address": email("another")}, bearer=owner_token, status=422)
            assert existing["error"] == "You already have a user in this account with that email address"
            expired_token = expired_email_token(owner_id, email("owner"), email("expired"))
            expired_path = f"/{account}/users/{owner_id}/email_addresses/{expired_token}/confirmation"
            html, _ = call(port, expired_path)
            assert "Confirm email change" in html
            error, _ = call(port, expired_path, "POST", status=422)
            assert error["error"] == "Token is invalid or has expired"
            _, headers = call(port, request_path, "POST", {"email_address": email("new")}, bearer=owner_token, status=201)
            token = headers["X-Email-Change-Token"]
            confirm_path = f"/{account}/users/{owner_id}/email_addresses/{token}/confirmation"
            _, headers = call(port, confirm_path, "POST", status=204)
            new_session = cookie(headers, "session_token")
            changed, _ = call(port, f"/{account}/users/{owner_id}", cookies=[new_session])
            assert changed["email_address"] == email("new")
            call(port, f"/{account}/users/{owner_id}", bearer=owner_token, status=401)
            error, _ = call(port, confirm_path, "POST", status=422)
            assert error["error"] == "Token is invalid or has expired"
            stop(server)
            mailbox = Path(temp) / "mail.txt"
            sendmail = Path(temp) / "sendmail"
            sendmail.write_text(f"#!/bin/sh\ncat > '{mailbox}'\n")
            sendmail.chmod(0o700)
            mail_env = {**env, "RIZZY_DEV_MAGIC_LINKS": "0", "RIZZY_SENDMAIL": str(sendmail), "RIZZY_MAIL_FROM": "rizzy@example.com"}
            port, server = start(mail_env)
            _, mail_headers = call(port, request_path, "POST", {"email_address": email("delivered")}, cookies=[new_session], status=201)
            assert mail_headers.get("X-Email-Change-Token") is None
            message = mailbox.read_text()
            assert "To: " + email("delivered") in message
            assert f"/{account}/users/{owner_id}/email_addresses/" in message
            print("Email changes passed: validation, self-only request, expiry, one-use confirmation, and session switch")
        finally:
            stop(server)
            if sys.exc_info()[0] is not None:
                print(server.stderr.read().decode()[-6000:], file=sys.stderr)


if __name__ == "__main__":
    main()
