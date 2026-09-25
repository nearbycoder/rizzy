#!/usr/bin/env python3
"""Exercise notification recipients, bundling, read state, and board access."""
import hashlib
import os
import re
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

from auth import BINARY, ROOT, call as session_call, cookie, request_code, start, stop
from join import call


def browser_response(port, path, method, session, accept, extra_headers=None):
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}", method=method,
        headers={"Accept": accept, "Cookie": session, **(extra_headers or {})},
    )
    try:
        response = urllib.request.urlopen(request, timeout=3)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


def browser_request(port, path, method, session, accept):
    status, headers, body = browser_response(port, path, method, session, accept)
    return status, headers.get_content_type(), body


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
            status, content_type, body = browser_request(
                port, f"/{account}/notifications", "GET", member_session, "text/html")
            assert status == 200 and content_type == "text/html"
            assert all(marker in body for marker in (
                b"New for you", b"notifications_list", b"First alert", b"Added by Owner",
                f'id="notification_{notice["id"]}"'.encode(), b"Mark all as read", b"Notification settings"))
            assert b'id="read_section"' not in body.split(b"</main>", 1)[0]
            call(port, f"/{account}/boards/{board_id}", "PUT", {"board": {"name": "Alerts renamed"}}, bearer=owner_token)
            time.sleep(1.1)
            status, _, body = browser_request(
                port, f"/{account}/notifications", "GET", member_session, "text/html")
            assert status == 200 and b"Alerts renamed" in body
            status, content_type, body = browser_request(
                port, f"/{account}/notifications/tray", "GET", member_session, "text/html")
            assert status == 200 and content_type == "text/html"
            assert b'<turbo-frame id="notifications">' in body and f'notification_{notice["id"]}'.encode() in body
            tray_path = f"/{account}/notifications/tray"
            status, tray_headers, _ = browser_response(
                port, tray_path, "GET", member_session, "text/html")
            unread_etag = tray_headers["ETag"]
            assert status == 200 and tray_headers["Cache-Control"] == "max-age=0, private, must-revalidate"
            status, cached_headers, cached_body = browser_response(
                port, tray_path, "GET", member_session, "text/html", {"If-None-Match": unread_etag})
            assert status == 304 and cached_headers["ETag"] == unread_etag and not cached_body
            false_status, false_headers, _ = browser_response(
                port, tray_path + "?include_read=0", "GET", member_session, "text/html")
            true_status, true_headers, _ = browser_response(
                port, tray_path + "?include_read=1", "GET", member_session, "text/html")
            assert false_status == true_status == 200
            assert len({unread_etag, false_headers["ETag"], true_headers["ETag"]}) == 3
            status, json_headers, json_body = browser_response(
                port, tray_path + ".json?include_read=true", "GET", member_session, "application/json")
            assert status == 200 and json_headers["ETag"] != true_headers["ETag"]
            assert b"First alert" in json_body
            status, _, cached_json = browser_response(
                port, tray_path + ".json?include_read=1", "GET", member_session,
                "application/json", {"If-None-Match": json_headers["ETag"]})
            assert status == 304 and not cached_json
            status, content_type, body = browser_request(
                port, f"/{account}/notifications?page=1", "GET", member_session,
                "text/vnd.turbo-stream.html")
            assert status == 200 and content_type == "text/vnd.turbo-stream.html"
            assert b'append" target="notifications_list_read' in body and b'replace" target="next_page' in body
            card_reading = f"/{account}/cards/{number}/reading"
            other_port, other_server = start(env)
            try:
                for _ in range(2):
                    status, _, body = browser_request(
                        other_port, f"/{account}/notifications", "GET", member_session, "text/html")
                    assert status == 200 and b"New for you" in body
                call(port, card_reading, "POST", bearer=member_token, status=201)
                status, changed_headers, changed_body = browser_response(
                    other_port, tray_path, "GET", member_session, "text/html", {"If-None-Match": unread_etag})
                assert status == 200 and changed_headers["ETag"] != unread_etag
                assert f'notification_{notice["id"]}'.encode() not in changed_body
                status, _, body = browser_request(
                    other_port, f"/{account}/notifications", "GET", member_session, "text/html")
                assert status == 200 and b"Previously seen" in body.split(b"</main>", 1)[0]
                call(port, card_reading, "DELETE", bearer=member_token, status=204)
                status, _, body = browser_request(
                    other_port, f"/{account}/notifications", "GET", member_session, "text/html")
                assert status == 200 and b"New for you" in body.split(b"</main>", 1)[0]
            finally:
                stop(other_server)
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
            status, content_type, body = browser_request(
                port, path, "POST", member_session, "text/vnd.turbo-stream.html")
            assert status == 200 and content_type == "text/vnd.turbo-stream.html"
            assert f'action="remove" target="notification_{notice["id"]}"'.encode() in body
            assert b'action="prepend" target="notifications_list_read"' in body and b"Previously seen" not in body
            status, _, body = browser_request(
                port, f"/{account}/notifications", "GET", member_session, "text/html")
            assert b"Previously seen" in body and f'notification_{notice["id"]}'.encode() in body
            status, content_type, body = browser_request(
                port, path, "DELETE", member_session, "text/vnd.turbo-stream.html")
            assert status == 200 and content_type == "text/vnd.turbo-stream.html"
            assert b'action="prepend" target="notifications_list"' in body
            call(port, path, "POST", bearer=member_token, status=204)
            tray, _ = call(port, f"/{account}/notifications/tray", bearer=member_token)
            assert tray == []
            tray, _ = call(port, f"/{account}/notifications/tray?include_read=true", bearer=member_token)
            assert len(tray) == 1 and tray[0]["read"] and tray[0]["unread_count"] == 0
            call(port, path, "DELETE", bearer=member_token, status=204)
            tray, _ = call(port, f"/{account}/notifications/tray", bearer=member_token)
            assert len(tray) == 1 and not tray[0]["read"]
            call(port, f"/{account}/notifications/bulk_reading", "POST", bearer=member_token, status=204)
            call(port, path, "DELETE", bearer=member_token, status=204)
            status, _, body = browser_request(
                port, f"/{account}/notifications/bulk_reading?from_tray=true",
                "POST", member_session, "text/html")
            assert status == 200 and not body
            tray, _ = call(port, f"/{account}/notifications/tray", bearer=member_token)
            assert tray == []
            call(port, f"/{account}/boards/{board_id}", "PUT", {"user_ids": [owner_id]}, bearer=owner_token)
            notices, _ = call(port, f"/{account}/notifications", bearer=member_token)
            assert notices == []
            print("Notifications passed: recipients, per-card bundling, read state, tray, and access revocation")
        finally:
            stop(server)


if __name__ == "__main__":
    main()
