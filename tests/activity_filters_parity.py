#!/usr/bin/env python3
"""Compare creator and board filters on equivalent activity fixtures."""
import os
import sqlite3
import urllib.parse
import uuid

from activity_event_board_parity import call, database_id
from activity_particulars_parity import add_users, stored_id


def fixture(target):
    boards = []
    people = add_users(target)
    for index in range(2):
        board = call(target, "/boards", "POST",
                     {"board": {"name": f"Activity filter {index}", "all_access": False}}, 201)
        boards.append(board)
        call(target, f"/boards/{board['id']}/cards", "POST",
             {"card": {"title": f"Filter card {index}"}}, 201)
        with sqlite3.connect(target["db"]) as db:
            db.row_factory = sqlite3.Row
            event = db.execute("SELECT * FROM events WHERE board_id=? AND action='card_published' "
                               "ORDER BY created_at DESC LIMIT 1",
                               (database_id(target, board["id"]),)).fetchone()
            assert event, target["name"]
            cloned = dict(event)
            cloned["id"] = stored_id(target, uuid.uuid4().int)
            cloned["creator_id"] = database_id(target, people["alice"])
            columns = list(cloned)
            db.execute(f"INSERT INTO events ({','.join(columns)}) VALUES ({','.join('?' for _ in columns)})",
                       [cloned[column] for column in columns])
    return boards, people


def verify(target, boards, people):
    owner = boards[0]["creator"]["id"]
    board_ids = [board["id"] for board in boards]
    cases = {
        "one_board": [("board_ids[]", board_ids[0])],
        "both_boards": [("board_ids[]", board_id) for board_id in board_ids],
        "one_creator": [("board_ids[]", board_id) for board_id in board_ids]
                       + [("creator_ids[]", people["alice"])],
        "both_creators": [("board_ids[]", board_id) for board_id in board_ids]
                         + [("creator_ids[]", owner), ("creator_ids[]", people["alice"])],
        "and_filters": [("board_ids[]", board_ids[1]), ("creator_ids[]", people["alice"])],
        "scalar_board": [("board_ids", board_ids[0])],
        "scalar_creator": [("board_ids[]", board_ids[0]), ("creator_ids", people["alice"])],
    }
    result = {}
    for name, fields in cases.items():
        events = call(target, "/activities?" + urllib.parse.urlencode(fields))
        selected = []
        for event in events:
            if event["board"]["id"] in board_ids:
                selected.append((board_ids.index(event["board"]["id"]),
                                 "alice" if event["creator"]["id"] == people["alice"] else "owner"))
        result[name] = sorted(selected)
    expected = {
        "one_board": [(0, "alice"), (0, "owner")],
        "both_boards": [(0, "alice"), (0, "owner"), (1, "alice"), (1, "owner")],
        "one_creator": [(0, "alice"), (1, "alice")],
        "both_creators": [(0, "alice"), (0, "owner"), (1, "alice"), (1, "owner")],
        "and_filters": [(1, "alice")],
        "scalar_board": [(0, "alice"), (0, "owner")],
        "scalar_creator": [(0, "alice")],
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
            boards, people = fixture(target)
            results[name.lower()] = verify(target, boards, people)
        finally:
            for board in reversed(boards):
                call(target, f"/boards/{board['id']}", "DELETE", status=204)
    assert results["fizzy"] == results["rizzy"], results
    print("activity board, creator, and combined filters matched Fizzy")


if __name__ == "__main__":
    main()
