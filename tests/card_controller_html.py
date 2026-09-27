#!/usr/bin/env python3
"""Check pinned CardsController HTML title, controls, deletion, and permissions."""
import hashlib
import http.cookiejar
import json
import sqlite3
import tempfile
import urllib.request

from auth import stop
from draft_workflow_parity import NoRedirect, login, request
from search_html_parity import rust_target


def api(target, path, method="GET", body=None, expected=200):
    status, headers, raw = request(target, f"/{target['account']}{path}", method, body, bearer=True)
    assert status == expected, (target["email"], method, path, status, raw[:200])
    return json.loads(raw) if raw else None


def html(target, path, method="GET", body=None, expected=200):
    status, headers, raw = request(target, f"/{target['account']}{path}", method, body,
                                   accept="text/html")
    assert status == expected, (target["email"], method, path, status, raw[:200])
    return headers, raw.decode()


def main():
    with tempfile.TemporaryDirectory() as temp:
        owner, process = rust_target(temp)
        owner["opener"] = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
        member = {**owner, "email": "card-member@example.test", "token": "card-member-token",
                  "opener": urllib.request.build_opener(
                      urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
        try:
            login(owner)
            board = api(owner, "/boards", "POST", {"name": "Card controller", "all_access": True}, 201)
            board_id = board["id"]
            title = "Fix the `bug` in production <script>"
            owner_card = api(owner, f"/boards/{board_id}/cards", "POST", {"title": title}, 201)
            with sqlite3.connect(owner["db"]) as db:
                account_id, owner_id = db.execute(
                    "SELECT account_id,creator_id FROM boards WHERE id=?", (board_id,)).fetchone()
                db.execute("INSERT INTO identities(id,email_address,created_at) "
                           "VALUES('card-member-identity',?,'2026-01-01T00:00:00Z')", (member["email"],))
                db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) "
                           "VALUES('card-member',?,'card-member-identity','Card Member','member','2026-01-01T00:00:00Z')",
                           (account_id,))
                db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) "
                           "VALUES('card-member-token-id','card-member-identity',?,'Test','write','2026-01-01T00:00:00Z')",
                           (hashlib.sha256(member["token"].encode()).hexdigest(),))
                db.execute("INSERT INTO accesses(board_id,user_id) VALUES(?,'card-member')", (board_id,))
            login(member)

            card_path = f"/cards/{owner_card['number']}"
            _, owner_page = html(owner, card_path)
            assert '<code>bug</code>' in owner_page and '&lt;script&gt;' in owner_page
            assert 'class="card__title-link"' in owner_page
            assert f'id="closure_notice_card_{owner_card["id"]}"' in owner_page
            assert f'action="/{owner["account"]}{card_path}/self_assignment"' in owner_page
            assert '<button type="submit" hidden>Assign to me</button>' in owner_page
            assert "Delete this card" in owner_page
            _, member_page = html(member, card_path)
            assert "Delete this card" not in member_page
            html(member, card_path, "DELETE", expected=403)
            assert api(owner, card_path)["id"] == owner_card["id"]

            with sqlite3.connect(owner["db"]) as db:
                db.execute("UPDATE boards SET auto_postpone_period_in_days=7 WHERE id=?", (board_id,))
                db.execute("UPDATE cards SET last_active_at='2030-03-10T15:00:00.000Z' WHERE id=?",
                           (owner_card["id"],))
            _, scheduled_page = html(owner, card_path)
            assert 'data-local-time-target="indays"' in scheduled_page
            assert 'datetime="1899990000"' in scheduled_page  # March 17, 2030 at 15:00 UTC
            api(owner, card_path + "/closure", "POST", expected=204)
            closed_page = html(owner, card_path)[1]
            assert 'closure_notice_card_' not in closed_page
            assert 'Completed by Owner on' in closed_page and 'data-local-time-target="shortdate"' in closed_page
            assert 'id="undo-closure"' in closed_page and 'data-text="Done"' in closed_page
            assert 'id="new-step"' not in closed_page
            api(owner, card_path + "/closure", "DELETE", expected=204)
            assert f'id="closure_notice_card_{owner_card["id"]}"' in html(owner, card_path)[1]
            api(owner, card_path + "/not_now", "POST", expected=204)
            postponed_page = html(owner, card_path)[1]
            assert 'closure_notice_card_' not in postponed_page and 'data-text="Not Now"' in postponed_page
            api(owner, card_path + "/not_now", "DELETE", expected=204)

            member_card = api(member, f"/boards/{board_id}/cards", "POST",
                              {"title": "Member's card"}, 201)
            member_path = f"/cards/{member_card['number']}"
            _, member_own_page = html(member, member_path)
            assert "Delete this card" in member_own_page
            headers, _ = html(member, member_path, "DELETE", expected=302)
            assert headers["Location"].endswith(f"/{owner['account']}/boards/{board_id}")
            with sqlite3.connect(owner["db"]) as db:
                assert db.execute("SELECT COUNT(*) FROM cards WHERE id=?", (member_card["id"],)).fetchone()[0] == 0

            member_board = api(member, "/boards", "POST", {"name": "Member board"}, 201)
            owner_on_member_board = api(owner, f"/boards/{member_board['id']}/cards", "POST",
                                        {"title": "Owner's card"}, 201)
            foreign_path = f"/cards/{owner_on_member_board['number']}"
            _, member_page = html(member, foreign_path)
            assert "Delete this card" not in member_page
            html(member, foreign_path, "DELETE", expected=403)
            assert api(owner, foreign_path)["id"] == owner_on_member_board["id"]

            headers, _ = html(owner, card_path, "POST", b"_method=delete", expected=302)
            assert headers["Location"].endswith(f"/{owner['account']}/boards/{board_id}")
            with sqlite3.connect(owner["db"]) as db:
                assert db.execute("SELECT COUNT(*) FROM cards WHERE id=?", (owner_card["id"],)).fetchone()[0] == 0
            assert api(owner, foreign_path, "DELETE", expected=204) is None
        finally:
            stop(process)
    print("Card HTML title, inactivity notice, self-assignment, deletion, and permissions passed")


if __name__ == "__main__":
    main()
