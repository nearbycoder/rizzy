#!/usr/bin/env python3
"""Compare real Rizzy export completion mail with the pinned Fizzy mailer."""

from email import policy
from email.parser import BytesParser
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from sustained_mixed import (SOURCE, prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def source_messages(directory):
    env, _ = prepare_source(directory, 2)
    env = {**env, "BASE_URL": "https://example.test"}
    ruby = '''require "json"
      owner = Identity.find_by!(email_address: "owner@example.com").users.first
      [User::DataExport, Account::Export].each do |klass|
        export = klass.create!(account: owner.account, user: owner)
        Current.set(account: owner.account) do
          mail = ExportMailer.completed(export).message
          puts JSON.generate({kind: klass == User::DataExport ? "user" : "account",
            subject: mail.subject, plain: mail.text_part.body.decoded,
            html: mail.html_part.body.decoded})
        end
      end'''
    output = subprocess.run(["bin/rails", "runner", ruby], cwd=SOURCE, env=env,
                            capture_output=True, text=True, check=True).stdout
    return {item["kind"]: item for line in output.splitlines() if line.startswith("{")
            for item in [json.loads(line)]}


def request(url, token, path, method="GET"):
    call = urllib.request.Request(
        url + path, method=method,
        headers={"Authorization": f"Bearer {token}", "Accept": "application/json"})
    with urllib.request.urlopen(call, timeout=20) as response:
        body = response.read()
        assert response.status in (200, 201), (path, response.status, body[:200])
        return json.loads(body)


def rust_messages(directory):
    mailbox = directory / "last-message.eml"
    sendmail = directory / "sendmail"
    sendmail.write_text(f"#!/bin/sh\ncat > '{mailbox}'\n")
    sendmail.chmod(0o700)
    env = rust_env(directory)
    env.update({"RIZZY_SENDMAIL": str(sendmail),
                "RIZZY_MAIL_FROM": "support@example.test",
                "RIZZY_SUPPORT_EMAIL": "support@fizzy.do",
                "RIZZY_PUBLIC_URL": "https://example.test",
                "RIZZY_EXPORT_DIR": str(directory / "exports")})
    env, seed = prepare_rust(directory, env)
    process, url = start_server("rizzy", directory, env, sorted(os.sched_getaffinity(0))[:2])
    results = {}
    try:
        account, token = seed["account"], seed["token"]
        users = request(url, token, f"/{account}/users")
        owner = next(user for user in users if user["role"] == "owner")
        for kind, create_path in (
            ("user", f"/{account}/users/{owner['id']}/data_exports"),
            ("account", f"/{account}/account/exports"),
        ):
            created = request(url, token, create_path, "POST")
            show_path = f"{create_path}/{created['id']}"
            for _ in range(200):
                state = request(url, token, show_path)
                if state["status"] in ("completed", "failed") and mailbox.exists():
                    break
                time.sleep(.1)
            assert state["status"] == "completed" and mailbox.exists(), (kind, state)
            message = BytesParser(policy=policy.default).parsebytes(mailbox.read_bytes())
            assert message.get_content_type() == "multipart/alternative", kind
            assert message["To"] == "owner@example.com", (kind, message["To"])
            results[kind] = {"subject": str(message["Subject"]),
                             "plain": message.get_body(preferencelist=("plain",)).get_content(),
                             "html": message.get_body(preferencelist=("html",)).get_content()}
            emailed = re.search(r"https://example\.test(/[^\s]+)", results[kind]["plain"])
            assert emailed, (kind, results[kind]["plain"])
            assert request(url, token, emailed.group(1))["status"] == "completed", kind
            mailbox.unlink()
    finally:
        stop_server(process)
    return results


def normalize(value, kind):
    value = value.replace("Fizzy", "Rizzy").replace("\r\n", "\n")
    if kind == "user":
        value, count = re.subn(r"/1/users/[^/\s\"<>]+/data_exports/[^/\s\"<>]+",
                               "/1/users/USER/data_exports/EXPORT", value)
    else:
        value, count = re.subn(r"/1/account/exports/[^/\s\"<>]+",
                               "/1/account/exports/EXPORT", value)
    assert count == 1, (kind, count, value[-500:])
    return value


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-export-mail-pair-") as temporary:
        root = Path(temporary)
        source_dir, rust_dir = root / "fizzy", root / "rizzy"
        source_dir.mkdir()
        rust_dir.mkdir()
        source, rust = source_messages(source_dir), rust_messages(rust_dir)
        assert source.keys() == rust.keys() == {"user", "account"}
        for kind in source:
            assert source[kind]["subject"].replace("Fizzy", "Rizzy") == rust[kind]["subject"]
            for field in ("plain", "html"):
                left, right = normalize(source[kind][field], kind), normalize(rust[kind][field], kind)
                if field == "html":
                    left, right = re.sub(r"\s+", " ", left).strip(), re.sub(r"\s+", " ", right).strip()
                else:
                    left, right = left.strip(), right.strip()
                assert left == right, (kind, field, left[-500:], right[-500:])
        print("PASS pinned-source user and account export completion mail")


if __name__ == "__main__":
    main()
