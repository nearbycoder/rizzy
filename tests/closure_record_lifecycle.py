#!/usr/bin/env python3
"""Compare closure record and completed-card HTML through close, Undo, and reclose."""
import os
import re
import sqlite3
import tempfile
import time
from datetime import datetime, timezone

from auth import stop
from draft_workflow_parity import login, request
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
    login(target)
    board = api(target, "/boards", "POST", {
        "board": {"name": "Closure record parity " + str(time.time_ns())}}, 201)
    try:
        card = api(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": "Closure lifecycle"}}, 201)
        path = f"/cards/{card['number']}/closure"
        page_path = f"/{target['account']}/cards/{card['number']}"
        def page():
            status, _, body = request(target, page_path, accept="text/html")
            assert status == 200, (target["name"], status)
            return body.decode()
        assert 'closure_notice_card_' in page()
        assert record(target, card["id"]) is None

        api(target, path, "POST", expected=204)
        first = record(target, card["id"])
        assert first and first[2] == card["creator"]["id"] and first[3] == first[4]
        assert api(target, f"/cards/{card['number']}")["closed"]
        closed_page = page()
        assert 'Completed by Owner on' in closed_page
        assert 'data-local-time-target="shortdate"' in closed_page
        shown_date = re.search(r'Completed by Owner on <time datetime="(\d+)" data-local-time-target="shortdate"', closed_page)
        stored_date = datetime.fromisoformat(first[3].replace("Z", "+00:00"))
        if stored_date.tzinfo is None:
            stored_date = stored_date.replace(tzinfo=timezone.utc)
        assert shown_date and int(shown_date.group(1)) == int(stored_date.timestamp())
        assert 'data-text="Done"' in closed_page and 'Undo' in closed_page
        assert 'id="new_step_card_' not in closed_page
        assert 'closure_notice_card_' not in closed_page

        api(target, path, "POST", expected=204)
        assert record(target, card["id"]) == first

        api(target, path, "DELETE", expected=204)
        assert record(target, card["id"]) is None
        assert not api(target, f"/cards/{card['number']}")["closed"]
        assert 'closure_notice_card_' in page()

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
            print("Closure record lifecycle and completed-card HTML parity passed:", actual)
        finally:
            stop(process)


if __name__ == "__main__":
    main()
