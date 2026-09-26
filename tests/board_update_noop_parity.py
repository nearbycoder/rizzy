#!/usr/bin/env python3
"""Compare repeated board updates, Action Text state, and board touch behavior."""
import os
import sqlite3

from activity_event_board_parity import call, database_id


def state(target, board_id):
    stored_id = database_id(target, board_id)
    with sqlite3.connect(target["db"]) as db:
        if target["name"] == "fizzy":
            stamp = db.execute("SELECT updated_at FROM boards WHERE id=?", (stored_id,)).fetchone()[0]
            rich = db.execute("SELECT body FROM action_text_rich_texts WHERE record_type='Board' "
                              "AND record_id=? AND name='public_description'", (stored_id,)).fetchone()
            entropy = db.execute("SELECT COUNT(*) FROM entropies WHERE container_type='Board' "
                                 "AND container_id=?", (stored_id,)).fetchone()[0]
            rich_state = "absent" if rich is None else "null" if rich[0] is None else "empty" if rich[0] == "" else "text"
        else:
            stamp, body = db.execute("SELECT updated_at,public_description FROM boards WHERE id=?",
                                     (stored_id,)).fetchone()
            rich = db.execute("SELECT body_is_null FROM rich_text_records WHERE record_type='Board' "
                              "AND record_id=? AND name='description'", (stored_id,)).fetchone()
            entropy = db.execute("SELECT COUNT(*) FROM entropy_records WHERE container_type='Board' "
                                 "AND container_id=?", (stored_id,)).fetchone()[0]
            rich_state = "absent" if rich is None else "null" if rich[0] else "empty" if body == "" else "text"
        accesses = db.execute("SELECT COUNT(*) FROM accesses WHERE board_id=?", (stored_id,)).fetchone()[0]
    return stamp, rich_state, entropy, accesses


def check(target):
    board = call(target, "/boards", "POST",
                 {"board": {"name": "No-op board updates", "all_access": False}}, 201)
    board_id = board["id"]
    owner_id = board["creator"]["id"]
    result = []
    steps = [
        ("same name", {"board": {"name": "No-op board updates"}}),
        ("same access mode", {"board": {"all_access": False}}),
        ("same period first", {"board": {"auto_postpone_period_in_days": board["auto_postpone_period_in_days"]}}),
        ("same period again", {"board": {"auto_postpone_period_in_days": board["auto_postpone_period_in_days"]}}),
        ("null description", {"board": {"public_description": None}}),
        ("null description again", {"board": {"public_description": None}}),
        ("empty description", {"board": {"public_description": ""}}),
        ("empty description again", {"board": {"public_description": ""}}),
        ("same members", {"board": {"name": "No-op board updates"}, "user_ids": [owner_id]}),
        ("text description", {"board": {"public_description": "Hi"}}),
        ("text description again", {"board": {"public_description": "Hi"}}),
        ("null after text", {"board": {"public_description": None}}),
        ("all access", {"board": {"all_access": True}}),
        ("all access again", {"board": {"all_access": True}}),
    ]
    try:
        before = state(target, board_id)
        assert before[1:] == ("absent", 0, 1), (target["name"], before)
        assert call(target, f"/boards/{board_id}", "PATCH",
                    {"board": {}}, 400) == {"status": 400, "error": "Bad Request"}
        assert state(target, board_id) == before
        for label, payload in steps:
            call(target, f"/boards/{board_id}", "PATCH", payload, 200)
            after = state(target, board_id)
            result.append((label, after[0] != before[0], after[1], after[2],
                           after[3] > before[3]))
            before = after
        null_board = call(target, "/boards", "POST",
                          {"board": {"name": "Created null description", "public_description": None}}, 201)
        try:
            assert state(target, null_board["id"])[1] == "null"
        finally:
            call(target, f"/boards/{null_board['id']}", "DELETE", status=204)
        return result
    finally:
        call(target, f"/boards/{board_id}", "DELETE", status=204)


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        fields = ("URL", "ACCOUNT", "TOKEN", "DB")
        if any(not os.getenv(f"{name}_{field}") for field in fields):
            raise SystemExit(f"Set {name}_URL, {name}_ACCOUNT, {name}_TOKEN, and {name}_DB")
        target = {"name": name.lower(),
                  **{field.lower(): os.environ[f"{name}_{field}"] for field in fields}}
        results[name.lower()] = check(target)
    assert results["fizzy"] == results["rizzy"], results
    print("Repeated board update timestamps, rich-text states, and access changes matched Fizzy")


if __name__ == "__main__":
    main()
