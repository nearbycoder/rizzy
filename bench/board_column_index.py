#!/usr/bin/env python3
"""Isolate the named-column card query's index cost on a 30k-card fixture."""
import json
import sqlite3
import statistics
import time


def measure(db, sql, params, repetitions=60):
    durations = []
    for _ in range(repetitions):
        start = time.perf_counter()
        rows = db.execute(sql, params).fetchall()
        assert len(rows) == 15
        durations.append((time.perf_counter() - start) * 1000)
    durations.sort()
    return {
        "median_ms": round(statistics.median(durations), 3),
        "p95_ms": round(durations[int((len(durations) - 1) * 0.95)], 3),
    }


def main():
    db = sqlite3.connect(":memory:")
    db.execute("CREATE TABLE cards(id TEXT PRIMARY KEY,board_id TEXT NOT NULL,column_id TEXT,title TEXT NOT NULL,status TEXT NOT NULL,closed INTEGER NOT NULL,postponed INTEGER NOT NULL,golden INTEGER NOT NULL,last_active_at TEXT NOT NULL)")
    db.execute("CREATE INDEX cards_board_active ON cards(board_id,status,closed,postponed,last_active_at DESC)")
    timestamp = "2026-09-25T12:00:00.000Z"
    rows = [
        (f"card-{i:05d}", "board", "target" if i % 20 == 0 else None,
         f"Probe {i}", "published", 0, 0, i % 23 == 0, timestamp)
        for i in range(30_000)
    ]
    db.executemany("INSERT INTO cards VALUES(?,?,?,?,?,?,?,?,?)", rows)
    query = "SELECT * FROM cards WHERE board_id=? AND column_id=? AND status='published' AND closed=0 AND postponed=0 ORDER BY golden DESC,last_active_at DESC,id DESC LIMIT 15"
    params = ("board", "target")
    before = {
        "query_plan": [row[3] for row in db.execute("EXPLAIN QUERY PLAN " + query, params)],
        **measure(db, query, params),
    }
    db.execute("CREATE INDEX cards_board_column_active ON cards(board_id,column_id,status,closed,postponed,golden DESC,last_active_at DESC,id DESC)")
    after = {
        "query_plan": [row[3] for row in db.execute("EXPLAIN QUERY PLAN " + query, params)],
        **measure(db, query, params),
    }
    print(json.dumps({
        "fixture": "30,000 synthetic published cards in one board; one in twenty in target column",
        "repetitions": 60,
        "before": before,
        "after": after,
        "median_speedup": round(before["median_ms"] / after["median_ms"], 1),
        "scope": "in-memory SQLite query probe, not full HTTP or Fizzy comparison",
    }, indent=2))


if __name__ == "__main__":
    main()
