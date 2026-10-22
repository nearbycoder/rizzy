#!/usr/bin/env python3
"""Compare cancellation notice mail from Rizzy's queue with the pinned mailer."""

from email import policy
from email.parser import BytesParser
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from sustained_mixed import (RUST, SOURCE, prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def source_message(directory):
    env, _ = prepare_source(directory, 2)
    ruby = '''require "json"
      identity = Identity.find_by!(email_address: "owner@example.com")
      user = identity.users.first
      account = user.account
      cancellation = Account::Cancellation.create!(account: account, initiated_by: user)
      Current.set(account: account) do
        mail = AccountMailer.cancellation(cancellation).message
        puts JSON.generate({subject: mail.subject, plain: mail.text_part.body.decoded,
          html: mail.html_part.body.decoded})
      end'''
    output = subprocess.run(["bin/rails", "runner", ruby], cwd=SOURCE, env=env,
                            capture_output=True, text=True, check=True).stdout
    return next(json.loads(line) for line in output.splitlines() if line.startswith("{"))


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, newurl):
        return None


def rust_message(directory):
    env = rust_env(directory)
    env["RIZZY_MULTI_TENANT"] = "1"
    env.pop("RIZZY_SENDMAIL", None)
    env, seed = prepare_rust(directory, env)
    process, url = start_server("rizzy", directory, env, sorted(os.sched_getaffinity(0))[:2])
    try:
        request = urllib.request.Request(
            f"{url}/{seed['account']}/account/cancellation", method="POST", data=b"",
            headers={"Authorization": f"Bearer {seed['token']}",
                     "Accept": "application/json"})
        try:
            response = urllib.request.build_opener(NoRedirect).open(request, timeout=15)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            assert response.status == 302, response.status
            assert response.headers["Location"] == "/session/menu"
    finally:
        stop_server(process)
    mailbox = directory / "notice.eml"
    sendmail = directory / "sendmail"
    sendmail.write_text(f"#!/bin/sh\ncat > '{mailbox}'\n")
    sendmail.chmod(0o700)
    env.update({"RIZZY_SENDMAIL": str(sendmail),
                "RIZZY_MAIL_FROM": "support@example.test",
                "RIZZY_SUPPORT_EMAIL": "support@fizzy.do"})
    delivered = subprocess.run([RUST, "deliver-cancellation-mail"], cwd=ROOT,
                               env=env, capture_output=True, text=True, check=True)
    assert "Delivered 1" in delivered.stdout and mailbox.exists(), delivered.stdout
    message = BytesParser(policy=policy.default).parsebytes(mailbox.read_bytes())
    assert message.get_content_type() == "multipart/alternative"
    assert message["To"] == "owner@example.com", message["To"]
    repeated = subprocess.run([RUST, "deliver-cancellation-mail"], cwd=ROOT,
                              env=env, capture_output=True, text=True, check=True)
    assert "Delivered 0" in repeated.stdout, repeated.stdout
    return {"subject": str(message["Subject"]),
            "plain": message.get_body(preferencelist=("plain",)).get_content(),
            "html": message.get_body(preferencelist=("html",)).get_content()}


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-cancel-mail-pair-") as temporary:
        root = Path(temporary)
        source_dir, rust_dir = root / "fizzy", root / "rizzy"
        source_dir.mkdir()
        rust_dir.mkdir()
        source, rust = source_message(source_dir), rust_message(rust_dir)
        assert source["subject"].replace("Fizzy", "Rizzy") == rust["subject"]
        for field in ("plain", "html"):
            left = source[field].replace("Fizzy", "Rizzy").replace("\r\n", "\n")
            right = rust[field].replace("\r\n", "\n")
            if field == "html":
                left, right = re.sub(r"\s+", " ", left).strip(), re.sub(r"\s+", " ", right).strip()
            else:
                left, right = left.strip(), right.strip()
            assert left == right, (field, left[-600:], right[-600:])
        print("PASS pinned-source account cancellation mail content and queue delivery")


if __name__ == "__main__":
    main()
