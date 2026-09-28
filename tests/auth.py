#!/usr/bin/env python3
"""Exercise cookie-based magic-link authentication without sending email."""
import json
import os
import re
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def call(port, path, method="GET", payload=None, cookies=(), status=200):
    headers = {"Accept": "application/json", "Content-Type": "application/json"}
    if cookies:
        headers["Cookie"] = "; ".join(cookies)
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}",
        json.dumps(payload).encode() if payload is not None else None,
        headers=headers,
        method=method,
    )
    try:
        response = urllib.request.urlopen(request, timeout=3)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read()
        assert response.status == status, (path, response.status, raw)
        return (json.loads(raw) if raw and response.headers.get_content_type() == "application/json" else None), response.headers


def cookie(headers, name):
    for value in headers.get_all("Set-Cookie", []):
        if value.startswith(name + "="):
            return value.split(";", 1)[0]
    raise AssertionError(f"missing {name} cookie")


def start(env, binary=BINARY):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    server = subprocess.Popen(
        [binary], cwd=ROOT,
        env={**env, "RIZZY_BIND": f"127.0.0.1:{port}", "RIZZY_DEV_MAGIC_LINKS": env.get("RIZZY_DEV_MAGIC_LINKS", "1")},
        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
    )
    for _ in range(60):
        try:
            call(port, "/up")
            return port, server
        except OSError:
            time.sleep(.05)
    raise RuntimeError("server did not start: " + server.stderr.read().decode())


def stop(server):
    server.terminate()
    server.wait(timeout=5)


def request_code(port, email):
    body, headers = call(port, "/session", "POST", {"email_address": email}, status=201)
    pending = cookie(headers, "pending_authentication_token")
    assert "HttpOnly" in str(headers.get_all("Set-Cookie"))
    assert body["pending_authentication_token"] == pending.split("=", 1)[1]
    return pending, headers.get("X-Magic-Link-Code")


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{Path(temp) / 'existing.sqlite3'}"}
        seeded = []
        for email in ("owner@example.com", "other@example.com"):
            result = subprocess.run([BINARY, "seed"], cwd=ROOT, env={**env, "RIZZY_SEED_EMAIL": email},
                                    capture_output=True, text=True, check=True)
            seeded.append(re.search(r"Account: (\d+)", result.stdout).group(1))
        port, server = start(env)
        try:
            call(port, "/my/identity", status=406)
            pending, code = request_code(port, "owner@example.com")
            assert code and len(code) == 6
            call(port, "/session/magic_link", "POST", {"code": "WRONG0"}, [pending], 401)
            other_pending, other_code = request_code(port, "other@example.com")
            call(port, "/session/magic_link", "POST", {"code": other_code}, [pending], 401)
            call(port, "/session/magic_link", "POST", {"code": other_code}, [other_pending], 401)
            pending, code = request_code(port, "owner@example.com")
            response, headers = call(port, "/session/magic_link", "POST", {"code": code}, [pending])
            assert response["requires_signup_completion"] is False
            session = cookie(headers, "session_token")
            assert response["session_token"] == session.split("=", 1)[1]
            assert cookie(headers, "pending_authentication_token") == "pending_authentication_token="
            identity, _ = call(port, "/my/identity", cookies=[session])
            assert [a["slug"] for a in identity["accounts"]] == [f"/{seeded[0]}"]
            call(port, f"/{seeded[0]}/boards", cookies=[session])
            call(port, f"/{seeded[1]}/boards", cookies=[session], status=401)
            call(port, "/session/magic_link", "POST", {"code": code}, [pending], 401)
            _, headers = call(port, "/session", "DELETE", cookies=[session], status=204)
            assert cookie(headers, "session_token") == "session_token="
            call(port, "/my/identity", cookies=[session], status=406)
            fake, no_code = request_code(port, "unknown@example.com")
            assert fake and no_code is None
            for n in range(6):
                request_code(port, f"unknown-{n}@example.com")
            call(port, "/session", "POST", {"email_address": "one-too-many@example.com"}, status=429)
        finally:
            stop(server)

        no_mail_env = {**env, "RIZZY_DEV_MAGIC_LINKS": "0"}
        no_mail_env.pop("RIZZY_SENDMAIL", None)
        no_mail_env.pop("RIZZY_MAIL_FROM", None)
        no_mail_env.pop("RIZZY_SUPPORT_EMAIL", None)
        no_mail_env["RIZZY_DATABASE_URL"] = f"sqlite://{Path(temp) / 'no-mail.sqlite3'}"
        subprocess.run([BINARY, "seed"], cwd=ROOT, env=no_mail_env,
                       capture_output=True, text=True, check=True)
        port, server = start(no_mail_env)
        try:
            call(port, "/session", "POST", {"email_address": "owner@example.com"}, status=503)
        finally:
            stop(server)

        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{Path(temp) / 'fresh.sqlite3'}", "RIZZY_MULTI_TENANT": "1"}
        port, server = start(env)
        try:
            pending, code = request_code(port, "new@example.com")
            assert code
            response, headers = call(port, "/session/magic_link", "POST", {"code": code}, [pending])
            assert response["requires_signup_completion"] is True
            session = cookie(headers, "session_token")
            identity, _ = call(port, "/my/identity", cookies=[session])
            assert identity["accounts"] == []
            invalid, _ = call(port, "/signup/completion", "POST", {"signup": {"full_name": ""}}, [session], 422)
            assert invalid == {"errors": ["Full name can't be blank"]}
            call(port, "/signup/completion", "POST", {"signup": {"full_name": "New Owner"}}, [session], 201)
            identity, _ = call(port, "/my/identity", cookies=[session])
            assert len(identity["accounts"]) == 1
            assert identity["accounts"][0]["user"]["name"] == "New Owner"
            join_code, _ = call(port, identity["accounts"][0]["slug"] + "/account/join_code", cookies=[session])
            assert join_code["active"] and join_code["usage_count"] == 0
            call(port, "/signup/completion", "POST", {"signup": {"full_name": "Again"}}, [session], 201)
            identity, _ = call(port, "/my/identity", cookies=[session])
            assert len(identity["accounts"]) == 2
            call(port, "/session", "DELETE", cookies=[session], status=204)
            _, headers = call(port, "/signup", "POST", {"signup": {"email_address": "new@example.com"}}, status=201)
            second_pending = cookie(headers, "pending_authentication_token")
            second_code = headers.get("X-Magic-Link-Code")
            response, headers = call(port, "/session/magic_link", "POST", {"code": second_code}, [second_pending])
            assert response["requires_signup_completion"] is True
            second_session = cookie(headers, "session_token")
            call(port, "/signup/completion", "POST", {"signup": {"full_name": "New Owner"}}, [second_session], 201)
            identity, _ = call(port, "/my/identity", cookies=[second_session])
            assert len(identity["accounts"]) == 3
        finally:
            stop(server)
    print("Auth passed: sign-in, code consumption, isolation, sign-out, signup")


if __name__ == "__main__":
    main()
