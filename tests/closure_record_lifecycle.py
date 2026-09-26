#!/usr/bin/env python3
"""Exercise persisted closure identity through repeat, reopen, and reclose."""
import os
import sqlite3
import tempfile
import time

from auth import stop
from not_now_record_parity import base36
from search_html_parity import api, rust_target, source_target


def record(target, card_id):
    key = int(card_id, 36).to_bytes(16, "big") if target["name"] == "fizzy" else card_id
    with sqlite3.connect(target["db"]) as db:
        row = db.execute(
            "SELECT id,account_id,user_id,created_at,updated_at FROM closures WHERE card_id=?", (key,)
        ).fetchone()
    if row and target["name"] == "fizzy":
        return tuple(base36(int.from_bytes(value, "big")) if isinstance(value, bytes) else value
                     for value in row)
    return row


def check(target):
    board = api(target, "/boards", "POST", {
        "board": {"name": "Closure record parity " + str(time.time_ns())}}, 201)
    try:
        card = api(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": "Closure lifecycle"}}, 201)
        path = f"/cards/{card['number']}/closure"
        assert record(target, card["id"]) is None

        api(target, path, "POST", expected=204)
        first = record(target, card["id"])
        assert first and first[2] == card["creator"]["id"] and first[3] == first[4]
        assert api(target, f"/cards/{card['number']}")["closed"]

        api(target, path, "POST", expected=204)
        assert record(target, card["id"]) == first

        api(target, path, "DELETE", expected=204)
        assert record(target, card["id"]) is None
        assert not api(target, f"/cards/{card['number']}")["closed"]

        api(target, path, "DELETE", expected=204)
        assert record(target, card["id"]) is None

        api(target, path, "POST", expected=204)
        second = record(target, card["id"])
        assert second and second[0] != first[0] and second[2] == first[2]
        return {"identity_replaced_after_reclose": True, "repeat_preserves_record": True}
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
            print("Closure record lifecycle passed:", actual)
        finally:
            stop(process)


if __name__ == "__main__":
    main()
