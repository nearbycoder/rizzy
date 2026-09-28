#!/usr/bin/env python3
"""Compare persisted Not Now records and public stamps against pinned Fizzy."""
import re
import sqlite3
import tempfile
import time
from datetime import datetime

from auth import stop
from draft_workflow_parity import login, request
from public_page_html_parity import anonymous
from search_html_parity import api, rust_target, source_target


def record(target, card_id):
    key = int(card_id, 36).to_bytes(16, "big") if target["name"] == "fizzy" else card_id
    with sqlite3.connect(target["db"]) as db:
        row = db.execute(
            "SELECT id,user_id,created_at FROM card_not_nows WHERE card_id=?", (key,)
        ).fetchone()
    if row and target["name"] == "fizzy":
        return tuple(base36(int.from_bytes(value, "big")) if isinstance(value, bytes) else value
                     for value in row)
    return row


def base36(value):
    digits = "0123456789abcdefghijklmnopqrstuvwxyz"
    output = ""
    while value:
        value, remainder = divmod(value, 36)
        output = digits[remainder] + output
    return output.rjust(25, "0")


def check(target):
    login(target)
    board = api(target, "/boards", "POST", {
        "board": {"name": "Not Now record parity " + str(time.time_ns())}}, 201)
    board_id = board["id"]
    try:
        column = api(target, f"/boards/{board_id}/columns", "POST", {
            "column": {"name": "Doing"}}, 201)
        card = api(target, f"/boards/{board_id}/cards", "POST", {
            "card": {"title": "Postponed record"}}, 201)
        actor = card["creator"]
        publication = api(target, f"/boards/{board_id}/publication", "POST", expected=201)
        key = publication["public_url"].split("/public/boards/")[1]
        path = f"/public/boards/{key}/columns/not_now"

        api(target, f"/cards/{card['number']}/not_now", "POST", expected=204)
        first = record(target, card["id"])
        assert first and first[1] == actor["id"], (target["name"], first, actor["id"])
        status, _, card_page = request(target, f"/{target['account']}/cards/{card['number']}", accept="text/html")
        assert status == 200 and b'data-text="Not Now"' in card_page
        assert b'closure_notice_card_' not in card_page and b'new_step_card_' in card_page
        html, _ = anonymous(target, path)
        article = re.search(r'<article id="article_card_' + re.escape(str(card["id"]))
                            + r'".*?</article>', html, re.S)
        assert article
        stamp_date = re.search(r'<strong class="card__closed-date">([^<]+)</strong>', article.group(0))
        stamp_by = re.search(r'<span class="card__closed-by">([^<]+)</span>', article.group(0))
        expected_date = datetime.fromisoformat(first[2].replace("Z", "+00:00")).strftime("%b %d, %Y")
        assert stamp_date and stamp_date.group(1) == expected_date
        assert stamp_by and actor["name"].split()[0] in stamp_by.group(1)

        api(target, f"/cards/{card['number']}/not_now", "POST", expected=204)
        second = record(target, card["id"])
        assert second is None and not api(target, f"/cards/{card['number']}")["postponed"]

        api(target, f"/cards/{card['number']}/not_now", "POST", expected=204)
        third = record(target, card["id"])
        assert third and third[0] != first[0] and third[1] == actor["id"]

        api(target, f"/cards/{card['number']}/closure", "POST", expected=204)
        assert record(target, card["id"]) is None
        assert api(target, f"/cards/{card['number']}")["closed"]

        api(target, f"/cards/{card['number']}/closure", "DELETE", expected=204)
        api(target, f"/cards/{card['number']}/not_now", "POST", expected=204)
        assert record(target, card["id"])
        api(target, f"/cards/{card['number']}/triage", "POST", {
            "column_id": column["id"]}, 204)
        assert record(target, card["id"]) is None
        state = api(target, f"/cards/{card['number']}")
        assert not state["postponed"] and state["column"]["id"] == column["id"]
        return {"creator_saved": True, "repeat_resumes_card": True,
                "public_stamp": True, "signed_in_stamp": True, "close_clears_record": True,
                "triage_clears_record": True}
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
            print("Not Now record parity passed:", actual)
        finally:
            stop(process)


if __name__ == "__main__":
    main()
