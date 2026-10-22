#!/usr/bin/env python3
"""Compare sign-in and sign-up verification email content with pinned Fizzy."""

from email import policy
from email.parser import BytesParser
import json
import os
import re
import subprocess
import sys
import tempfile
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
      owner = Identity.find_by!(email_address: "owner@example.com")
      fresh = Identity.create!(email_address: "new@example.test")
      [[owner, "sign_in", "ABC123"], [fresh, "sign_up", "XYZ789"]].each do |identity, purpose, code|
        link = MagicLink.create!(identity: identity, purpose: purpose, code: code)
        mail = MagicLinkMailer.sign_in_instructions(link).message
        puts JSON.generate({ purpose: purpose, subject: mail.subject,
          plain: mail.text_part.body.decoded, html: mail.html_part.body.decoded })
      end'''
    output = subprocess.run(["bin/rails", "runner", ruby], cwd=SOURCE, env=env,
                            capture_output=True, text=True, check=True).stdout
    return {item["purpose"]: item for line in output.splitlines() if line.startswith("{")
            for item in [json.loads(line)]}


def rust_messages(directory):
    mailbox = directory / "last-message.eml"
    sendmail = directory / "sendmail"
    sendmail.write_text(f"#!/bin/sh\ncat > '{mailbox}'\n")
    sendmail.chmod(0o700)
    env = rust_env(directory)
    env.update({"RIZZY_MULTI_TENANT": "1", "RIZZY_SENDMAIL": str(sendmail),
                "RIZZY_MAIL_FROM": "support@example.test",
                "RIZZY_SUPPORT_EMAIL": "support@fizzy.do"})
    env, _ = prepare_rust(directory, env)
    process, url = start_server("rizzy", directory, env, sorted(os.sched_getaffinity(0))[:2])
    results = {}
    try:
        for purpose, address in (("sign_in", "owner@example.com"),
                                 ("sign_up", "new@example.test")):
            request = urllib.request.Request(
                url + "/session", method="POST",
                data=json.dumps({"email_address": address}).encode(),
                headers={"Accept": "application/json", "Content-Type": "application/json"})
            with urllib.request.urlopen(request, timeout=15) as response:
                assert response.status == 201, (purpose, response.status)
            message = BytesParser(policy=policy.default).parsebytes(mailbox.read_bytes())
            assert message.get_content_type() == "multipart/alternative", purpose
            plain = message.get_body(preferencelist=("plain",)).get_content()
            html = message.get_body(preferencelist=("html",)).get_content()
            assert message["To"] == address, (purpose, message["To"])
            code = re.search(r"Your Rizzy code is ([A-Z0-9]{6})", message["Subject"])
            assert code, (purpose, message["Subject"])
            results[purpose] = {"purpose": purpose, "subject": str(message["Subject"]),
                                "plain": plain, "html": html, "code": code.group(1)}
            mailbox.unlink()
    finally:
        stop_server(process)
    return results


def normalize(value, code):
    value = value.replace(code, "SIXCODE").replace("Fizzy", "Rizzy")
    value = re.sub(r"https?://[^/\s\"<>]+(?=/1/my/passkeys)", "ORIGIN", value)
    value = value.replace("\r\n", "\n")
    return value


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-magic-mail-pair-") as temporary:
        root = Path(temporary)
        source_dir, rust_dir = root / "fizzy", root / "rizzy"
        source_dir.mkdir()
        rust_dir.mkdir()
        source, rust = source_messages(source_dir), rust_messages(rust_dir)
        assert source.keys() == rust.keys() == {"sign_in", "sign_up"}
        for purpose in source:
            expected = source[purpose]
            actual = rust[purpose]
            for field in ("subject", "plain", "html"):
                left = normalize(expected[field], "ABC123" if purpose == "sign_in" else "XYZ789")
                right = normalize(actual[field], actual["code"])
                if field == "html":
                    left = re.sub(r"\s+", " ", left).strip()
                    right = re.sub(r"\s+", " ", right).strip()
                else:
                    left = left.strip()
                    right = right.strip()
                assert left == right, (purpose, field, left[-500:], right[-500:])
        print("PASS pinned-source sign-in and sign-up verification mail content")


if __name__ == "__main__":
    main()
