#!/usr/bin/env python3
"""Compare supplied microsecond card and comment times with pinned Fizzy."""
import os
import sqlite3
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
import sustained_mixed as suite  # noqa: E402
from card_workflow import expect  # noqa: E402


def utc_time(raw):
    parsed = datetime.fromisoformat(raw.replace("Z", "+00:00"))
    return parsed.replace(tzinfo=timezone.utc) if parsed.tzinfo is None else parsed.astimezone(timezone.utc)


def check(name, directory, cpus):
    env, credentials = (suite.prepare_source(directory, 1) if name == "fizzy"
                        else suite.prepare_rust(directory))
    process, url = suite.start_server(name, directory, env, cpus)
    try:
        target = {"name": name, "url": url, "account": str(credentials["account"]),
                  "token": credentials["token"]}
        board, _ = expect(target, "/boards", "POST", {"board": {"name": "Precise times"}}, 201)
        created = "2024-01-15T10:30:00.123456Z"
        active = "2024-01-15T10:30:00.654321Z"
        card, _ = expect(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": "Precise card", "created_at": created,
                     "last_active_at": active}}, 201)
        assert (card["created_at"], card["last_active_at"]) == (
            "2024-01-15T10:30:00.123Z", "2024-01-15T10:30:00.654Z")
        comment, _ = expect(target, f"/cards/{card['number']}/comments", "POST", {
            "comment": {"body": "Precise comment", "created_at": created}}, 201)
        assert comment["created_at"] == "2024-01-15T10:30:00.123Z"
        changed = "2024-01-15T10:30:00.123457Z"
        later = "2024-01-15T10:30:00.654322Z"
        updated, _ = expect(target, f"/cards/{card['number']}", "PUT", {
            "card": {"created_at": changed, "last_active_at": later}}, 200)
        assert (updated["created_at"], updated["last_active_at"]) == (
            "2024-01-15T10:30:00.123Z", "2024-01-15T10:30:00.654Z")
        database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
        with sqlite3.connect(database) as db:
            row = db.execute("SELECT created_at,last_active_at FROM cards WHERE number=?",
                             (card["number"],)).fetchone()
            comment_time = db.execute(
                "SELECT m.created_at FROM comments m JOIN cards c ON c.id=m.card_id "
                "WHERE c.number=?", (card["number"],)).fetchone()[0]
        assert tuple(map(utc_time, row)) == (utc_time(changed), utc_time(later)), (name, row)
        assert utc_time(comment_time) == utc_time(created), (name, comment_time)
        return card["created_at"], comment["created_at"], updated["last_active_at"]
    finally:
        suite.stop_server(process)


def main():
    if binary := os.environ.get("RIZZY_TEST_BINARY"):
        suite.RUST = Path(binary)
    cpus = sorted(os.sched_getaffinity(0))[:2]
    with tempfile.TemporaryDirectory(prefix="rizzy-time-precision-") as temporary:
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = Path(temporary) / name
            directory.mkdir()
            results[name] = check(name, directory, cpus)
        assert results["fizzy"] == results["rizzy"]
    print("Supplied card/comment microseconds and millisecond JSON matched pinned Fizzy")


if __name__ == "__main__":
    main()
