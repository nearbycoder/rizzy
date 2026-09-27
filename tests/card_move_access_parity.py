#!/usr/bin/env python3
"""Compare private-board card move access and cleanup against Fizzy."""
import hashlib
import json
import os
import secrets
import sqlite3
import time
import urllib.request
import uuid
from datetime import datetime, timezone

from draft_workflow_parity import NoRedirect, request


def timestamp():
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def database_id(target, value):
    return int(value, 36).to_bytes(16, "big") if target["name"] == "fizzy" and isinstance(value, str) else value


def new_id(target):
    return uuid.uuid4().bytes if target["name"] == "fizzy" else secrets.token_hex(12)


def run(target):
    account = target["account"]
    db = sqlite3.connect(target["db"])
    db.execute("PRAGMA foreign_keys=ON")
    account_id, owner_id = db.execute(
        "SELECT a.id,u.id FROM accounts a JOIN users u ON u.account_id=a.id "
        "WHERE a.external_account_id=? AND u.role='owner' AND u.active=1 LIMIT 1",
        (account,)).fetchone()
    created = {"identities": [], "users": [], "tokens": [], "boards": []}

    def create_user(name):
        identity_id, user_id, token_id = (new_id(target) for _ in range(3))
        token = secrets.token_hex(32)
        stamp = timestamp()
        email_id = secrets.token_hex(12)
        if target["name"] == "fizzy":
            db.execute("INSERT INTO identities(id,email_address,created_at,updated_at) VALUES(?,?,?,?)",
                       (identity_id, f"{email_id}@example.test", stamp, stamp))
            db.execute("INSERT INTO users(id,account_id,identity_id,name,role,active,verified_at,created_at,updated_at) VALUES(?,?,?,?,'member',1,?,?,?)",
                       (user_id, account_id, identity_id, name, stamp, stamp, stamp))
            db.execute("INSERT INTO identity_access_tokens(id,identity_id,token,description,permission,created_at,updated_at) VALUES(?,?,?,'Card move parity','write',?,?)",
                       (token_id, identity_id, token, stamp, stamp))
        else:
            db.execute("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)",
                       (identity_id, f"{email_id}@example.test", stamp))
            db.execute("INSERT INTO users(id,account_id,identity_id,name,role,active,verified_at,created_at) VALUES(?,?,?,?,'member',1,?,?)",
                       (user_id, account_id, identity_id, name, stamp, stamp))
            db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES(?,?,?,'Card move parity','write',?)",
                       (token_id, identity_id, hashlib.sha256(token.encode()).hexdigest(), stamp))
        db.commit()
        created["identities"].append(identity_id)
        created["users"].append(user_id)
        created["tokens"].append(token_id)
        return user_id, token

    def board(name, all_access):
        status, _, raw = request(target, f"/{account}/boards", "POST",
                                 {"board": {"name": name, "all_access": all_access}}, bearer=True)
        assert status == 201, (target["name"], "board", status, raw[:150])
        board_id = json.loads(raw)["id"]
        created["boards"].append(board_id)
        return board_id

    def card(board_id):
        status, _, raw = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                 {"card": {"title": "Private move parity"}}, bearer=True)
        assert status == 201, (target["name"], "card", status, raw[:150])
        number = json.loads(raw)["number"]
        card_id = db.execute("SELECT id FROM cards WHERE account_id=? AND number=?",
                             (account_id, number)).fetchone()[0]
        event_id = db.execute("SELECT id FROM events WHERE eventable_type='Card' AND eventable_id=? ORDER BY created_at DESC LIMIT 1",
                              (card_id,)).fetchone()[0]
        return number, card_id, event_id

    def attach(card_id, event_id, assignee):
        stamp = timestamp()
        db.execute("INSERT INTO assignments(id,account_id,card_id,assignee_id,assigner_id,created_at"
                   + (",updated_at) VALUES(?,?,?,?,?,?,?)" if target["name"] == "fizzy" else ") VALUES(?,?,?,?,?,?)"),
                   (new_id(target), account_id, card_id, assignee, owner_id, stamp)
                   + ((stamp,) if target["name"] == "fizzy" else ()))
        db.execute("INSERT INTO pins(id,account_id,card_id,user_id,created_at"
                   + (",updated_at) VALUES(?,?,?,?,?,?)" if target["name"] == "fizzy" else ") VALUES(?,?,?,?,?)"),
                   (new_id(target), account_id, card_id, assignee, stamp)
                   + ((stamp,) if target["name"] == "fizzy" else ()))
        db.execute("INSERT INTO watches(id,account_id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,?,1,?,?)"
                   if target["name"] == "fizzy" else
                   "INSERT INTO watches(id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,1,?,?)",
                   (new_id(target), account_id, card_id, assignee, stamp, stamp)
                   if target["name"] == "fizzy" else
                   (new_id(target), card_id, assignee, stamp, stamp))
        db.execute("INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,unread_count,created_at,updated_at) VALUES(?,?,?,?,?,?,'Event',1,?,?)",
                   (new_id(target), account_id, assignee, owner_id, card_id, event_id, stamp, stamp))
        db.commit()

    def state(board_id, card_id, event_id, assignee):
        access = db.execute("SELECT COUNT(*) FROM accesses WHERE board_id=? AND user_id=?",
                            (database_id(target, board_id), assignee)).fetchone()[0]
        counts = [access]
        for table, column in (("assignments", "assignee_id"), ("pins", "user_id"),
                              ("watches", "user_id")):
            counts.append(db.execute(f"SELECT COUNT(*) FROM {table} WHERE card_id=? AND {column}=?",
                                     (card_id, assignee)).fetchone()[0])
        counts.append(db.execute("SELECT COUNT(*) FROM notifications WHERE card_id=? AND user_id=? AND source_type='Event' AND source_id=?",
                                 (card_id, assignee, event_id)).fetchone()[0])
        return tuple(counts)

    def move(number, destination, token):
        status, _, raw = request({**target, "token": token}, f"/{account}/cards/{number}/board",
                                 "PATCH", {"board_id": destination}, bearer=True)
        assert status == 200, (target["name"], "move", status, raw[:150])

    def check_event_move(card_id, event_id, destination):
        actual_board = db.execute("SELECT board_id FROM events WHERE id=?", (event_id,)).fetchone()[0]
        assert actual_board == database_id(target, destination), (target["name"], "old event board")
        move_events = db.execute("SELECT COUNT(*) FROM events WHERE eventable_type='Card' AND eventable_id=? AND action='card_board_changed'",
                                 (card_id,)).fetchone()[0]
        assert move_events == 1, (target["name"], "move event", move_events)

    try:
        assignee, assignee_token = create_user("Parity assignee")
        mover, mover_token = create_user("Parity member")
        source = board("Parity source", True)
        admin_destination = board("Parity admin destination", False)
        number, card_id, event_id = card(source)
        attach(card_id, event_id, assignee)
        move(number, admin_destination, target["token"])
        check_event_move(card_id, event_id, admin_destination)
        admin_state = state(admin_destination, card_id, event_id, assignee)
        assert admin_state == (1, 1, 1, 1, 1), (target["name"], "admin state", admin_state)

        member_destination = board("Parity member destination", False)
        if target["name"] == "fizzy":
            stamp = timestamp()
            db.execute("INSERT INTO accesses(id,account_id,board_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)",
                       (new_id(target), account_id, database_id(target, member_destination), mover, stamp, stamp))
        else:
            db.execute("INSERT INTO accesses(board_id,user_id) VALUES(?,?)",
                       (member_destination, mover))
        db.commit()
        number, card_id, event_id = card(source)
        attach(card_id, event_id, assignee)
        move(number, member_destination, mover_token)
        check_event_move(card_id, event_id, member_destination)
        deadline = time.monotonic() + 15
        member_state = state(member_destination, card_id, event_id, assignee)
        while member_state != (0, 0, 0, 0, 0) and time.monotonic() < deadline:
            time.sleep(.2)
            member_state = state(member_destination, card_id, event_id, assignee)
        assert member_state == (0, 0, 0, 0, 0), (target["name"], "member state", member_state)
        pin_status, _, _ = request({**target, "token": assignee_token},
                                   f"/{account}/cards/{number}/pin", "POST", bearer=True)
        assert pin_status == 404, (target["name"], "pin after access loss", pin_status)
        assert state(member_destination, card_id, event_id, assignee) == member_state
        return {"admin": admin_state, "member": member_state,
                "pin_after_access_loss": pin_status}
    finally:
        for board_id in created["boards"]:
            request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)
        token_table = "identity_access_tokens" if target["name"] == "fizzy" else "access_tokens"
        for token_id in created["tokens"]:
            db.execute(f"DELETE FROM {token_table} WHERE id=?", (token_id,))
        for user_id in created["users"]:
            db.execute("DELETE FROM users WHERE id=?", (user_id,))
        for identity_id in created["identities"]:
            db.execute("DELETE FROM identities WHERE id=?", (identity_id,))
        db.commit()
        db.close()


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        target = {"name": name.lower(), "url": os.environ[f"{name}_URL"],
                  "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
                  "db": os.environ[f"{name}_DB"], "opener": urllib.request.build_opener(NoRedirect())}
        results[name.lower()] = run(target)
    assert results["fizzy"] == results["rizzy"], results
    print("Card move access parity matched: admin grants, member cleanup, and pin rejection after access loss")


if __name__ == "__main__":
    main()
