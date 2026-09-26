#!/usr/bin/env python3
"""Compare Rizzy card-update timestamp and board-touch effects with the pinned probe."""
import json
import sqlite3
import tempfile
import time

from auth import stop
from card_update_multipart import update
from draft_workflow_parity import login, request
from search_html_parity import ROOT, rust_target


EXPECTED = {
    "empty": {"card_updated": False, "last_active_changed": False, "board_updated": True, "events_added": 0},
    "description": {"card_updated": True, "last_active_changed": False, "board_updated": True, "events_added": 0},
    "title": {"card_updated": True, "last_active_changed": True, "board_updated": True, "events_added": 1},
    "same_title": {"card_updated": False, "last_active_changed": False, "board_updated": True, "events_added": 0},
    "image": {"card_updated": True, "last_active_changed": False, "board_updated": True, "events_added": 0},
}


def snapshot(database, card_id):
    with sqlite3.connect(database) as db:
        card_updated, last_active, board_updated = db.execute(
            "SELECT c.updated_at,c.last_active_at,b.updated_at FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.id=?",
            (card_id,)).fetchone()
        events = db.execute("SELECT COUNT(*) FROM events WHERE card_id=?", (card_id,)).fetchone()[0]
    return card_updated, last_active, board_updated, events


def changes(before, after):
    return {"card_updated": after[0] > before[0],
            "last_active_changed": after[1] != before[1],
            "board_updated": after[2] > before[2],
            "events_added": after[3] - before[3]}


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        try:
            login(target)
            account = target["account"]
            status, _, raw = request(target, f"/{account}/boards", "POST",
                                     {"board": {"name": "Card update touch"}}, bearer=True)
            assert status == 201, (status, raw[:200])
            board_id = json.loads(raw)["id"]
            status, _, raw = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                     {"card": {"title": "Card update timestamp probe"}}, bearer=True)
            assert status == 201, (status, raw[:200])
            card = json.loads(raw)
            path = f"/{account}/cards/{card['number']}.json"
            before = snapshot(target["db"], card["id"])
            results = {}
            for name, payload in (
                ("empty", {}),
                ("description", {"description": "Description changed"}),
                ("title", {"title": "Card update timestamp probe renamed"}),
                ("same_title", {"title": "Card update timestamp probe renamed"}),
            ):
                time.sleep(.03)
                status, _, raw = request(target, path, "PATCH", {"card": payload}, bearer=True)
                assert status == 200, (name, status, raw[:200])
                after = snapshot(target["db"], card["id"])
                results[name] = changes(before, after)
                before = after
            time.sleep(.03)
            update(target, card["number"], {},
                   (ROOT / "upstream/test/fixtures/files/moon.jpg").read_bytes())
            results["image"] = changes(before, snapshot(target["db"], card["id"]))
            assert results == EXPECTED, (results, EXPECTED)
        finally:
            stop(process)
    print("Rizzy card no-op, description, title, and image touch effects match pinned Fizzy")


if __name__ == "__main__":
    main()
