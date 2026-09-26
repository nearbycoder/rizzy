#!/usr/bin/env python3
"""Compare the due-card sweep with pinned Fizzy on two entropy periods."""

import os
import sqlite3
import subprocess
import sys
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from sustained_mixed import (RUST, SOURCE, prepare_rust, prepare_source,
                             start_server, stop_server)  # noqa: E402
from activity_event_board_parity import database_id  # noqa: E402


def stored_state(name, directory, cards):
    database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
    with sqlite3.connect(database) as db:
        state = {}
        for label, card_id in cards.items():
            row = db.execute("SELECT column_id FROM cards WHERE id=?", (card_id,)).fetchone()
            not_now = db.execute(
                "SELECT u.role FROM card_not_nows n LEFT JOIN users u ON u.id=n.user_id WHERE n.card_id=?",
                (card_id,),
            ).fetchone()
            events = db.execute(
                "SELECT COUNT(*) FROM events WHERE eventable_type='Card' AND eventable_id=? "
                "AND action='card_auto_postponed'", (card_id,),
            ).fetchone()[0]
            state[label] = (not_now is not None, None if not_now is None else not_now[0],
                            row[0] is None, events)
        return state


def system_comments(name, directory, cards):
    database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
    with sqlite3.connect(database) as db:
        return {label: db.execute(
            "SELECT COUNT(*) FROM comments m JOIN users u ON u.id=m.creator_id "
            "WHERE m.card_id=? AND u.role='system'", (card_id,),
        ).fetchone()[0] for label, card_id in cards.items()}


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    if name == "rizzy":
        env["RIZZY_DISABLE_AUTO_POSTPONE"] = "1"
    process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
    target = {"name": name, "url": url, "account": str(credentials["account"]),
              "token": credentials["token"]}
    try:
        cards = {}
        for label, period in (("inherited", None), ("override", 7)):
            board, _ = expect(target, "/boards", "POST",
                              {"board": {"name": "Auto postpone " + label}}, 201)
            path = f"/boards/{board['id']}"
            column, _ = expect(target, path + "/columns", "POST",
                               {"column": {"name": "Doing"}}, 201)
            if period is not None:
                expect(target, path + "/entropy", "PUT",
                       {"board": {"auto_postpone_period_in_days": period}})
            cases = (("due", 40 if period is None else 8),
                     ("recent", 2 if period is None else 6),
                     ("closed", 40 if period is None else 8),
                     ("already", 40 if period is None else 8))
            for suffix, age in cases:
                card, _ = expect(target, path + "/cards", "POST",
                                 {"card": {"title": label + " " + suffix}}, 201)
                expect(target, f"/cards/{card['number']}/triage", "POST",
                       {"column_id": column["id"]}, 204)
                if suffix == "closed":
                    expect(target, f"/cards/{card['number']}/closure", "POST", status=204)
                elif suffix == "already":
                    expect(target, f"/cards/{card['number']}/not_now", "POST", status=204)
                cards[label + "_" + suffix] = (database_id(target, card["id"]), age)
    finally:
        stop_server(process)
    database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
    with sqlite3.connect(database) as db:
        for card_id, age in cards.values():
            stamp = (datetime.now(timezone.utc) - timedelta(days=age)).isoformat(
                timespec="milliseconds").replace("+00:00", "Z")
            db.execute("UPDATE cards SET last_active_at=? WHERE id=?", (stamp, card_id))
    ids = {label: card_id for label, (card_id, _) in cards.items()}
    before = stored_state(name, directory, ids)
    comments_before = system_comments(name, directory, ids)
    if name == "fizzy":
        command = ["bin/rails", "runner", "Card.auto_postpone_all_due"]
        cwd = SOURCE
    else:
        command = [RUST, "auto-postpone"]
        cwd = ROOT
    first = subprocess.run(command, cwd=cwd, env=env, capture_output=True, text=True, check=True)
    after = stored_state(name, directory, ids)
    comments_after = system_comments(name, directory, ids)
    second = subprocess.run(command, cwd=cwd, env=env, capture_output=True, text=True, check=True)
    repeated = stored_state(name, directory, ids)
    assert repeated == after, (name, first.stdout, second.stdout, after, repeated)
    assert all(after[label][0] and after[label][1] == "system" and after[label][2]
               and after[label][3] == 1 for label in ("inherited_due", "override_due")), (name, after)
    assert all(after[label] == before[label] for label in
               ("inherited_recent", "override_recent", "inherited_closed", "override_closed",
                "inherited_already", "override_already")), (name, before, after)
    comment_delta = {label: comments_after[label] - comments_before[label] for label in ids}
    assert all(comment_delta[label] == 1 for label in
               ("inherited_due", "override_due")), (name, comment_delta)
    return before, after, comment_delta


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-auto-postpone-") as temp:
        root = Path(temp)
        source = root / "fizzy"
        rust = root / "rizzy"
        source.mkdir()
        rust.mkdir()
        left, right = check("fizzy", source), check("rizzy", rust)
        assert left == right, (left, right)
        print("Auto postponement matched pinned Fizzy for inherited, override, recent, closed, and already postponed cards")


if __name__ == "__main__":
    main()
