#!/usr/bin/env python3
"""Compare activity descriptions and API particulars across the source action set."""
import json
import os
import secrets
import sqlite3
import urllib.parse
import uuid
from datetime import datetime, timezone

from activity_event_board_parity import call, database_id


ACTIONS = (
    "card_assigned", "card_auto_postponed", "card_board_changed", "card_closed",
    "card_postponed", "card_published", "card_reopened", "card_sent_back_to_triage",
    "card_title_changed", "card_triaged", "card_unassigned", "comment_created",
)


def stored_id(target, value):
    return value.to_bytes(16, "big") if target["name"] == "fizzy" else f"{value:032x}"


def public_id(target, value):
    if target["name"] == "rizzy":
        return f"{value:032x}"
    alphabet = "0123456789abcdefghijklmnopqrstuvwxyz"
    result = ""
    while value:
        value, digit = divmod(value, 36)
        result = alphabet[digit] + result
    return result.rjust(25, "0")


def add_users(target):
    base = uuid.uuid4().int & ~255
    people = {"alice": base + 1, "bob": base + 2}
    stamp = datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")
    with sqlite3.connect(target["db"]) as db:
        db.execute("PRAGMA foreign_keys=ON")
        account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?",
                                (target["account"],)).fetchone()[0]
        for label, value in people.items():
            identity = stored_id(target, uuid.uuid4().int)
            user = stored_id(target, value)
            name = {"alice": "Alice & Co", "bob": "Bob <Team>"}[label]
            email = f"activity-{secrets.token_hex(8)}@example.test"
            if target["name"] == "fizzy":
                db.execute("INSERT INTO identities(id,email_address,created_at,updated_at) VALUES(?,?,?,?)",
                           (identity, email, stamp, stamp))
                db.execute("INSERT INTO users(id,account_id,identity_id,name,role,active,verified_at,created_at,updated_at) VALUES(?,?,?,?,'member',1,?,?,?)",
                           (user, account_id, identity, name, stamp, stamp, stamp))
            else:
                db.execute("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)",
                           (identity, email, stamp))
                db.execute("INSERT INTO users(id,account_id,identity_id,name,role,active,verified_at,created_at) VALUES(?,?,?,?,'member',1,?,?)",
                           (user, account_id, identity, name, stamp, stamp))
    return {label: public_id(target, value) for label, value in people.items()}


def fixture(target):
    board = call(target, "/boards", "POST", {"board": {"name": "Activity particulars"}}, 201)
    card = call(target, f"/boards/{board['id']}/cards", "POST",
                {"card": {"title": "A&B <Card>"}}, 201)
    call(target, f"/cards/{card['number']}/comments", "POST",
         {"comment": {"body": "A comment"}}, 201)
    people = add_users(target)
    owner_id = board["creator"]["id"]
    details = {
        "card_assigned": {"assignee_ids": [people["bob"], people["alice"]]},
        "card_unassigned": {"assignee_ids": [people["bob"], people["alice"]]},
        "card_board_changed": {"particulars": {"old_board": "Board & A", "new_board": "Board <B>"}},
        "card_title_changed": {"particulars": {"old_title": "Old <Title>", "new_title": "A&B <Card>"}},
        "card_triaged": {"particulars": {"column": "In <Progress>"}},
    }
    with sqlite3.connect(target["db"]) as db:
        db.row_factory = sqlite3.Row
        event = db.execute("SELECT * FROM events WHERE board_id=? AND action='card_published' "
                           "ORDER BY created_at DESC LIMIT 1",
                           (database_id(target, board["id"]),)).fetchone()
        assert event, target["name"]
        columns = list(event.keys())
        query = f"INSERT INTO events ({','.join(columns)}) VALUES ({','.join('?' for _ in columns)})"
        additions = [(action, details.get(action, {"ignored": "not in API"}))
                     for action in ACTIONS if action not in ("card_published", "comment_created")]
        additions += [("card_assigned", {"assignee_ids": [owner_id]}),
                      ("card_unassigned", {"assignee_ids": [owner_id]})]
        for index, (action, particulars) in enumerate(additions):
            cloned = dict(event)
            cloned["id"] = stored_id(target, uuid.uuid4().int)
            cloned["action"] = action
            cloned["particulars"] = json.dumps(particulars)
            cloned["created_at"] = f"2024-01-01T00:00:{index:02d}.000Z"
            cloned["updated_at"] = cloned["created_at"]
            db.execute(query, [cloned[column] for column in columns])
    return board, people


def summary(target, board, people):
    query = urllib.parse.urlencode({"board_ids[]": board["id"]})
    events = call(target, f"/activities?{query}")
    assert len(events) == 14, (target["name"], len(events))
    observed = {}
    for event in events:
        action = event["action"]
        particulars = event["particulars"]
        key = action
        if action in ("card_assigned", "card_unassigned"):
            key += "_self" if board["creator"]["id"] in particulars["assignee_ids"] else "_others"
        assert key not in observed, (target["name"], key)
        normalized = json.loads(json.dumps(particulars))
        if "assignee_ids" in normalized:
            normalized["assignee_ids"] = [
                "ALICE" if item == people["alice"] else
                "BOB" if item == people["bob"] else
                "OWNER" if item == board["creator"]["id"] else item
                for item in normalized["assignee_ids"]
            ]
        description = event["description"].replace(board["creator"]["name"], "<OWNER>", 1)
        observed[key] = {"description": description, "particulars": normalized}
    assert len(observed) == 14
    return observed


def main():
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        fields = ("URL", "ACCOUNT", "TOKEN", "DB")
        if any(not os.getenv(f"{name}_{field}") for field in fields):
            raise SystemExit(f"Set {name}_URL, {name}_ACCOUNT, {name}_TOKEN, and {name}_DB")
        targets[name.lower()] = {"name": name.lower(),
                                 **{field.lower(): os.environ[f"{name}_{field}"] for field in fields}}
    results = {}
    for name, target in targets.items():
        board = None
        try:
            board, people = fixture(target)
            results[name] = summary(target, board, people)
        finally:
            if board:
                call(target, f"/boards/{board['id']}", "DELETE", status=204)
    assert results["fizzy"] == results["rizzy"], {
        key: (results["fizzy"].get(key), results["rizzy"].get(key))
        for key in results["fizzy"] if results["fizzy"].get(key) != results["rizzy"].get(key)
    }
    print("activity descriptions and particulars matched Fizzy for 14 action cases")


if __name__ == "__main__":
    main()
