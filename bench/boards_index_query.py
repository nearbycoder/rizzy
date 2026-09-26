#!/usr/bin/env python3
"""Measure the board index page query with and without its recent-access index."""
import json
import sqlite3
import statistics
import time


COUNT = 30_000
SQL = ("SELECT b.* FROM boards b JOIN accesses x ON x.board_id=b.id "
       "WHERE b.account_id=? AND x.user_id=? "
       "ORDER BY x.accessed_at DESC LIMIT ? OFFSET ?")
PARAMS = ("account", "user", 100, 55)


def measure(db):
    for _ in range(5):
        assert len(db.execute(SQL, PARAMS).fetchall()) == 100
    samples = []
    for _ in range(40):
        start = time.perf_counter()
        db.execute(SQL, PARAMS).fetchall()
        samples.append((time.perf_counter() - start) * 1000)
    return {
        "median_ms": round(statistics.median(samples), 3),
        "query_plan": [row[3] for row in db.execute("EXPLAIN QUERY PLAN " + SQL, PARAMS)],
    }


def main():
    with sqlite3.connect(":memory:") as db:
        db.executescript("""
            CREATE TABLE boards (id INTEGER PRIMARY KEY, account_id TEXT NOT NULL, name TEXT);
            CREATE TABLE accesses (board_id INTEGER PRIMARY KEY, user_id TEXT NOT NULL,
                                   accessed_at TEXT);
            CREATE INDEX accesses_user_board ON accesses(user_id, board_id);
        """)
        db.executemany("INSERT INTO boards VALUES (?, 'account', ?)",
                       ((i, f"Board {i}") for i in range(COUNT)))
        db.executemany("INSERT INTO accesses VALUES (?, 'user', ?)",
                       ((i, f"2030-01-01T{i // 3600:02}:{i // 60 % 60:02}:{i % 60:02}Z")
                        for i in range(COUNT)))
        before = measure(db)
        db.execute("CREATE INDEX accesses_user_recent ON accesses(user_id, accessed_at DESC)")
        after = measure(db)
        print(json.dumps({"fixture_boards": COUNT, "page_limit": PARAMS[2],
                          "page_offset": PARAMS[3], "warm_reads": 40,
                          "before": before, "after": after}, indent=2))


if __name__ == "__main__":
    main()
