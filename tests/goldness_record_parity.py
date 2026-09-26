#!/usr/bin/env python3
"""Compare Golden Ticket record lifetime and card activity with pinned Fizzy."""
import sqlite3
import tempfile
import time
import re

from auth import stop
from not_now_record_parity import base36
from public_page_html_parity import anonymous
from search_html_parity import api, rust_target, source_target


def record(target, card_id):
    key = int(card_id, 36).to_bytes(16, "big") if target["name"] == "fizzy" else card_id
    with sqlite3.connect(target["db"]) as db:
        row = db.execute(
            "SELECT id,created_at,updated_at FROM card_goldnesses WHERE card_id=?", (key,)
        ).fetchone()
    if row and target["name"] == "fizzy":
        return (base36(int.from_bytes(row[0], "big")), row[1], row[2])
    return row


def check(target):
    board = api(target, "/boards", "POST", {
        "board": {"name": "Goldness record parity " + str(time.time_ns())}}, 201)
    board_id = board["id"]
    try:
        card = api(target, f"/boards/{board_id}/cards", "POST", {
            "card": {"title": "Golden Ticket record"}}, 201)
        publication = api(target, f"/boards/{board_id}/publication", "POST", expected=201)
        key = publication["public_url"].split("/public/boards/")[1]

        def public_golden():
            html, _ = anonymous(target, f"/public/boards/{key}/columns/stream")
            article = re.search(r'<article id="article_card_' + re.escape(str(card["id"]))
                                + r'"[^>]*>', html)
            assert article, (target["name"], card["id"])
            return "golden-effect" in article.group(0)

        path = f"/cards/{card['number']}/goldness"
        initial_activity = api(target, f"/cards/{card['number']}")["last_active_at"]
        assert not public_golden()
        api(target, path, "POST", expected=204)
        first = record(target, card["id"])
        assert first and first[1] == first[2]
        state = api(target, f"/cards/{card['number']}")
        assert state["golden"] and state["last_active_at"] == initial_activity
        assert public_golden()

        api(target, path, "POST", expected=204)
        assert record(target, card["id"]) == first
        assert api(target, f"/cards/{card['number']}")["last_active_at"] == initial_activity

        api(target, path, "DELETE", expected=204)
        assert record(target, card["id"]) is None
        state = api(target, f"/cards/{card['number']}")
        assert not state["golden"] and state["last_active_at"] == initial_activity
        assert not public_golden()

        api(target, path, "POST", expected=204)
        second = record(target, card["id"])
        assert second and second[0] != first[0]
        assert api(target, f"/cards/{card['number']}")["last_active_at"] == initial_activity
        assert public_golden()
        return {"record_created": True, "repeat_preserves_record": True,
                "ungild_removes_record": True, "regild_creates_record": True,
                "activity_unchanged": True, "public_preview": True}
    finally:
        api(target, f"/boards/{board_id}", "DELETE", expected=204)


def main():
    source = source_target()
    with tempfile.TemporaryDirectory() as temp:
        rust, process = rust_target(temp)
        try:
            expected = check(source)
            actual = check(rust)
            assert actual == expected, (expected, actual)
            print("Goldness record parity passed:", actual)
        finally:
            stop(process)


if __name__ == "__main__":
    main()
