#!/usr/bin/env python3
"""Time equivalent Fizzy and Rizzy due-card sweeps on fresh SQLite data."""

import argparse
import json
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests"))
from activity_event_board_parity import database_id  # noqa: E402
from card_workflow import expect  # noqa: E402
from sustained_mixed import (RUST, SOURCE, prepare_rust, prepare_source,
                             start_server, stop_server)  # noqa: E402

def base36(value):
    alphabet = "0123456789abcdefghijklmnopqrstuvwxyz"
    digits = []
    while value:
        value, remainder = divmod(value, 36)
        digits.append(alphabet[remainder])
    return "".join(reversed(digits)).rjust(25, "0")


def prepare_fixture(name, directory, due_count, cpus):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    if name == "rizzy":
        env["RIZZY_DISABLE_AUTO_POSTPONE"] = "1"
    process, url = start_server(name, directory, env, cpus)
    target = {"name": name, "url": url, "account": str(credentials["account"]),
              "token": credentials["token"]}
    boards = []
    try:
        for index in range(4):
            board, _ = expect(target, "/boards", "POST",
                              {"board": {"name": f"Sweep board {index}"}}, 201)
            if index >= 2:
                expect(target, f"/boards/{board['id']}/entropy", "PUT",
                       {"board": {"auto_postpone_period_in_days": 7}})
            boards.append(database_id(target, board["id"]))
    finally:
        stop_server(process)

    database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
    stamp = datetime.now(timezone.utc)
    created = stamp.isoformat(timespec="milliseconds").replace("+00:00", "Z")
    rows = []
    with sqlite3.connect(database) as db:
        db.execute("PRAGMA foreign_keys=ON")
        account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?",
                                (int(target["account"]),)).fetchone()[0]
        creator_id = db.execute("SELECT id FROM users WHERE account_id=? AND role='owner'",
                                (account_id,)).fetchone()[0]
        for index in range(due_count * 2):
            board_index = index % len(boards)
            is_due = index < due_count
            age = (40 if is_due else 2) if board_index < 2 else (8 if is_due else 6)
            last_active = (stamp - timedelta(days=age)).isoformat(
                timespec="milliseconds").replace("+00:00", "Z")
            card_id = uuid.uuid4().bytes if name == "fizzy" else base36(uuid.uuid4().int)
            rows.append((card_id, account_id, boards[board_index], creator_id,
                         1000000 + index, f"Sweep card {index}", "published",
                         created, created, last_active))
        db.executemany(
            "INSERT INTO cards(id,account_id,board_id,creator_id,number,title,status,"
            "created_at,updated_at,last_active_at) VALUES(?,?,?,?,?,?,?,?,?,?)", rows,
        )
    return env, database


def run_sweep(name, env, cpus):
    if name == "fizzy":
        script = ('start=Process.clock_gettime(Process::CLOCK_MONOTONIC); '
                  'Card.auto_postpone_all_due; '
                  'puts "AUTO_POSTPONE_SECONDS=#{format("%.6f", '
                  'Process.clock_gettime(Process::CLOCK_MONOTONIC)-start)}"')
        command, cwd = ["bin/rails", "runner", script], SOURCE
    else:
        env = {**env, "RIZZY_BENCH_AUTO_POSTPONE": "1"}
        command, cwd = [RUST, "auto-postpone"], ROOT
    result = subprocess.run(command, cwd=cwd, env=env, capture_output=True, text=True,
                            check=True, timeout=300,
                            preexec_fn=lambda: os.sched_setaffinity(0, cpus))
    output = result.stdout + "\n" + result.stderr
    match = re.search(r"AUTO_POSTPONE_SECONDS=(\d+\.\d+)", output)
    if not match:
        raise AssertionError(output[-2000:])
    return float(match.group(1))


def verify(name, database, due_count):
    def parsed_time(value):
        timestamp = datetime.fromisoformat(value.replace("Z", "+00:00"))
        return timestamp if timestamp.tzinfo else timestamp.replace(tzinfo=timezone.utc)

    with sqlite3.connect(database) as db:
        rich_text_table = ("action_text_rich_texts" if name == "fizzy"
                           else "rich_text_records")
        rows = db.execute(
            "SELECT c.number,c.created_at,c.last_active_at,"
            "(SELECT COUNT(*) FROM card_not_nows n JOIN users u ON u.id=n.user_id "
            "WHERE n.card_id=c.id AND u.role='system'),"
            "(SELECT COUNT(*) FROM events e JOIN users u ON u.id=e.creator_id "
            "WHERE e.eventable_type='Card' AND e.eventable_id=c.id "
            "AND e.action='card_auto_postponed' AND u.role='system'),"
            "(SELECT COUNT(*) FROM comments m JOIN users u ON u.id=m.creator_id "
            "WHERE m.card_id=c.id AND u.role='system'),"
            f"(SELECT COUNT(*) FROM comments m JOIN {rich_text_table} r "
            "ON r.record_type='Comment' AND r.record_id=m.id AND r.name='body' "
            "WHERE m.card_id=c.id) "
            "FROM cards c WHERE c.number>=? AND c.number<? ORDER BY c.number",
            (1000000, 1000000 + due_count * 2),
        ).fetchall()
        assert len(rows) == due_count * 2, (name, len(rows))
        for number, created, active, not_now, events, comments, rich_text in rows:
            due = number < 1000000 + due_count
            expected = 1 if due else 0
            assert (not_now, events, comments, rich_text) == (expected,) * 4, (
                name, number, not_now, events, comments, rich_text)
            assert (parsed_time(active) >= parsed_time(created)) == due, (
                name, number, created, active)
        counts = {
            "not_now": db.execute("SELECT COUNT(*) FROM card_not_nows").fetchone()[0],
            "events": db.execute(
                "SELECT COUNT(*) FROM events WHERE action='card_auto_postponed'"
            ).fetchone()[0],
            "system_comments": db.execute(
                "SELECT COUNT(*) FROM comments m JOIN users u ON u.id=m.creator_id "
                "WHERE u.role='system'"
            ).fetchone()[0],
            "recent_remaining": db.execute(
                "SELECT COUNT(*) FROM cards c WHERE c.number>=? AND NOT EXISTS "
                "(SELECT 1 FROM card_not_nows n WHERE n.card_id=c.id)",
                (1000000 + due_count,),
            ).fetchone()[0],
            "cards_verified": len(rows),
        }
    assert counts == {"not_now": due_count, "events": due_count,
                      "system_comments": due_count, "recent_remaining": due_count,
                      "cards_verified": due_count * 2}, counts
    return counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sizes", default="100,1000")
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    cpus = sorted(os.sched_getaffinity(0))[:2]
    if len(cpus) != 2:
        raise SystemExit("This benchmark requires two available CPUs")
    sizes = [int(value) for value in args.sizes.split(",")]
    rows = []
    for size in sizes:
        for round_index in range(args.rounds):
            order = ("fizzy", "rizzy") if round_index % 2 == 0 else ("rizzy", "fizzy")
            for name in order:
                with tempfile.TemporaryDirectory(prefix=f"rizzy-sweep-{name}-") as temp:
                    directory = Path(temp)
                    env, database = prepare_fixture(name, directory, size, cpus)
                    seconds = run_sweep(name, env, cpus)
                    counts = verify(name, database, size)
                    entry = {"cards_due": size, "cards_recent": size,
                             "boards": 4, "round": round_index + 1, "app": name,
                             "seconds": seconds, "cards_per_second": round(size / seconds, 2),
                             "counts": counts}
                    rows.append(entry)
                    print(json.dumps(entry), flush=True)
    result = {"source_commit": "477c943e0506f109e5bc83ae9dadbe519732c045",
              "rust_commit": subprocess.check_output(["git", "rev-parse", "HEAD"],
                                                     cwd=ROOT, text=True).strip(),
              "rust_worktree_dirty_at_run": bool(subprocess.check_output(
                  ["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()),
              "cpu_affinity": cpus, "database": "SQLite", "timing": "sweep body only",
              "fixture": "four boards, half inherited 30-day and half overridden 7-day; "
                         "equal overdue and recent published cards inserted directly",
              "runs": rows}
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
