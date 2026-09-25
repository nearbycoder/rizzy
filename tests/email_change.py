#!/usr/bin/env python3
"""Verify email changes require a fresh, one-use confirmation token."""
import hashlib
import os
import re
import sqlite3
import subprocess
import tempfile
from pathlib import Path

from auth import BINARY, ROOT, cookie, start, stop
from join import call


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "email-change.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
        owner_token = re.search(r"Bearer token: (\w+)", seeded.stdout).group(1)
        port, server = start(env)
        try:
            identity, _ = call(port, "/my/identity", bearer=owner_token)
            owner_id = identity["accounts"][0]["user"]["id"]
            request_path = f"/{account}/users/{owner_id}/email_addresses"
            call(port, request_path, "POST", {"email_address": "new@example.com"}, status=401)
            call(port, f"/{account}/users/other/email_addresses", "POST", {"email_address": "new@example.com"}, bearer=owner_token, status=404)
            call(port, request_path, "POST", {}, bearer=owner_token, status=400)
            invalid, _ = call(port, request_path, "POST", {"email_address": "invalid"}, bearer=owner_token, status=422)
            assert invalid["error"] == "Please enter a valid email address"
            same, _ = call(port, request_path, "POST", {"email_address": "OWNER@EXAMPLE.COM"}, bearer=owner_token, status=422)
            assert same["error"] == "That is already your email address"
            with sqlite3.connect(database) as db:
                account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?", (int(account),)).fetchone()[0]
                db.execute("INSERT INTO identities(id,email_address,created_at) VALUES('another-identity','another@example.com','2026-01-01T00:00:00Z')")
                db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('another-user',?,'another-identity','Another','member','2026-01-01T00:00:00Z')", (account_id,))
            existing, _ = call(port, request_path, "POST", {"email_address": "another@example.com"}, bearer=owner_token, status=422)
            assert existing["error"] == "You already have a user in this account with that email address"
            _, headers = call(port, request_path, "POST", {"email_address": "expired@example.com"}, bearer=owner_token, status=201)
            expired_token = headers["X-Email-Change-Token"]
            expired_path = f"/{account}/users/{owner_id}/email_addresses/{expired_token}/confirmation"
            html, _ = call(port, expired_path)
            assert "Confirm email address" in html
            with sqlite3.connect(database) as db:
                db.execute("UPDATE email_change_requests SET expires_at='2025-01-01T00:00:00Z' WHERE token_hash=?", (hashlib.sha256(expired_token.encode()).hexdigest(),))
            error, _ = call(port, expired_path, "POST", status=422)
            assert error["error"] == "Token is invalid or has expired"
            _, headers = call(port, request_path, "POST", {"email_address": "new@example.com"}, bearer=owner_token, status=201)
            token = headers["X-Email-Change-Token"]
            confirm_path = f"/{account}/users/{owner_id}/email_addresses/{token}/confirmation"
            _, headers = call(port, confirm_path, "POST", status=204)
            new_session = cookie(headers, "session_token")
            changed, _ = call(port, f"/{account}/users/{owner_id}", cookies=[new_session])
            assert changed["email_address"] == "new@example.com"
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
            _, mail_headers = call(port, request_path, "POST", {"email_address": "delivered@example.com"}, cookies=[new_session], status=201)
            assert mail_headers.get("X-Email-Change-Token") is None
            message = mailbox.read_text()
            assert "To: delivered@example.com" in message
            assert f"/{account}/users/{owner_id}/email_addresses/" in message
            print("Email changes passed: validation, self-only request, expiry, one-use confirmation, and session switch")
        finally:
            stop(server)


if __name__ == "__main__":
    main()
