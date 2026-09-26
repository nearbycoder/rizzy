#!/usr/bin/env python3
"""Compare activity access and filters across two private boards and creators."""
import os
import sqlite3
import urllib.parse
import uuid

from activity_event_board_parity import call, create_member, database_id, switch_member_access
from activity_particulars_parity import stored_id


def events_for(target, board_ids, member_id, owner_id, filters):
    query = urllib.parse.urlencode(filters)
    events = call(target, "/activities" + ("?" + query if query else ""))
    observed = []
    for event in events:
        board_id = event["board"]["id"]
        if board_id not in board_ids:
            continue
        creator_id = event["creator"]["id"]
        creator = "member" if creator_id == member_id else "owner" if creator_id == owner_id else "other"
        observed.append((board_ids.index(board_id), creator))
    return sorted(observed)


def fixture(target):
    boards = []
    for index in range(2):
        board = call(target, "/boards", "POST",
                     {"board": {"name": f"Activity access {index}", "all_access": False}}, 201)
        boards.append(board)
        call(target, f"/boards/{board['id']}/cards", "POST",
             {"card": {"title": f"Access card {index}"}}, 201)
    member_id, token = create_member(target, boards[0]["id"])
    with sqlite3.connect(target["db"]) as db:
        db.row_factory = sqlite3.Row
        for board in boards:
            event = db.execute("SELECT * FROM events WHERE board_id=? AND action='card_published' "
                               "ORDER BY created_at DESC LIMIT 1",
                               (database_id(target, board["id"]),)).fetchone()
            assert event, target["name"]
            cloned = dict(event)
            cloned["id"] = stored_id(target, uuid.uuid4().int)
            cloned["creator_id"] = member_id
            columns = list(cloned)
            db.execute(f"INSERT INTO events ({','.join(columns)}) VALUES ({','.join('?' for _ in columns)})",
                       [cloned[column] for column in columns])
    return boards, member_id, token


def verify(target, boards, member_id, token):
    board_ids = [board["id"] for board in boards]
    owner_id = boards[0]["creator"]["id"]
    if target["name"] == "rizzy":
        member_public_id = member_id
    else:
        number = int.from_bytes(member_id, "big")
        alphabet = "0123456789abcdefghijklmnopqrstuvwxyz"
        digits = ""
        while number:
            number, digit = divmod(number, 36)
            digits = alphabet[digit] + digits
        member_public_id = digits.rjust(25, "0")
    member = {**target, "token": token}
    both = [("board_ids[]", board_id) for board_id in board_ids]
    result = {}
    result["owner_both"] = events_for(target, board_ids, member_public_id, owner_id, both)
    result["member_first"] = events_for(member, board_ids, member_public_id, owner_id, both)
    result["member_hidden_board"] = events_for(member, board_ids, member_public_id, owner_id,
                                                [("board_ids[]", board_ids[1])])
    result["member_creator"] = events_for(member, board_ids, member_public_id, owner_id,
                                           both + [("creator_ids[]", member_public_id)])
    result["member_owner"] = events_for(member, board_ids, member_public_id, owner_id,
                                         both + [("creator_ids[]", owner_id)])
    switch_member_access(target, member_id, board_ids[0], board_ids[1])
    result["member_second"] = events_for(member, board_ids, member_public_id, owner_id, both)
    result["member_former_board"] = events_for(member, board_ids, member_public_id, owner_id,
                                                [("board_ids[]", board_ids[0])])
    expected = {
        "owner_both": [(0, "member"), (0, "owner"), (1, "member"), (1, "owner")],
        "member_first": [(0, "member"), (0, "owner")],
        "member_hidden_board": [],
        "member_creator": [(0, "member")],
        "member_owner": [(0, "owner")],
        "member_second": [(1, "member"), (1, "owner")],
        "member_former_board": [],
    }
    assert result == expected, (target["name"], result)
    return result


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        fields = ("URL", "ACCOUNT", "TOKEN", "DB")
        if any(not os.getenv(f"{name}_{field}") for field in fields):
            raise SystemExit(f"Set {name}_URL, {name}_ACCOUNT, {name}_TOKEN, and {name}_DB")
        target = {"name": name.lower(),
                  **{field.lower(): os.environ[f"{name}_{field}"] for field in fields}}
        boards = []
        try:
            boards, member_id, token = fixture(target)
            results[name.lower()] = verify(target, boards, member_id, token)
        finally:
            for board in reversed(boards):
                call(target, f"/boards/{board['id']}", "DELETE", status=204)
    assert results["fizzy"] == results["rizzy"], results
    print("private-board activity access and creator filters matched Fizzy")


if __name__ == "__main__":
    main()
