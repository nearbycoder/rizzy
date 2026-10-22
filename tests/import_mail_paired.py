#!/usr/bin/env python3
"""Compare import worker mail with the pinned source mailer."""

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
from account_import import await_result, upload  # noqa: E402


def source_messages(directory):
    env, _ = prepare_source(directory, 2)
    env = {**env, "BASE_URL": "https://example.test"}
    ruby = '''require "json"
      identity = Identity.find_by!(email_address: "owner@example.com")
      account = identity.users.first.account
      Current.set(account: account) do
        mail = ImportMailer.completed(identity, account).message
        puts JSON.generate({kind: "completed", subject: mail.subject,
          plain: mail.text_part.body.decoded, html: mail.html_part.body.decoded})
        [nil, :conflict, :invalid_export, :insufficient_storage_space].each do |reason|
          import = Account::Import.create!(identity: identity, account: account,
            status: :failed, failure_reason: reason)
          mail = ImportMailer.failed(import).message
          puts JSON.generate({kind: (reason || :other).to_s, subject: mail.subject,
            plain: mail.text_part.body.decoded, html: mail.html_part.body.decoded})
        end
      end'''
    output = subprocess.run(["bin/rails", "runner", ruby], cwd=SOURCE, env=env,
                            capture_output=True, text=True, check=True).stdout
    return {item["kind"]: item for line in output.splitlines() if line.startswith("{")
            for item in [json.loads(line)]}


def request(url, token, path, method="GET", raw=False):
    call = urllib.request.Request(
        url + path, method=method,
        headers={"Authorization": f"Bearer {token}", "Accept": "application/json"})
    with urllib.request.urlopen(call, timeout=30) as response:
        body = response.read()
        assert response.status in (200, 201), (path, response.status, body[:200])
        return body if raw else json.loads(body)


def export_archive(directory):
    env = rust_env(directory)
    env["RIZZY_EXPORT_DIR"] = str(directory / "exports")
    env.pop("RIZZY_SENDMAIL", None)
    env, seed = prepare_rust(directory, env)
    process, url = start_server("rizzy", directory, env, sorted(os.sched_getaffinity(0))[:2])
    try:
        account, token = seed["account"], seed["token"]
        created = request(url, token, f"/{account}/account/exports", "POST")
        for _ in range(200):
            state = request(url, token, f"/{account}/account/exports/{created['id']}")
            if state["status"] in ("completed", "failed"):
                break
            time.sleep(.1)
        assert state["status"] == "completed", state
        return request(url, token, f"/{account}/exports/{created['id']}/download", raw=True)
    finally:
        stop_server(process)


def rust_messages(directory, archive):
    mailbox = directory / "last-message.eml"
    sendmail = directory / "sendmail"
    sendmail.write_text(f"#!/bin/sh\ncat > '{mailbox}'\n")
    sendmail.chmod(0o700)
    env = rust_env(directory)
    env.update({"RIZZY_IMPORT_DIR": str(directory / "imports"),
                "RIZZY_SENDMAIL": str(sendmail),
                "RIZZY_MAIL_FROM": "support@example.test",
                "RIZZY_SUPPORT_EMAIL": "support@fizzy.do",
                "RIZZY_PUBLIC_URL": "https://example.test"})
    env, seed = prepare_rust(directory, env)
    process, url = start_server("rizzy", directory, env, sorted(os.sched_getaffinity(0))[:2])
    messages = {}
    try:
        for kind, data in (("completed", archive), ("conflict", archive),
                           ("invalid_export", b"not a zip archive")):
            created = upload(url, seed["token"], data)
            state = await_result(url, seed["token"], created)
            assert state["status"] == ("completed" if kind == "completed" else "failed"), (kind, state)
            if kind != "completed":
                assert state["failure_reason"] == kind, (kind, state)
            for _ in range(100):
                if mailbox.exists():
                    break
                time.sleep(.1)
            assert mailbox.exists(), (kind, state)
            message = BytesParser(policy=policy.default).parsebytes(mailbox.read_bytes())
            assert message.get_content_type() == "multipart/alternative", kind
            assert message["To"] == "owner@example.com", (kind, message["To"])
            messages[kind] = {"subject": str(message["Subject"]),
                              "plain": message.get_body(preferencelist=("plain",)).get_content(),
                              "html": message.get_body(preferencelist=("html",)).get_content()}
            if kind == "completed":
                assert f"/{created['account']}/landing" in messages[kind]["plain"]
            mailbox.unlink()
    finally:
        stop_server(process)
    return messages


def normalize(value):
    value = value.replace("Fizzy", "Rizzy").replace("\r\n", "\n")
    return re.sub(r"https://example\.test/\d+/landing", "ORIGIN/ACCOUNT/landing", value)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-import-mail-pair-") as temporary:
        root = Path(temporary)
        source_dir, exporter_dir, importer_dir = (root / name for name in ("fizzy", "exporter", "importer"))
        for directory in (source_dir, exporter_dir, importer_dir):
            directory.mkdir()
        source = source_messages(source_dir)
        archive = export_archive(exporter_dir)
        rust = rust_messages(importer_dir, archive)
        assert set(rust) == {"completed", "conflict", "invalid_export"}
        for kind in rust:
            assert normalize(source[kind]["subject"]) == rust[kind]["subject"]
            for field in ("plain", "html"):
                left, right = normalize(source[kind][field]), normalize(rust[kind][field])
                if field == "html":
                    left, right = re.sub(r"\s+", " ", left).strip(), re.sub(r"\s+", " ", right).strip()
                else:
                    left, right = left.strip(), right.strip()
                assert left == right, (kind, field, left[-500:], right[-500:])
        print("PASS pinned-source import completion and failure mail")


if __name__ == "__main__":
    main()
