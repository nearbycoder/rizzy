#!/usr/bin/env python3
"""Measure the named-column read versus column-delete index trade-off."""

import argparse
import json
import os
import sqlite3
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

from column_delete import fixture
from sustained_mixed import ROOT, prepare_rust, start_server, stop_server


INDEXES = {
    "wide": None,
    "pair": "CREATE INDEX cards_board_column_active ON cards(board_id,column_id)",
    "partial_order": "CREATE INDEX cards_board_column_active ON cards(board_id,column_id,golden DESC,last_active_at DESC,id DESC) WHERE status='published' AND closed=0 AND postponed=0",
}
READ = "SELECT * FROM cards WHERE board_id=? AND column_id=? AND status='published' AND closed=0 AND postponed=0 ORDER BY golden DESC,last_active_at DESC,id DESC LIMIT 15"
COUNT = "SELECT COUNT(*) FROM cards WHERE board_id=? AND column_id=? AND status='published' AND closed=0 AND postponed=0"


def timed(db, sql, args):
    started = time.perf_counter()
    cursor = db.execute(sql, args)
    return cursor, round((time.perf_counter() - started) * 1000, 3)


def sample(db, item, repetitions):
    read_times = []
    count_times = []
    for _ in range(repetitions + 1):
        started = time.perf_counter()
        rows = db.execute(READ, (item["board_id"], item["delete_id"])).fetchall()
        read_times.append((time.perf_counter() - started) * 1000)
        started = time.perf_counter()
        count = db.execute(COUNT, (item["board_id"], item["delete_id"])).fetchone()[0]
        count_times.append((time.perf_counter() - started) * 1000)
    assert len(rows) == 15 and count == 1 + (item["size"] - 3 + 2) // 3
    db.isolation_level = None
    db.execute("BEGIN IMMEDIATE")
    db.execute("INSERT INTO card_bulk_touches(board_id) VALUES(?)", (item["board_id"],))
    _, nullify = timed(db,
        "UPDATE cards SET column_id=NULL WHERE board_id=? AND column_id=?",
        (item["board_id"], item["delete_id"]))
    _, touch = timed(db, "UPDATE cards SET updated_at=? WHERE board_id=?",
                     ("2026-09-26T00:00:00.000Z", item["board_id"]))
    db.execute("DELETE FROM columns WHERE id=?", (item["delete_id"],))
    db.execute("DELETE FROM card_bulk_touches WHERE board_id=?", (item["board_id"],))
    db.execute("COMMIT")
    return {"read_ms": round(statistics.median(read_times[1:]), 3),
            "count_ms": round(statistics.median(count_times[1:]), 3),
            "nullify_ms": nullify, "touch_ms": touch}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=30000)
    parser.add_argument("--trials", type=int, default=3)
    parser.add_argument("--repetitions", type=int, default=100)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    assert args.cards >= 15 and args.trials > 0 and args.repetitions > 0
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-column-index-") as temporary:
        directory = Path(temporary)
        env, credentials = prepare_rust(directory)
        process, url = start_server("rizzy", directory, env,
                                    sorted(os.sched_getaffinity(0))[:2])
        try:
            target = {"name": "rizzy", "url": url,
                      "account": str(credentials["account"]),
                      "token": credentials["token"],
                      "database": directory / "rizzy.sqlite3"}
            item = fixture(target, args.cards, 0)
            result = {"cards": args.cards,
                      "deleted_column_cards": 1 + (args.cards - 1) // 3,
                      "trials": args.trials, "read_repetitions": args.repetitions,
                      "variants": {}}
            with sqlite3.connect(target["database"]) as source:
                for name, ddl in INDEXES.items():
                    samples = []
                    plan = []
                    for trial in range(args.trials):
                        with sqlite3.connect(directory / f"{name}-{trial}.sqlite3") as db:
                            source.backup(db)
                            db.execute("PRAGMA synchronous=NORMAL")
                            if ddl:
                                db.execute("DROP INDEX cards_board_column_active")
                                db.execute(ddl)
                                db.commit()
                            if not plan:
                                plan = [row[3] for row in db.execute(
                                    "EXPLAIN QUERY PLAN " + READ,
                                    (item["board_id"], item["delete_id"]))]
                            samples.append(sample(db, item, args.repetitions))
                    result["variants"][name] = {
                        "plan": plan,
                        "median_ms": {key: round(statistics.median(
                            entry[key] for entry in samples), 3) for key in samples[0]},
                        "samples": samples,
                    }
            if args.output:
                args.output.parent.mkdir(parents=True, exist_ok=True)
                args.output.write_text(json.dumps(result, indent=2) + "\n")
            print(json.dumps(result, indent=2))
        finally:
            stop_server(process)


if __name__ == "__main__":
    main()
