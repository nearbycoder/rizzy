#!/usr/bin/env python3
"""Check card move access grants and private-board cleanup on a running Rizzy."""
import hashlib
import http.cookiejar
import json
import os
import secrets
import sqlite3
import time
import urllib.request

from draft_workflow_parity import NoRedirect, request


def main():
    target = {"url": os.environ["RIZZY_URL"], "account": os.environ["RIZZY_ACCOUNT"],
              "token": os.environ["RIZZY_TOKEN"],
              "opener": urllib.request.build_opener(
                  urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
    db = sqlite3.connect(os.environ["RIZZY_DB"])
    db.execute("PRAGMA foreign_keys=ON")
    account = target["account"]
    account_id, owner_id = db.execute(
        "SELECT a.id,u.id FROM accounts a JOIN users u ON u.account_id=a.id "
        "WHERE a.external_account_id=? AND u.role='owner' LIMIT 1", (account,)).fetchone()
    identities, users, tokens, boards = [], [], [], []

    def create_user(name):
        identity_id, user_id, token_id = (secrets.token_hex(12) for _ in range(3))
        token = secrets.token_hex(32)
        timestamp = str(time.time())
        db.execute("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)",
                   (identity_id, f"{identity_id}@example.test", timestamp))
        db.execute("INSERT INTO users(id,account_id,identity_id,name,role,active,created_at) VALUES(?,?,?,?,'member',1,?)",
                   (user_id, account_id, identity_id, name, timestamp))
        db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) "
                   "VALUES(?,?,?,'Card move test','write',?)",
                   (token_id, identity_id, hashlib.sha256(token.encode()).hexdigest(), timestamp))
        db.commit()
        identities.append(identity_id)
        users.append(user_id)
        tokens.append(token_id)
        return user_id, token

    def make_board(name, all_access):
        status, _, body = request(target, f"/{account}/boards", "POST",
                                  {"board":{"name":name,"all_access":all_access}}, bearer=True)
        assert status == 201, ("board", status, body[:160])
        board_id = json.loads(body)["id"]
        boards.append(board_id)
        return board_id

    def make_card(board_id):
        status, _, body = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                  {"card":{"title":"Private move check"}}, bearer=True)
        assert status == 201, ("card", status, body[:160])
        number = json.loads(body)["number"]
        card_id = db.execute("SELECT id FROM cards WHERE account_id=? AND number=?",
                             (account_id, number)).fetchone()[0]
        return number, card_id

    def attach(card_id, user_id):
        timestamp = str(time.time())
        db.execute("INSERT INTO assignments(id,account_id,card_id,assignee_id,assigner_id,created_at) "
                   "VALUES(?,?,?,?,?,?)", (secrets.token_hex(12), account_id, card_id, user_id, owner_id, timestamp))
        db.execute("INSERT INTO pins(id,account_id,card_id,user_id,created_at) VALUES(?,?,?,?,?)",
                   (secrets.token_hex(12), account_id, card_id, user_id, timestamp))
        db.execute("INSERT INTO watches(card_id,user_id,watching) VALUES(?,?,1)", (card_id, user_id))
        db.execute("INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,unread_count,created_at,updated_at) "
                   "VALUES(?,?,?,?,?,?,'Event',1,?,?)",
                   (secrets.token_hex(12), account_id, user_id, owner_id, card_id, secrets.token_hex(12), timestamp, timestamp))
        db.commit()

    def count(table, card_id, user_id, column="user_id"):
        return db.execute(f"SELECT COUNT(*) FROM {table} WHERE card_id=? AND {column}=?",
                          (card_id, user_id)).fetchone()[0]

    try:
        assignee, _ = create_user("Move assignee")
        mover, mover_token = create_user("Move member")
        source = make_board("Move source", True)
        admin_dest = make_board("Admin destination", False)
        number, card_id = make_card(source)
        attach(card_id, assignee)
        # The owner can administer the private destination, so assignment grants access.
        status, _, body = request(target, f"/{account}/cards/{number}/board", "PATCH",
                                  {"board_id":admin_dest}, bearer=True)
        assert status == 200, ("admin move", status, body[:160])
        assert db.execute("SELECT COUNT(*) FROM accesses WHERE board_id=? AND user_id=?",
                          (admin_dest, assignee)).fetchone()[0] == 1
        for table in ("assignments", "pins", "watches", "notifications"):
            assert count(table, card_id, assignee, "assignee_id" if table == "assignments" else "user_id") == 1, table

        member_dest = make_board("Member destination", False)
        db.execute("INSERT INTO accesses(board_id,user_id) VALUES(?,?)", (member_dest, mover))
        db.commit()
        number, card_id = make_card(source)
        attach(card_id, assignee)
        member_target = {**target, "token": mover_token}
        status, _, body = request(member_target, f"/{account}/cards/{number}/board", "PATCH",
                                  {"board_id":member_dest}, bearer=True)
        assert status == 200, ("member move", status, body[:160])
        assert db.execute("SELECT COUNT(*) FROM accesses WHERE board_id=? AND user_id=?",
                          (member_dest, assignee)).fetchone()[0] == 0
        for table in ("assignments", "pins", "watches", "notifications"):
            assert count(table, card_id, assignee, "assignee_id" if table == "assignments" else "user_id") == 0, table
        print("Card move access checks passed: admin grant and member cleanup")
    finally:
        for board_id in boards:
            status, _, _ = request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)
            assert status == 204, ("cleanup board", board_id, status)
        for token_id in tokens:
            db.execute("DELETE FROM access_tokens WHERE id=?", (token_id,))
        for user_id in users:
            db.execute("DELETE FROM users WHERE id=?", (user_id,))
        for identity_id in identities:
            db.execute("DELETE FROM identities WHERE id=?", (identity_id,))
        db.commit()
        db.close()


if __name__ == "__main__":
    main()
