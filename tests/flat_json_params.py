#!/usr/bin/env python3
"""Exercise the pinned source FlatJsonParamsTest cases against Rizzy."""
import http.cookiejar
import os
import sqlite3
import tempfile
import urllib.request
from pathlib import Path

from auth import start, stop
from draft_workflow_parity import NoRedirect, request as http_request
from search_html_parity import api, rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        checked = []
        try:
            def request(name, path, method, body, status, verify=None):
                result = api(target, path, method, body, status)
                if verify:
                    assert verify(result), (name, result)
                checked.append(name)
                return result

            board = request("board create", "/boards", "POST", {"name": "Flat board"}, 201,
                            lambda result: result["name"] == "Flat board")
            board_id = board["id"]
            request("board update", f"/boards/{board_id}", "PUT",
                    {"name": "Flat updated", "auto_postpone_period_in_days": 7,
                     "public_description": "<p>Flat public desc</p>"}, 200,
                    lambda result: result["name"] == "Flat updated")
            request("board entropy", f"/boards/{board_id}/entropy", "PUT",
                    {"auto_postpone_period_in_days": 90}, 200)
            card = request("card create", f"/boards/{board_id}/cards", "POST",
                           {"title": "Flat card", "description": "<p>Flat description</p>"}, 201,
                           lambda result: result["title"] == "Flat card")
            number = card["number"]
            request("card update", f"/cards/{number}", "PUT",
                    {"title": "Flat update", "description": "<p>Updated flat</p>"}, 200,
                    lambda result: result["title"] == "Flat update")
            column = request("column create", f"/boards/{board_id}/columns", "POST",
                             {"name": "Flat Column"}, 201)
            request("column update", f"/boards/{board_id}/columns/{column['id']}", "PUT",
                    {"name": "Flat Updated"}, 200)
            step = request("step create", f"/cards/{number}/steps", "POST", {"content": "Flat step"}, 201)
            request("step update", f"/cards/{number}/steps/{step['id']}", "PUT",
                    {"content": "Flat updated"}, 200)
            request("card reaction", f"/cards/{number}/reactions", "POST", {"content": "🎉"}, 201)
            comment = request("comment create", f"/cards/{number}/comments", "POST", {"body": "Comment"}, 201)
            request("comment reaction", f"/cards/{number}/comments/{comment['id']}/reactions", "POST",
                    {"content": "👍"}, 201)
            request("notification settings", "/notifications/settings", "PUT",
                    {"bundle_email_frequency": "every_few_hours"}, 204)
            request("join code", "/account/join_code", "PUT", {"usage_limit": 5}, 204)
            request("account settings", "/account/settings", "PUT", {"name": "New Name"}, 204)
            request("account entropy", "/account/entropy", "PUT",
                    {"auto_postpone_period_in_days": 7}, 200)
            request("access token", "/my/access_tokens", "POST",
                    {"description": "Flat token", "permission": "read"}, 201)
            webhook = request("webhook create", f"/boards/{board_id}/webhooks", "POST",
                              {"name": "Flat Webhook", "url": "https://example.com/flat",
                               "subscribed_actions": ["card_published"]}, 201)
            request("webhook update", f"/boards/{board_id}/webhooks/{webhook['id']}", "PATCH",
                    {"name": "Flat Updated", "subscribed_actions": ["card_published"]}, 200)
            with sqlite3.connect(target["db"]) as db:
                account_id, owner_id = db.execute(
                    "SELECT account_id,creator_id FROM boards WHERE id=?", (board_id,)).fetchone()
                db.execute("INSERT INTO users(id,account_id,name,role,created_at) "
                           "VALUES('flat-member',?,'Flat Member','member','2026-01-01T00:00:00Z')", (account_id,))
            request("user update", f"/users/{owner_id}", "PUT", {"name": "Flat Name"}, 200)
            request("user role", "/users/flat-member/role", "PUT", {"role": "admin"}, 204)
            request("user via join", "/users/joins", "POST", {"name": "Flat Join"}, 204)
            request("push subscription", f"/users/{owner_id}/push_subscriptions", "POST",
                    {"endpoint": "https://fcm.googleapis.com/fcm/send/flatjson",
                     "p256dh_key": "key1", "auth_key": "key2"}, 201)
            with sqlite3.connect(target["db"]) as db:
                assert db.execute("SELECT name FROM boards WHERE id=?", (board_id,)).fetchone()[0] == "Flat updated"
                assert db.execute("SELECT auto_postpone_period_in_days,public_description FROM boards WHERE id=?",
                                  (board_id,)).fetchone() == (90, "<p>Flat public desc</p>")
                assert db.execute("SELECT title,description FROM cards WHERE id=?", (card["id"],)).fetchone() == (
                    "Flat update", "<p>Updated flat</p>")
                assert db.execute("SELECT name FROM columns WHERE id=?", (column["id"],)).fetchone()[0] == "Flat Updated"
                assert db.execute("SELECT content FROM steps WHERE id=?", (step["id"],)).fetchone()[0] == "Flat updated"
                assert db.execute("SELECT COUNT(*) FROM reactions WHERE reactable_type='Card' AND reactable_id=? AND content='🎉'",
                                  (card["id"],)).fetchone()[0] == 1
                assert db.execute("SELECT COUNT(*) FROM reactions WHERE reactable_type='Comment' AND reactable_id=? AND content='👍'",
                                  (comment["id"],)).fetchone()[0] == 1
                assert db.execute("SELECT bundle_email_frequency FROM notification_settings WHERE user_id=?",
                                  (owner_id,)).fetchone()[0] == "every_few_hours"
                assert db.execute("SELECT usage_limit FROM account_join_codes WHERE account_id=?",
                                  (account_id,)).fetchone()[0] == 5
                assert db.execute("SELECT role FROM users WHERE id='flat-member'").fetchone()[0] == "admin"
                assert db.execute("SELECT name FROM users WHERE id=?", (owner_id,)).fetchone()[0] == "Flat Join"
                assert db.execute("SELECT name,auto_postpone_period_in_days FROM accounts WHERE id=?",
                                  (account_id,)).fetchone() == ("New Name", 7)
                assert db.execute("SELECT COUNT(*) FROM access_tokens WHERE description='Flat token' AND permission='read'").fetchone()[0] == 1
                assert db.execute("SELECT name FROM webhooks WHERE id=?", (webhook["id"],)).fetchone()[0] == "Flat Updated"
                assert db.execute("SELECT p256dh_key,auth_key FROM push_subscriptions WHERE user_id=?",
                                  (owner_id,)).fetchone() == ("key1", "key2")
        finally:
            stop(process)

        signup_db = Path(temp) / "flat-signup.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{signup_db}",
               "RIZZY_MULTI_TENANT": "1", "RIZZY_DEV_MAGIC_LINKS": "1"}
        port, process = start(env)
        signup_target = {"url": f"http://127.0.0.1:{port}", "opener": urllib.request.build_opener(
            NoRedirect(), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))}
        try:
            status, headers, _ = http_request(signup_target, "/signup", "POST",
                                              {"email_address": "flat-signup@example.test"})
            assert status == 201, status
            checked.append("signup create")
            status, _, _ = http_request(signup_target, "/session/magic_link", "POST",
                                        {"code": headers["X-Magic-Link-Code"]})
            assert status == 200, status
            status, _, _ = http_request(signup_target, "/signup/completion", "POST",
                                        {"full_name": "Flat JSON User"})
            assert status == 201, status
            checked.append("signup complete")
            with sqlite3.connect(signup_db) as db:
                assert db.execute("SELECT name FROM users WHERE role='owner'").fetchone()[0] == "Flat JSON User"
        finally:
            stop(process)
        assert len(checked) == 25, checked  # The source's 24 cases plus a comment fixture.
        print(f"Flat JSON requests passed on all 24 pinned source cases: {', '.join(checked)}")


if __name__ == "__main__":
    main()
