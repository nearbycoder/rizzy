#!/usr/bin/env python3
"""Verify live Rizzy email links use Fizzy's signed GlobalID format."""

import http.cookiejar
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import (SOURCE, prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def rails(env, script, **variables):
    result = subprocess.run(["bin/rails", "runner", script], cwd=SOURCE,
                            env={**env, **variables}, capture_output=True, text=True)
    assert result.returncode == 0, result.stderr[-1200:]
    return result.stdout.strip()


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-email-sgid-") as temporary:
        directory = Path(temporary)
        source_dir = directory / "fizzy"
        rust_dir = directory / "rizzy"
        source_dir.mkdir()
        rust_dir.mkdir()
        source_env, _ = prepare_source(source_dir, 1)
        env = rust_env(rust_dir)
        env["RIZZY_SECRET_KEY_BASE"] = source_env["SECRET_KEY_BASE"]
        env["RIZZY_DEV_MAGIC_LINKS"] = "1"
        env, credentials = prepare_rust(rust_dir, env)
        process, url = start_server("rizzy", rust_dir, env, sorted(os.sched_getaffinity(0))[:2])
        try:
            account = credentials["account"]
            target = {"name": "rizzy", "url": url, "account": account,
                      "email": "owner@example.com", "token": credentials["token"],
                      "opener": urllib.request.build_opener(
                          urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                          NoRedirect())}
            login(target)
            status, _, body = request(target, f"/{account}/users", bearer=True)
            assert status == 200, status
            owner = next(user for user in json.loads(body) if user["role"] == "owner")
            user_id = owner["id"]
            path = f"/{account}/users/{user_id}/email_addresses"
            status, headers, _ = request(target, path, "POST",
                                         {"email_address": "from-rust@example.com"}, bearer=True)
            assert status == 201, status
            rust_token = headers["X-Email-Change-Token"]
            parsed = rails(source_env,
                           'require "json"; token = SignedGlobalID.parse(ENV.fetch("SIGNED_TOKEN"), for: "change_email_address"); puts JSON.generate({ id: token&.model_id, old: token&.params&.fetch("old_email_address"), new: token&.params&.fetch("new_email_address") })',
                           SIGNED_TOKEN=rust_token)
            assert json.loads(parsed) == {"id": user_id, "old": "owner@example.com",
                                          "new": "from-rust@example.com"}, parsed

            new_email = "from-source@example.com"
            source_token = rails(source_env,
                                 'user = User.new(id: ENV.fetch("TEST_USER_ID")); puts user.to_sgid(for: "change_email_address", old_email_address: ENV.fetch("OLD_EMAIL"), new_email_address: ENV.fetch("NEW_EMAIL"), expires_in: 30.minutes).to_s',
                                 TEST_USER_ID=user_id, OLD_EMAIL="owner@example.com",
                                 NEW_EMAIL=new_email)
            confirmation = f"{path}/{source_token}/confirmation"
            status, _, _ = request(target, confirmation, "POST", b"", accept="application/json")
            assert status == 204, status
            with sqlite3.connect(rust_dir / "rizzy.sqlite3") as database:
                actual = database.execute(
                    "SELECT i.email_address FROM users u JOIN identities i ON i.id=u.identity_id WHERE u.id=?",
                    (user_id,)).fetchone()[0]
            assert actual == new_email, actual
            status, _, _ = request(target, confirmation, "POST", b"", accept="application/json")
            assert status == 422, status
            print("PASS Rust email SGID parses in Fizzy; Fizzy email SGID confirms in Rust")
        finally:
            stop_server(process)


if __name__ == "__main__":
    main()
