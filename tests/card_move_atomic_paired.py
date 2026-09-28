#!/usr/bin/env python3
"""Compare board-move rollback when its event insert fails on fresh paired apps."""

import json
import os
import sqlite3
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from card_image_parity import call  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402


def check(target):
    account = target["account"]
    boards = []
    for name in ("Move source", "Move destination"):
        status, _, body = call(target, f"/{account}/boards", "POST", {"board": {"name": name}})
        assert status == 201, (target["name"], status, body[:300])
        boards.append(json.loads(body)["id"])
    source, destination = boards
    with sqlite3.connect(target["db"]) as db:
        source_db = db.execute("SELECT id FROM boards WHERE name='Move source'").fetchone()[0]
        destination_db = db.execute("SELECT id FROM boards WHERE name='Move destination'").fetchone()[0]
    status, _, body = call(target, f"/{account}/boards/{source}/cards", "POST",
                           {"card": {"title": "Atomic board move"}})
    assert status == 201, (target["name"], status, body[:300])
    card = json.loads(body)
    number = card["number"]
    with sqlite3.connect(target["db"]) as db:
        card_id = db.execute("SELECT id FROM cards WHERE number=?", (number,)).fetchone()[0]
    path = f"/{account}/cards/{number}/board"

    def state(db):
        board = db.execute("SELECT board_id FROM cards WHERE id=?", (card_id,)).fetchone()[0]
        events = db.execute("SELECT COUNT(*) FROM events WHERE " + target["event_card_filter"] +
                            " AND action='card_board_changed'", (card_id,)).fetchone()[0]
        comments = db.execute("SELECT COUNT(*) FROM comments WHERE card_id=?", (card_id,)).fetchone()[0]
        return board, events, comments

    with sqlite3.connect(target["db"]) as db:
        before = state(db)
        assert before[0] == source_db and before[1] == 0, (target["name"], before)
    for table, condition in (("events", "NEW.action='card_board_changed'"),
                             ("comments", "1=1")):
        trigger = f"fail_card_board_{table}"
        with sqlite3.connect(target["db"]) as db:
            db.execute(f"CREATE TRIGGER {trigger} BEFORE INSERT ON {table} "
                       f"WHEN {condition} BEGIN SELECT RAISE(ABORT, 'forced move failure'); END")
        try:
            status, _, _ = call(target, path, "PATCH", {"board_id": destination})
            assert status == 500, (target["name"], table, status)
            with sqlite3.connect(target["db"]) as db:
                failed = state(db)
            assert failed == before, (target["name"], table, before, failed)
        finally:
            with sqlite3.connect(target["db"]) as db:
                db.execute(f"DROP TRIGGER {trigger}")

    status, _, body = call(target, path, "PATCH", {"board_id": destination})
    assert status == 200 and json.loads(body)["board"]["id"] == destination, (target["name"], status, body[:300])
    with sqlite3.connect(target["db"]) as db:
        succeeded = state(db)
        event_time = db.execute("SELECT created_at FROM events WHERE " + target["event_card_filter"] +
                                " AND action='card_board_changed'",
                                (card_id,)).fetchone()[0]
        comment_time = db.execute("SELECT created_at FROM comments WHERE card_id=? ORDER BY created_at DESC LIMIT 1",
                                  (card_id,)).fetchone()[0]
    assert succeeded == (destination_db, 1, before[2] + 1), (target["name"], succeeded)
    assert event_time == comment_time, (target["name"], event_time, comment_time)
    return {"failed_status": 500, "rolled_back": True, "successful_status": 200,
            "events": 1, "comments_added": 1, "event_comment_time_equal": True}


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-move-atomic-") as temp:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temp) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                results[name] = check({"name": name, "url": url,
                                       "account": str(credentials["account"]),
                                       "token": credentials["token"],
                                       "event_card_filter": ("eventable_type='Card' AND eventable_id=?"
                                                             if name == "fizzy" else "card_id=?"),
                                       "db": str(directory / ("db.sqlite3" if name == "fizzy"
                                                              else "rizzy.sqlite3"))})
            assert results["fizzy"] == results["rizzy"], results
        finally:
            for process in reversed(processes):
                stop_server(process)
    print("PASS paired board moves roll back on event/comment failure and commit one event/comment")


if __name__ == "__main__":
    main()
