#!/usr/bin/env python3
"""Exercise account invite links, joining, limits, and permissions."""
import json
import concurrent.futures
import os
import re
import sqlite3
import subprocess
import tempfile
import threading
import urllib.error
import urllib.request
from pathlib import Path

from auth import BINARY, ROOT, cookie, start, stop


def call(port, path, method="GET", body=None, bearer=None, cookies=(), status=200, accept="application/json"):
    headers = {"Accept": accept, "Content-Type": "application/json"}
    if bearer:
        headers["Authorization"] = "Bearer " + bearer
    if cookies:
        headers["Cookie"] = "; ".join(cookies)
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}",
        json.dumps(body).encode() if body is not None else None,
        method=method, headers=headers,
    )
    try:
        response = urllib.request.urlopen(request, timeout=3)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read()
        assert response.status == status, (path, response.status, raw)
        value = json.loads(raw) if raw and response.headers.get_content_type() == "application/json" else raw.decode()
        return value, response.headers


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{Path(temp) / 'join.sqlite3'}"}
        result = subprocess.run([BINARY, "seed"], cwd=ROOT, env={**env, "RIZZY_SEED_EMAIL": "join-owner@example.com"},
                                capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", result.stdout).group(1)
        owner_token = re.search(r"Bearer token: (\w+)", result.stdout).group(1)
        port, server = start(env)
        try:
            path = f"/{account}/account/join_code"
            public, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Everyone"}}, bearer=owner_token, status=201)
            private, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Private", "all_access": False}}, bearer=owner_token, status=201)
            owner_id = public["creator"]["id"]
            call(port, path, status=406)
            invite, _ = call(port, path, bearer=owner_token)
            assert invite["usage_count"] == 0 and invite["usage_limit"] == 10 and invite["active"]
            assert re.fullmatch(r"[1-9A-HJ-NP-Za-km-z]{4}(?:-[1-9A-HJ-NP-Za-km-z]{4}){2}", invite["code"])
            invite_path = f"/{account}/join/{invite['code']}"
            html, _ = call(port, invite_path)
            assert "Join Rizzy in Rizzy" in html
            call(port, f"/9999/join/{invite['code']}", status=404)
            call(port, invite_path, "POST", {"email_address": "invalid"}, status=422)
            pending_body, headers = call(port, invite_path, "POST", {"email_address": "new-member@example.com"}, status=201)
            assert pending_body["pending_authentication_token"]
            pending = cookie(headers, "pending_authentication_token")
            code = headers.get("X-Magic-Link-Code")
            assert code
            invite, _ = call(port, path, bearer=owner_token)
            assert invite["usage_count"] == 1
            signed_in, headers = call(port, "/session/magic_link", "POST", {"code": code}, cookies=[pending])
            assert signed_in["requires_signup_completion"] is False
            session = cookie(headers, "session_token")
            identity, _ = call(port, "/my/identity", cookies=[session])
            member = next(a["user"] for a in identity["accounts"] if a["slug"] == f"/{account}")
            assert member["role"] == "member" and member["name"] == "new-member@example.com"
            call(port, f"/{account}/boards/{public['id']}", cookies=[session])
            call(port, f"/{account}/boards/{private['id']}", cookies=[session], status=404)
            public_accesses, _ = call(port, f"/{account}/boards/{public['id']}/accesses", cookies=[session])
            public_member = next(u for u in public_accesses["users"] if u["id"] == member["id"])
            assert public_member["has_access"] and public_member["involvement"] == "access_only"
            private_accesses, _ = call(port, f"/{account}/boards/{private['id']}/accesses", bearer=owner_token)
            private_member = next(u for u in private_accesses["users"] if u["id"] == member["id"])
            assert not private_member["has_access"] and private_member["involvement"] is None
            assert next(u for u in private_accesses["users"] if u["id"] == owner_id)["involvement"] == "watching"
            call(port, f"/{account}/boards/{public['id']}/involvement", "PUT", {"involvement": "watching"}, cookies=[session], status=204)
            public_accesses, _ = call(port, f"/{account}/boards/{public['id']}/accesses", bearer=owner_token)
            assert next(u for u in public_accesses["users"] if u["id"] == member["id"])["involvement"] == "watching"
            call(port, f"/{account}/boards/{private['id']}", "PUT", {"user_ids": [owner_id, member["id"]]}, bearer=owner_token)
            call(port, f"/{account}/boards/{private['id']}", cookies=[session])
            call(port, f"/{account}/boards/{private['id']}/involvement", "PUT", {"involvement": "watching"}, cookies=[session], status=204)
            call(port, f"/{account}/boards/{private['id']}", "PUT", {"user_ids": [owner_id]}, bearer=owner_token)
            call(port, f"/{account}/boards/{private['id']}", cookies=[session], status=404)
            call(port, f"/{account}/users/joins", "POST", {"user": {"name": "New Member"}}, cookies=[session], status=204)
            member, _ = call(port, f"/{account}/users/{member['id']}", cookies=[session])
            assert member["name"] == "New Member"
            call(port, path, "PUT", {"account_join_code": {"usage_limit": 2}}, cookies=[session], status=403)
            call(port, path, "DELETE", cookies=[session], status=403)
            call(port, path, cookies=[session])
            call(port, path, "PUT", {"account_join_code": {"usage_limit": "1"}}, bearer=owner_token, status=204)
            call(port, invite_path, status=410)
            call(port, invite_path, "POST", {"email_address": "another@example.com"}, status=410)
            call(port, path, "PUT", {"account_join_code": {"usage_limit": "999999999999999999999999999"}}, bearer=owner_token, status=422)
            call(port, path, "DELETE", bearer=owner_token, status=204)
            refreshed, _ = call(port, path, bearer=owner_token)
            assert refreshed["code"] != invite["code"] and refreshed["usage_count"] == 0
            assert refreshed["usage_limit"] == 1 and refreshed["active"]
            call(port, invite_path, status=404)
            second_path = f"/{account}/join/{refreshed['code']}"
            joined, _ = call(port, second_path, "POST", {"email_address": "new-member@example.com"}, cookies=[session])
            assert joined == {"joined": True, "requires_name": False, "account_slug": f"/{account}"}
            refreshed, _ = call(port, path, bearer=owner_token)
            assert refreshed["usage_count"] == 0
            other_port, other_server = start(env)
            try:
                barrier = threading.Barrier(2)

                def race(port_number, email_address):
                    barrier.wait()
                    req = urllib.request.Request(
                        f"http://127.0.0.1:{port_number}{second_path}",
                        json.dumps({"email_address": email_address}).encode(), method="POST",
                        headers={"Content-Type": "application/json", "Accept": "application/json"},
                    )
                    try:
                        with urllib.request.urlopen(req, timeout=5) as response:
                            return response.status
                    except urllib.error.HTTPError as error:
                        return error.code

                with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                    results = [
                        pool.submit(race, port, "racer-one@example.com"),
                        pool.submit(race, other_port, "racer-two@example.com"),
                    ]
                    statuses = sorted(future.result() for future in results)
                assert statuses == [201, 410], statuses
                refreshed, _ = call(port, path, bearer=owner_token)
                assert refreshed["usage_count"] == 1
            finally:
                stop(other_server)
            print("Join passed: redemption, setup, limits, two-process race, reset, permissions")
        finally:
            stop(server)
        with sqlite3.connect(Path(temp) / "join.sqlite3") as db:
            db.execute("DELETE FROM account_join_codes")
        port, server = start(env)
        try:
            restored, _ = call(port, f"/{account}/account/join_code", bearer=owner_token)
            assert restored["code"] and restored["usage_count"] == 0
        finally:
            stop(server)


if __name__ == "__main__":
    main()
