#!/usr/bin/env python3
"""Exercise asynchronous user exports with an isolated Rizzy account and archive."""
import hashlib
import io
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
import zipfile
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def call(url, token, path, method="GET", payload=None, expected=200, raw=False):
    request = urllib.request.Request(
        url + path,
        method=method,
        data=json.dumps(payload).encode() if payload is not None else None,
        headers={"Authorization": f"Bearer {token}", "Accept": "application/json", "Content-Type": "application/json"},
    )
    try:
        response = urllib.request.urlopen(request, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        body = response.read()
        assert response.status == expected, (path, response.status, body[:300])
        return body if raw else json.loads(body) if body else None


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        db_path = Path(temp) / "test.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{db_path}", "RIZZY_EXPORT_DIR": str(Path(temp) / "archives")}
        result = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, text=True, capture_output=True, check=True)
        account = re.search(r"Account: (\d+)", result.stdout).group(1)
        token = re.search(r"Bearer token: (\w+)", result.stdout).group(1)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        server = subprocess.Popen([BINARY], cwd=ROOT, env={**env, "RIZZY_BIND": f"127.0.0.1:{port}"}, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            for _ in range(100):
                try:
                    urllib.request.urlopen(f"http://127.0.0.1:{port}/up", timeout=.2)
                    break
                except OSError:
                    time.sleep(.05)
            else:
                raise RuntimeError("Rizzy did not start")
            url = f"http://127.0.0.1:{port}/{account}"
            owner = next(u for u in call(url, token, "/users") if u["role"] == "owner")
            board = call(url, token, "/boards", "POST", {"board": {"name": "Export board", "all_access": False}}, 201)
            column = call(url, token, f"/boards/{board['id']}/columns", "POST", {"column": {"name": "Doing"}}, 201)
            card = call(url, token, f"/boards/{board['id']}/cards", "POST", {"card": {"title": "Export card", "description": "A < B"}}, 201)
            call(url, token, f"/cards/{card['number']}/triage", "POST", {"column_id": column["id"]}, 204)
            call(url, token, f"/cards/{card['number']}/comments", "POST", {"comment": {"body": "One & two"}}, 201)

            # Another member's private board must be excluded, and their export route is forbidden.
            other_token = "other-export-token"
            with sqlite3.connect(db_path) as db:
                account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?", (account,)).fetchone()[0]
                db.execute("INSERT INTO identities(id,email_address,created_at) VALUES('other-export-identity','other@example.com',?)", ("2026-09-25T00:00:00Z",))
                db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('other-export-user',?,'other-export-identity','Other','member',?)", (account_id, "2026-09-25T00:00:00Z"))
                db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES('other-export-token-row','other-export-identity',?,'test','write',?)", (hashlib.sha256(other_token.encode()).hexdigest(), "2026-09-25T00:00:00Z"))
            private = call(url, other_token, "/boards", "POST", {"board": {"name": "Other private", "all_access": False}}, 201)
            other_card = call(url, other_token, f"/boards/{private['id']}/cards", "POST", {"card": {"title": "Exclude this"}}, 201)
            call(url, token, "/users/other-export-user/data_exports", "POST", expected=403)

            exported = call(url, token, f"/users/{owner['id']}/data_exports", "POST", expected=201)
            assert exported["status"] == "pending" and "download_url" not in exported
            for _ in range(100):
                result = call(url, token, f"/users/{owner['id']}/data_exports/{exported['id']}")
                if result["status"] in ("completed", "failed"):
                    break
                time.sleep(.1)
            assert result["status"] == "completed", result
            assert result["download_url"].startswith(url)
            archive_bytes = call(url, token, f"/exports/{exported['id']}/download", raw=True)
            with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
                assert archive.namelist() == [f"{card['number']}.json"], archive.namelist()
                value = json.loads(archive.read(f"{card['number']}.json"))
            assert value["title"] == "Export card" and value["board"] == "Export board"
            assert value["status"] == "Doing" and value["description"] == "A &lt; B"
            assert value["creator"]["id"] == owner["id"]
            assert "Exclude this" not in json.dumps(value) and value["number"] != other_card["number"]
            assert [c["body"] for c in value["comments"]] == [
                "Owner <strong>moved</strong> this to “Doing”", "One &amp; two"
            ]
            call(url, other_token, f"/exports/{exported['id']}/download", expected=404)
            call(url, other_token, f"/users/{owner['id']}/data_exports/{exported['id']}", expected=403)
            with sqlite3.connect(db_path) as db:
                account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?", (account,)).fetchone()[0]
                recent = datetime.now(timezone.utc).isoformat()
                for index in range(9):
                    db.execute("INSERT INTO exports(id,account_id,user_id,kind,created_at,updated_at) VALUES(?,?,?,'user',?,?)", (f"limit-{index}", account_id, owner["id"], recent, recent))
            call(url, token, f"/users/{owner['id']}/data_exports", "POST", expected=429)
            print("User exports passed: archive content, access scope, download, and 24-hour limit")
        finally:
            server.terminate()
            server.wait(timeout=5)


if __name__ == "__main__":
    main()
