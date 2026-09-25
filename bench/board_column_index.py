#!/usr/bin/env python3
"""Isolate the named-column card query's index cost on a 30k-card fixture."""
import json
import sqlite3
import statistics
import time


def measure(db, sql, params, repetitions=60, expected_rows=15):
    durations = []
    for _ in range(repetitions):
        start = time.perf_counter()
        rows = db.execute(sql, params).fetchall()
        assert len(rows) == expected_rows
        durations.append((time.perf_counter() - start) * 1000)
    durations.sort()
    return {
        "median_ms": round(statistics.median(durations), 3),
        "p95_ms": round(durations[int((len(durations) - 1) * 0.95)], 3),
    }


def main():
    db = sqlite3.connect(":memory:")
    db.execute("CREATE TABLE cards(id TEXT PRIMARY KEY,board_id TEXT NOT NULL,column_id TEXT,title TEXT NOT NULL,status TEXT NOT NULL,closed INTEGER NOT NULL,postponed INTEGER NOT NULL,golden INTEGER NOT NULL,last_active_at TEXT NOT NULL)")
    db.execute("CREATE TABLE closures(card_id TEXT PRIMARY KEY,created_at TEXT NOT NULL)")
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
    old_board_query = "WITH scoped AS (SELECT c.*,CASE WHEN c.closed=1 THEN 'done' WHEN c.postponed=1 THEN 'not-now' ELSE COALESCE(c.column_id,'maybe') END AS group_key,cl.created_at AS closed_at FROM cards c LEFT JOIN closures cl ON cl.card_id=c.id WHERE c.board_id=? AND c.status='published'), ranked AS (SELECT scoped.*,COUNT(*) OVER (PARTITION BY group_key) AS group_count,ROW_NUMBER() OVER (PARTITION BY group_key ORDER BY CASE WHEN closed=1 THEN closed_at END DESC,CASE WHEN closed=0 AND postponed=0 THEN golden END DESC,last_active_at DESC,id DESC) AS group_rank FROM scoped) SELECT * FROM ranked WHERE group_rank<=15 ORDER BY group_key,group_rank"
    count_query = "SELECT CASE WHEN closed=1 THEN 'done' WHEN postponed=1 THEN 'not-now' ELSE COALESCE(column_id,'maybe') END AS group_key,COUNT(*) AS total FROM cards WHERE board_id=? AND status='published' GROUP BY group_key"
    maybe_query = "SELECT * FROM cards WHERE board_id=? AND column_id IS NULL AND status='published' AND closed=0 AND postponed=0 ORDER BY golden DESC,last_active_at DESC,id DESC LIMIT 15"
    old_board = measure(db, old_board_query, ("board",), expected_rows=30)
    board_samples = []
    for _ in range(60):
        start = time.perf_counter()
        assert len(db.execute(count_query, ("board",)).fetchall()) == 2
        assert len(db.execute(maybe_query, ("board",)).fetchall()) == 15
        board_samples.append((time.perf_counter() - start) * 1000)
    board_samples.sort()
    new_board = {
        "median_ms": round(statistics.median(board_samples), 3),
        "p95_ms": round(board_samples[int((len(board_samples) - 1) * 0.95)], 3),
    }
    print(json.dumps({
        "fixture": "30,000 synthetic published cards in one board; one in twenty in target column",
        "repetitions": 60,
        "before": before,
        "after": after,
        "median_speedup": round(before["median_ms"] / after["median_ms"], 1),
        "board_initial_response": {
            "eager_all_columns": old_board,
            "maybe_and_counts_with_lazy_other_columns": new_board,
            "scope": "the new first response defers non-Maybe card rows to browser requests",
        },
        "scope": "in-memory SQLite query probe, not full HTTP or Fizzy comparison",
    }, indent=2))


if __name__ == "__main__":
    main()
