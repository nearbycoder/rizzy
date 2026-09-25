#!/usr/bin/env python3
"""Check saved filter persistence, deduplication, ownership, and Turbo responses."""
import hashlib
import json
import os
import re
import sqlite3
import subprocess
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from join import call


def turbo(port, path, token, method="POST", params=None, status=200, bearer=None):
    body = urllib.parse.urlencode(params or {}, doseq=True).encode() if method == "POST" else None
    headers = {"Accept": "text/vnd.turbo-stream.html",
               "Content-Type": "application/x-www-form-urlencoded"}
    if token:
        headers["Cookie"] = "session_token=" + token
    if bearer:
        headers["Authorization"] = "Bearer " + bearer
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}", body, method=method,
        headers=headers,
    )
    try:
        response = urllib.request.urlopen(request, timeout=3)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        content = response.read().decode()
        assert response.status == status, (path, response.status, content)
        if status == 200:
            assert response.headers.get_content_type() == "text/vnd.turbo-stream.html"
            assert '<turbo-stream action="replace" target="filter-settings-save-toggle">' in content
        return content


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "filters.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        seed = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seed.stdout).group(1)
        owner_token = re.search(r"Bearer token: (\w+)", seed.stdout).group(1)
        with sqlite3.connect(database) as db:
            account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?", (account,)).fetchone()[0]
            db.execute("INSERT INTO identities(id,email_address,created_at) VALUES('filter-identity','filter@example.com','2026-01-01T00:00:00Z')")
            db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('filter-user',?,'filter-identity','Filter Member','member','2026-01-01T00:00:00Z')", (account_id,))
            db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES('filter-token','filter-identity',?,'Test','write','2026-01-01T00:00:00Z')", (hashlib.sha256(b'filter-token').hexdigest(),))
            owner_identity = db.execute("SELECT identity_id FROM users WHERE role='owner'").fetchone()[0]
            db.execute("INSERT INTO sessions(id,identity_id,token_hash,signup_pending,created_at) VALUES('filter-owner-session',?,?,0,'2026-01-01T00:00:00Z')", (owner_identity, hashlib.sha256(b'filter-owner-cookie').hexdigest()))
            db.execute("INSERT INTO sessions(id,identity_id,token_hash,signup_pending,created_at) VALUES('filter-member-session','filter-identity',?,0,'2026-01-01T00:00:00Z')", (hashlib.sha256(b'filter-member-cookie').hexdigest(),))
        port, server = start(env)
        try:
            board, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Saved views"}}, bearer=owner_token, status=201)
            card, _ = call(port, f"/{account}/boards/{board['id']}/cards", "POST", {"card": {"title": "Tagged"}}, bearer=owner_token, status=201)
            call(port, f"/{account}/cards/{card['number']}/taggings", "POST", {"tag_title": "mobile"}, bearer=owner_token, status=204)
            with sqlite3.connect(database) as db:
                tag_id = db.execute("SELECT id FROM tags WHERE title='mobile'").fetchone()[0]
                owner_id = db.execute("SELECT id FROM users WHERE role='owner'").fetchone()[0]
            params = {"indexed_by": "closed", "assignment_status": "unassigned", "tag_ids[]": [tag_id],
                      "assignee_ids[]": ["filter-user"], "board_ids[]": [board["id"]]}
            path = f"/{account}/filters"
            turbo(port, path, None, status=406)
            turbo(port, path, None, bearer=owner_token, status=401)
            html = turbo(port, path, "filter-owner-cookie", params=params)
            assert "Delete custom view" in html
            with sqlite3.connect(database) as db:
                filter_id, fields = db.execute("SELECT id,fields FROM filters WHERE creator_id=?", (owner_id,)).fetchone()
                values = json.loads(fields)
                assert values == {"indexed_by": "closed", "assignment_status": "unassigned",
                                  "tag_ids": [tag_id], "assignee_ids": ["filter-user"], "board_ids": [board["id"]]}
                for table in ("filters_tags", "assignees_filters", "boards_filters"):
                    assert db.execute(f"SELECT COUNT(*) FROM {table} WHERE filter_id=?", (filter_id,)).fetchone()[0] == 1
            turbo(port, path, "filter-owner-cookie", params={**params, "sorted_by": "latest"})
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM filters WHERE creator_id=?", (owner_id,)).fetchone()[0] == 1
            turbo(port, path, "filter-member-cookie", params=params)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM filters").fetchone()[0] == 2
            private, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Private", "all_access": False}}, bearer=owner_token, status=201)
            turbo(port, path, "filter-member-cookie", params={"board_ids[]": [private["id"]]}, status=404)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM filters").fetchone()[0] == 2
            turbo(port, path + "/" + filter_id, "filter-member-cookie", method="DELETE", status=404)
            html = turbo(port, path + "/" + filter_id, "filter-owner-cookie", method="DELETE")
            assert "Save custom view" in html
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM filters WHERE id=?", (filter_id,)).fetchone()[0] == 0
                assert db.execute("SELECT COUNT(*) FROM filters_tags WHERE filter_id=?", (filter_id,)).fetchone()[0] == 0
        finally:
            stop(server)
    print("saved filter checks passed")


if __name__ == "__main__":
    main()
