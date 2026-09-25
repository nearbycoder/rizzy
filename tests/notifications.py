#!/usr/bin/env python3
"""Exercise notification recipients, bundling, read state, and board access."""
import hashlib
import os
import re
import sqlite3
import subprocess
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

from auth import BINARY, ROOT, call as session_call, cookie, request_code, start, stop
from join import call


def browser_request(port, path, method, session, accept):
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}", method=method,
        headers={"Accept": accept, "Cookie": session},
    )
    try:
        response = urllib.request.urlopen(request, timeout=3)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers.get_content_type(), response.read()


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "notifications.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
        owner_token = re.search(r"Bearer token: (\w+)", seeded.stdout).group(1)
        port, server = start(env)
        try:
            empty, _ = call(port, f"/{account}/notifications", bearer=owner_token)
            assert empty == []
            settings, _ = call(port, f"/{account}/notifications/settings", bearer=owner_token)
            assert settings == {"bundle_email_frequency": "every_few_hours"}
            call(port, f"/{account}/notifications/settings", "PUT", {"user_settings": {"bundle_email_frequency": "every_few_hours"}}, bearer=owner_token, status=204)
            settings, _ = call(port, f"/{account}/notifications/settings", bearer=owner_token)
            assert settings == {"bundle_email_frequency": "every_few_hours"}
            call(port, f"/{account}/notifications/settings", "PUT", {"user_settings": {"bundle_email_frequency": "never"}}, bearer=owner_token, status=204)
            board, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Alerts", "all_access": False}}, bearer=owner_token, status=201)
            board_id, owner_id = board["id"], board["creator"]["id"]
            with sqlite3.connect(database) as db:
                account_id = db.execute("SELECT account_id FROM boards WHERE id=?", (board_id,)).fetchone()[0]
                db.execute("INSERT INTO identities(id,email_address,created_at) VALUES('notice-identity','notice@example.com','2026-01-01T00:00:00Z')")
                db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('notice-member',?,'notice-identity','Member','member','2026-01-01T00:00:00Z')", (account_id,))
                db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES('notice-token','notice-identity',?,'Test','write','2026-01-01T00:00:00Z')", (hashlib.sha256(b'notice-token').hexdigest(),))
            member_token = "notice-token"
            call(port, f"/{account}/boards/{board_id}", "PUT", {"user_ids": [owner_id, "notice-member"]}, bearer=owner_token)
            call(port, f"/{account}/boards/{board_id}/involvement", "PUT", {"involvement": "watching"}, bearer=member_token, status=204)
            card, _ = call(port, f"/{account}/boards/{board_id}/cards", "POST", {"card": {"title": "First alert"}}, bearer=owner_token, status=201)
            number = card["number"]
            notices, _ = call(port, f"/{account}/notifications", bearer=member_token)
            assert len(notices) == 1 and notices[0]["title"] == "First alert" and notices[0]["body"] == "Added by Owner", notices
            notice = notices[0]
            assert notice["creator"]["id"] == owner_id and notice["card"]["number"] == number
            assert notice["card"]["column"] is None and notice["unread_count"] == 1 and not notice["read"]
            pending, code = request_code(port, "notice@example.com")
            _, session_headers = session_call(port, "/session/magic_link", "POST", {"code": code}, [pending])
            member_session = cookie(session_headers, "session_token")
            card_reading = f"/{account}/cards/{number}/reading"
            status, content_type, body = browser_request(
                port, card_reading, "POST", member_session, "text/vnd.turbo-stream.html")
            assert status == 200 and content_type == "text/vnd.turbo-stream.html"
            assert f'action="remove" target="notification_{notice["id"]}"'.encode() in body, body
            notices, _ = call(port, f"/{account}/notifications", bearer=member_token)
            assert notices[0]["read"] and notices[0]["unread_count"] == 0
            status, content_type, body = browser_request(
                port, card_reading, "DELETE", member_session, "text/vnd.turbo-stream.html")
            assert status == 204 and content_type == "text/plain" and not body
            notices, _ = call(port, f"/{account}/notifications", bearer=member_token)
            assert not notices[0]["read"] and notices[0]["unread_count"] == 1
            owner_notices, _ = call(port, f"/{account}/notifications", bearer=owner_token)
            assert owner_notices == []
            with sqlite3.connect(database) as db:
                owner_email = db.execute(
                    "SELECT i.email_address FROM users u JOIN identities i ON i.id=u.identity_id WHERE u.id=?",
                    (owner_id,),
                ).fetchone()[0]
            owner_pending, owner_code = request_code(port, owner_email)
            _, owner_session_headers = session_call(
                port, "/session/magic_link", "POST", {"code": owner_code}, [owner_pending])
            owner_session = cookie(owner_session_headers, "session_token")
            status, content_type, body = browser_request(
                port, card_reading, "POST", owner_session, "text/vnd.turbo-stream.html")
            assert status == 200 and content_type == "text/vnd.turbo-stream.html" and body == b"\n"
            call(port, f"/{account}/cards/{number}/closure", "POST", bearer=owner_token, status=204)
            notices, _ = call(port, f"/{account}/notifications", bearer=member_token)
            assert len(notices) == 1 and notices[0]["unread_count"] == 2 and notices[0]["body"] == "Moved to Done by Owner", notices
            call(port, f"/{account}/boards/{board_id}/involvement", "PUT", {"involvement": "access_only"}, bearer=member_token, status=204)
            call(port, f"/{account}/cards/{number}/assignments", "POST", {"assignee_id": "notice-member"}, bearer=owner_token, status=204)
            notices, _ = call(port, f"/{account}/notifications", bearer=member_token)
            assert len(notices) == 1 and notices[0]["unread_count"] == 3 and notices[0]["body"] == "Assigned to Member", notices
            call(port, card_reading, "POST", bearer=member_token, status=201)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT accessed_at FROM accesses WHERE board_id=? AND user_id='notice-member'", (board_id,)).fetchone()[0]
            second_board, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Second", "all_access": False}}, bearer=owner_token, status=201)
            call(port, f"/{account}/boards/{second_board['id']}", "PUT", {"user_ids": [owner_id, "notice-member"]}, bearer=owner_token)
            boards, _ = call(port, f"/{account}/boards", bearer=member_token)
            assert boards[0]["id"] == board_id
            notices, _ = call(port, f"/{account}/notifications", bearer=member_token)
            assert notices[0]["read"] and notices[0]["unread_count"] == 0
            call(port, card_reading, "DELETE", bearer=member_token, status=204)
            notices, _ = call(port, f"/{account}/notifications", bearer=member_token)
            assert not notices[0]["read"] and notices[0]["unread_count"] == 1
            path = f"/{account}/notifications/{notice['id']}/reading"
            call(port, path, "POST", bearer=owner_token, status=404)
            call(port, path, "POST", bearer=member_token, status=204)
            tray, _ = call(port, f"/{account}/notifications/tray", bearer=member_token)
            assert tray == []
            tray, _ = call(port, f"/{account}/notifications/tray?include_read=true", bearer=member_token)
            assert len(tray) == 1 and tray[0]["read"] and tray[0]["unread_count"] == 0
            call(port, path, "DELETE", bearer=member_token, status=204)
            tray, _ = call(port, f"/{account}/notifications/tray", bearer=member_token)
            assert len(tray) == 1 and not tray[0]["read"]
            call(port, f"/{account}/notifications/bulk_reading", "POST", bearer=member_token, status=204)
            call(port, f"/{account}/boards/{board_id}", "PUT", {"user_ids": [owner_id]}, bearer=owner_token)
            notices, _ = call(port, f"/{account}/notifications", bearer=member_token)
            assert notices == []
            print("Notifications passed: recipients, per-card bundling, read state, tray, and access revocation")
        finally:
            stop(server)


if __name__ == "__main__":
    main()
