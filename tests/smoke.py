#!/usr/bin/env python3
"""End-to-end API and tenant isolation smoke test using a temporary database."""
import hashlib
import json
import os
import re
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def call(port, account, token, path, method="GET", payload=None, status=200):
    body = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/{account}{path}", body, method=method,
        headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json", "Accept": "application/json"},
    )
    try:
        response = urllib.request.urlopen(req, timeout=3)
    except urllib.error.HTTPError as e:
        response = e
    with response:
        assert response.status == status, (path, response.status, response.read())
        raw = response.read()
        return json.loads(raw) if raw else None


def seed(env, email):
    result = subprocess.run([BINARY, "seed"], cwd=ROOT, env={**env, "RIZZY_SEED_EMAIL": email}, capture_output=True, text=True, check=True)
    account = re.search(r"Account: (\d+)", result.stdout).group(1)
    token = re.search(r"Bearer token: (\w+)", result.stdout).group(1)
    return account, token


def main():
    if not BINARY.exists():
        subprocess.run(["cargo", "build"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "test.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        owner, owner_token = seed(env, "owner@example.com")
        outsider, outsider_token = seed(env, "outsider@example.com")
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        server = subprocess.Popen([BINARY], cwd=ROOT, env={**env, "RIZZY_BIND": f"127.0.0.1:{port}"}, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            for _ in range(50):
                try:
                    urllib.request.urlopen(f"http://127.0.0.1:{port}/up", timeout=.2)
                    break
                except OSError:
                    time.sleep(.05)
            else:
                raise RuntimeError("server did not start: " + server.stderr.read().decode())
            call(port, owner, "wrong", "/boards", status=401)
            board = call(port, owner, owner_token, "/boards", "POST", {"board": {"name": "Private", "all_access": False}}, 201)
            board_id = board["id"]
            assert board["user_ids"] == [board["creator"]["id"]]
            assert call(port, outsider, outsider_token, f"/boards/{board_id}", status=404)
            col = call(port, owner, owner_token, f"/boards/{board_id}/columns", "POST", {"column": {"name": "Doing"}}, 201)
            card = call(port, owner, owner_token, f"/boards/{board_id}/cards", "POST", {"card": {"title": "First", "description": "Safe <text>"}}, 201)
            number = card["number"]
            assert "&lt;text&gt;" in card["description_html"]
            assert call(port, outsider, outsider_token, f"/cards/{number}", status=404)
            call(port, owner, owner_token, f"/cards/{number}/triage", "POST", {"column_id": col["id"]}, 204)
            assert call(port, owner, owner_token, f"/cards/{number}")["column"]["name"] == "Doing"
            step = call(port, owner, owner_token, f"/cards/{number}/steps", "POST", {"step": {"content": "Test"}}, 201)
            call(port, owner, owner_token, f"/cards/{number}/steps/{step['id']}", "PUT", {"step": {"completed": True}})
            assert call(port, owner, owner_token, f"/cards/{number}")["steps"][0]["completed"] is True
            call(port, owner, owner_token, f"/cards/{number}/taggings", "POST", {"tag_title": "#urgent"}, 204)
            assert call(port, owner, owner_token, f"/cards/{number}")["tags"] == ["urgent"]
            assert [tag["title"] for tag in call(port, owner, owner_token, "/tags")] == ["urgent"]
            assert call(port, outsider, outsider_token, "/tags") == []
            assert [item["id"] for item in call(port, owner, owner_token, "/search?q=First")] == [card["id"]]
            assert call(port, outsider, outsider_token, "/search?q=First") == []
            call(port, owner, owner_token, f"/cards/{number}/comments", "POST", {"comment": {"body": "Looks good"}}, 201)
            comments = call(port, owner, owner_token, f"/cards/{number}/comments")
            assert len(comments) == 2 and any(item["body"]["plain_text"] == "Looks good" for item in comments)
            published = call(port, owner, owner_token, f"/boards/{board_id}/publication", "POST", status=201)
            public_url = published["public_url"]
            with urllib.request.urlopen(public_url, timeout=3) as response:
                assert "Private" in response.read().decode()
            with urllib.request.urlopen(f"{public_url}/cards/{number}", timeout=3) as response:
                assert "Safe &lt;text&gt;" in response.read().decode()
            try:
                urllib.request.urlopen(public_url.replace(f"/{owner}/public/", f"/{outsider}/public/"), timeout=3)
                raise AssertionError("cross-account public key accepted")
            except urllib.error.HTTPError as error:
                assert error.code == 404
            call(port, owner, owner_token, f"/boards/{board_id}/publication", "DELETE", status=204)
            try:
                urllib.request.urlopen(public_url, timeout=3)
                raise AssertionError("unpublished key accepted")
            except urllib.error.HTTPError as error:
                assert error.code == 404
            call(port, owner, owner_token, f"/cards/{number}/closure", "POST", status=204)
            assert call(port, owner, owner_token, f"/cards/{number}")["closed"] is True
            with sqlite3.connect(database) as db:
                identity = db.execute("SELECT identity_id FROM users WHERE id=?", (board["creator"]["id"],)).fetchone()[0]
                db.execute("INSERT INTO access_tokens VALUES(?,?,?,?,?,?)", ("read-only", identity, hashlib.sha256(b"read-only-token").hexdigest(), "read", "read", "2026-01-01T00:00:00Z"))
            assert call(port, owner, "read-only-token", "/boards")
            call(port, owner, "read-only-token", "/boards", "POST", {"board": {"name": "Forbidden"}}, 403)
            print("Smoke passed: CRUD, card states, content escaping, token permissions, tenant isolation")
        finally:
            server.terminate()
            server.wait(timeout=5)


if __name__ == "__main__":
    main()
