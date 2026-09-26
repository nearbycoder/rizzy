#!/usr/bin/env python3
"""Check access identity and timestamps across board involvement updates."""
import os
import sqlite3
import tempfile
import time

from auth import stop
from not_now_record_parity import base36
from search_html_parity import api, rust_target, source_target


def record(target, board_id, user_id):
    if target["name"] == "fizzy":
        board_id = int(board_id, 36).to_bytes(16, "big")
        user_id = int(user_id, 36).to_bytes(16, "big")
        query = ("SELECT id,board_id,user_id,involvement,accessed_at,created_at,updated_at "
                 "FROM accesses WHERE board_id=? AND user_id=?")
    else:
        query = ("SELECT r.id,r.board_id,r.user_id,a.involvement,a.accessed_at,r.created_at,r.updated_at "
                 "FROM access_records r JOIN accesses a ON a.board_id=r.board_id AND a.user_id=r.user_id "
                 "WHERE r.board_id=? AND r.user_id=?")
    with sqlite3.connect(target["db"]) as db:
        row = db.execute(query, (board_id, user_id)).fetchone()
    if row and target["name"] == "fizzy":
        return tuple(base36(int.from_bytes(value, "big")) if isinstance(value, bytes) else value
                     for value in row)
    return row


def check(target):
    board = api(target, "/boards", "POST", {
        "board": {"name": "Access record parity " + str(time.time_ns())}}, 201)
    owner = board["creator"]["id"]
    path = f"/boards/{board['id']}/involvement"
    try:
        first = record(target, board["id"], owner)
        assert first and first[3] == "watching" and first[4] is None and first[5] == first[6]

        time.sleep(.02)
        api(target, path, "PUT", {"involvement": "access_only"}, expected=204)
        changed = record(target, board["id"], owner)
        assert changed and changed[:3] == first[:3]
        assert changed[3] == "access_only" and changed[5] == first[5] and changed[6] > first[6]

        time.sleep(.02)
        api(target, path, "PUT", {"involvement": "watching"}, expected=204)
        restored = record(target, board["id"], owner)
        assert restored and restored[:3] == first[:3] and restored[3] == "watching"
        assert restored[6] > changed[6]
        return {"stable_identity": True, "update_time_tracks_involvement": True}
    finally:
        api(target, f"/boards/{board['id']}", "DELETE", expected=204)
        assert record(target, board["id"], owner) is None


def main():
    with tempfile.TemporaryDirectory() as temp:
        rust, process = rust_target(temp)
        try:
            actual = check(rust)
            if os.getenv("FIZZY_URL"):
                expected = check(source_target())
                assert actual == expected, (expected, actual)
            print("Access record lifecycle passed:", actual)
        finally:
            stop(process)


if __name__ == "__main__":
    main()
