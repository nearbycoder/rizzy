#!/usr/bin/env python3
"""Compare the live email-change message and confirmation link with Fizzy."""

from email import policy
from email.parser import BytesParser
import json
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from sustained_mixed import (SOURCE, prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def source_message(directory):
    env, _ = prepare_source(directory, 2)
    env = {**env, "BASE_URL": "https://example.test"}
    ruby = '''require "json"
      user = Identity.find_by!(email_address: "owner@example.com").users.first
      Current.set(account: user.account) do
        mail = UserMailer.email_change_confirmation(
          email_address: "new@example.test", token: "TESTTOKEN", user: user).message
        puts JSON.generate({subject: mail.subject, content_type: mail.content_type,
          html: mail.body.decoded})
      end'''
    output = subprocess.run(["bin/rails", "runner", ruby], cwd=SOURCE, env=env,
                            capture_output=True, text=True, check=True).stdout
    return next(json.loads(line) for line in output.splitlines() if line.startswith("{"))


def request(url, path, method="GET", data=None, token=None):
    headers = {"Accept": "application/json"}
    if data is not None:
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = f"Bearer {token}"
    call = urllib.request.Request(url + path, method=method,
                                  data=json.dumps(data).encode() if data is not None else None,
                                  headers=headers)
    with urllib.request.urlopen(call, timeout=15) as response:
        body = response.read()
        return response.status, json.loads(body) if body and response.headers.get_content_type() == "application/json" else body


def rust_message(directory):
    mailbox = directory / "mail.eml"
    sendmail = directory / "sendmail"
    sendmail.write_text(f"#!/bin/sh\ncat > '{mailbox}'\n")
    sendmail.chmod(0o700)
    env = rust_env(directory)
    env.update({"RIZZY_DEV_MAGIC_LINKS": "0", "RIZZY_SENDMAIL": str(sendmail),
                "RIZZY_MAIL_FROM": "support@example.test",
                "RIZZY_SUPPORT_EMAIL": "support@fizzy.do"})
    env, seed = prepare_rust(directory, env)
    process, url = start_server("rizzy", directory, env, sorted(os.sched_getaffinity(0))[:2])
    try:
        account, token = seed["account"], seed["token"]
        status, users = request(url, f"/{account}/users", token=token)
        assert status == 200
        owner = next(user for user in users if user["role"] == "owner")
        path = f"/{account}/users/{owner['id']}/email_addresses"
        status, _ = request(url, path, "POST", {"email_address": "new@example.test"}, token)
        assert status == 201 and mailbox.exists(), status
        message = BytesParser(policy=policy.default).parsebytes(mailbox.read_bytes())
        assert message.get_content_type() == "text/html", message.get_content_type()
        assert not message.is_multipart()
        assert message["To"] == "new@example.test", message["To"]
        html = message.get_content()
        link = re.search(r'<a class="btn" href="([^"]+)">Yes use this email address</a>', html)
        assert link, html[-800:]
        local_path = re.sub(r"^https?://[^/]+", "", link.group(1))
        assert local_path.startswith(f"/{account}/users/{owner['id']}/email_addresses/")
        status, _ = request(url, local_path)
        assert status == 200
        status, _ = request(url, local_path, "POST")
        assert status == 204
        with sqlite3.connect(directory / "rizzy.sqlite3") as db:
            changed = db.execute("SELECT i.email_address FROM users u JOIN identities i ON i.id=u.identity_id WHERE u.id=?", (owner["id"],)).fetchone()[0]
        assert changed == "new@example.test", changed
        return {"subject": str(message["Subject"]), "html": html}
    finally:
        stop_server(process)


def normalize(value):
    value = value.replace("\r\n", "\n")
    value, count = re.subn(
        r"https?://[^/\s\"<>]+/1/users/[^/\s\"<>]+/email_addresses/[^/\s\"<>]+/confirmation",
        "CONFIRMATION_LINK", value)
    assert count == 1, (count, value[-500:])
    return re.sub(r"\s+", " ", value).strip()


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-email-change-mail-pair-") as temporary:
        root = Path(temporary)
        source_dir, rust_dir = root / "fizzy", root / "rizzy"
        source_dir.mkdir()
        rust_dir.mkdir()
        source, rust = source_message(source_dir), rust_message(rust_dir)
        assert source["content_type"].startswith("text/html")
        assert source["subject"] == rust["subject"]
        assert normalize(source["html"]) == normalize(rust["html"])
        print("PASS pinned-source email-change HTML mail and live confirmation link")


if __name__ == "__main__":
    main()
