#!/usr/bin/env python3
"""Compare activity scoping to an event board rather than its card's board."""
import hashlib
import os
import secrets
import sqlite3
import urllib.parse
import uuid
from datetime import datetime, timezone

from parity import request


def database_id(target, public_id):
    if target["name"] == "rizzy":
        return public_id
    return int(public_id, 36).to_bytes(16, "big")


def call(target, path, method="GET", payload=None, status=200):
    actual, body = request(target, path, method, payload)
    assert actual == status, (target["name"], method, path, actual, body)
    return body


def setup(target):
    card_board = call(target, "/boards", "POST",
                      {"board": {"name": "Activity card board", "all_access": False}}, 201)
    event_board = call(target, "/boards", "POST",
                       {"board": {"name": "Activity event board", "all_access": False}}, 201)
    card = call(target, f"/boards/{card_board['id']}/cards", "POST",
                {"card": {"title": "Event board scope"}}, 201)
    card_board_id = database_id(target, card_board["id"])
    event_board_id = database_id(target, event_board["id"])
    with sqlite3.connect(target["db"]) as db:
        db.row_factory = sqlite3.Row
        published = db.execute(
            "SELECT * FROM events WHERE board_id=? AND action='card_published' "
            "ORDER BY created_at DESC LIMIT 1", (card_board_id,)
        ).fetchone()
        assert published, target["name"]
        db.execute("UPDATE events SET board_id=? WHERE id=?", (event_board_id, published["id"]))
        cloned = dict(published)
        cloned["id"] = uuid.uuid4().hex if target["name"] == "rizzy" else uuid.uuid4().bytes
        cloned["board_id"] = event_board_id
        cloned["action"] = "internal_audit"
        columns = list(cloned)
        db.execute(f"INSERT INTO events ({','.join(columns)}) VALUES ({','.join('?' for _ in columns)})",
                   [cloned[column] for column in columns])
    return card_board, event_board, card


def create_member(target, board_id):
    stamp = datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")

    def new_id():
        return uuid.uuid4().bytes if target["name"] == "fizzy" else secrets.token_hex(12)

    identity_id, user_id, token_id = new_id(), new_id(), new_id()
    token = secrets.token_hex(24)
    with sqlite3.connect(target["db"]) as db:
        db.execute("PRAGMA foreign_keys=ON")
        account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?",
                                (target["account"],)).fetchone()[0]
        if target["name"] == "fizzy":
            db.execute("INSERT INTO identities(id,email_address,created_at,updated_at) VALUES(?,?,?,?)",
                       (identity_id, f"activity-{token}@example.test", stamp, stamp))
            db.execute("INSERT INTO users(id,account_id,identity_id,name,role,active,verified_at,created_at,updated_at) VALUES(?,?,?,'Activity member','member',1,?,?,?)",
                       (user_id, account_id, identity_id, stamp, stamp, stamp))
            db.execute("INSERT INTO identity_access_tokens(id,identity_id,token,description,permission,created_at,updated_at) VALUES(?,?,?,'Activity parity','write',?,?)",
                       (token_id, identity_id, token, stamp, stamp))
            db.execute("INSERT INTO accesses(id,account_id,board_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)",
                       (new_id(), account_id, database_id(target, board_id), user_id, stamp, stamp))
        else:
            db.execute("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)",
                       (identity_id, f"activity-{token}@example.test", stamp))
            db.execute("INSERT INTO users(id,account_id,identity_id,name,role,active,verified_at,created_at) VALUES(?,?,?,'Activity member','member',1,?,?)",
                       (user_id, account_id, identity_id, stamp, stamp))
            db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES(?,?,?,'Activity parity','write',?)",
                       (token_id, identity_id, hashlib.sha256(token.encode()).hexdigest(), stamp))
            db.execute("INSERT INTO accesses(board_id,user_id) VALUES(?,?)", (board_id, user_id))
    return user_id, token


def switch_member_access(target, user_id, old_board, new_board):
    with sqlite3.connect(target["db"]) as db:
        db.execute("DELETE FROM accesses WHERE board_id=? AND user_id=?",
                   (database_id(target, old_board), user_id))
        if target["name"] == "fizzy":
            stamp = datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")
            account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?",
                                    (target["account"],)).fetchone()[0]
            db.execute("INSERT INTO accesses(id,account_id,board_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)",
                       (uuid.uuid4().bytes, account_id, database_id(target, new_board), user_id, stamp, stamp))
        else:
            db.execute("INSERT INTO accesses(board_id,user_id) VALUES(?,?)", (new_board, user_id))


def verify(target, card_board, event_board, card):
    event_filter = urllib.parse.urlencode({"board_ids[]": event_board["id"]})
    card_filter = urllib.parse.urlencode({"board_ids[]": card_board["id"]})
    events = call(target, f"/activities?{event_filter}")
    assert len(events) == 1, (target["name"], events)
    event = events[0]
    assert event["action"] == "card_published"
    assert event["board"]["id"] == event_board["id"]
    assert event["eventable"]["id"] == card["id"]
    assert event["eventable"]["board"]["id"] == card_board["id"]
    assert call(target, f"/activities?{card_filter}") == []
    user_id, token = create_member(target, event_board["id"])
    member = {**target, "token": token}
    assert len(call(member, f"/activities?{event_filter}")) == 1
    switch_member_access(target, user_id, event_board["id"], card_board["id"])
    assert call(member, f"/activities?{event_filter}") == []
    return event["action"], event["board"]["name"], event["eventable"]["board"]["name"]


def main():
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        missing = [field for field in ("URL", "ACCOUNT", "TOKEN", "DB")
                   if not os.getenv(f"{name}_{field}")]
        if missing:
            raise SystemExit(f"Set {name}_" + ", ".join(missing))
        targets[name.lower()] = {"name": name.lower(),
                                 **{field.lower(): os.environ[f"{name}_{field}"]
                                    for field in ("URL", "ACCOUNT", "TOKEN", "DB")}}
    observed = {}
    for name, target in targets.items():
        card_board = event_board = None
        try:
            card_board, event_board, card = setup(target)
            observed[name] = verify(target, card_board, event_board, card)
        finally:
            if event_board:
                call(target, f"/boards/{event_board['id']}", "DELETE", status=204)
            if card_board:
                call(target, f"/boards/{card_board['id']}", "DELETE", status=204)
    assert observed["fizzy"] == observed["rizzy"], observed
    print("activity action and event-board scoping matched Fizzy on both apps")


if __name__ == "__main__":
    main()
