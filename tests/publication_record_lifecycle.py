#!/usr/bin/env python3
"""Check publication identity, key, and timestamps across repeated requests."""
import os
import sqlite3
import tempfile
import time
import urllib.error
import urllib.request

from auth import stop
from not_now_record_parity import base36
from search_html_parity import api, rust_target, source_target


def record(target, board_id):
    key = int(board_id, 36).to_bytes(16, "big") if target["name"] == "fizzy" else board_id
    with sqlite3.connect(target["db"]) as db:
        row = db.execute(
            "SELECT id,board_id,`key`,created_at,updated_at FROM board_publications WHERE board_id=?",
            (key,)).fetchone()
    if row and target["name"] == "fizzy":
        return tuple(base36(int.from_bytes(value, "big")) if isinstance(value, bytes) else value
                     for value in row)
    return row


def check(target):
    board = api(target, "/boards", "POST", {
        "board": {"name": "Publication record parity " + str(time.time_ns())}}, 201)
    path = f"/boards/{board['id']}/publication"
    try:
        assert record(target, board["id"]) is None
        published = api(target, path, "POST", expected=201)
        first = record(target, board["id"])
        assert first and first[1] == board["id"] and first[2] in published["public_url"]
        assert first[3] == first[4]
        with urllib.request.urlopen(published["public_url"], timeout=5) as response:
            assert response.status == 200

        repeated = api(target, path, "POST", expected=201)
        assert record(target, board["id"]) == first
        assert repeated["public_url"] == published["public_url"]

        api(target, path, "DELETE", expected=204)
        assert record(target, board["id"]) is None
        try:
            urllib.request.urlopen(published["public_url"], timeout=5)
        except urllib.error.HTTPError as error:
            assert error.code == 404
        else:
            raise AssertionError("unpublished board remained public")
        api(target, path, "DELETE", expected=204)
        assert record(target, board["id"]) is None

        republished = api(target, path, "POST", expected=201)
        second = record(target, board["id"])
        assert second and second[0] != first[0] and second[2] != first[2]
        assert second[2] in republished["public_url"]
        return {"repeat_preserves_identity": True, "republish_replaces_identity_and_key": True}
    finally:
        api(target, f"/boards/{board['id']}", "DELETE", expected=204)


def main():
    with tempfile.TemporaryDirectory() as temp:
        rust, process = rust_target(temp)
        try:
            actual = check(rust)
            if os.getenv("FIZZY_URL"):
                expected = check(source_target())
                assert actual == expected, (expected, actual)
            print("Publication record lifecycle passed:", actual)
        finally:
            stop(process)


if __name__ == "__main__":
    main()
