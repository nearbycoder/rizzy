#!/usr/bin/env python3
"""Compare SQLite column-delete update orders on an identical copied fixture."""

import argparse
import hashlib
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


def run(db, item, strategy):
    board, column = item["board_id"], item["delete_id"]
    stamp = "2026-09-27T00:00:00.000Z"
    db.isolation_level = None
    started = time.perf_counter()
    db.execute("BEGIN IMMEDIATE")
    if strategy not in ("split_epoch", "split_epoch_nonnull_index"):
        db.execute("INSERT INTO card_bulk_touches(board_id) VALUES(?)", (board,))
    if strategy in ("split", "split_without_update_triggers", "split_nonnull_index"):
        db.execute("UPDATE cards SET column_id=NULL WHERE board_id=? AND column_id=?", (board, column))
        db.execute("UPDATE cards SET updated_at=? WHERE board_id=?", (stamp, board))
    elif strategy == "disjoint":
        db.execute("UPDATE cards SET updated_at=? WHERE board_id=? AND column_id IS NOT ?", (stamp, board, column))
        db.execute("UPDATE cards SET column_id=NULL,updated_at=? WHERE board_id=? AND column_id=?", (stamp, board, column))
    elif strategy == "single":
        db.execute("UPDATE cards SET column_id=CASE WHEN column_id=? THEN NULL ELSE column_id END,updated_at=? WHERE board_id=?", (column, stamp, board))
    elif strategy in ("split_epoch", "split_epoch_nonnull_index"):
        db.execute("UPDATE cards SET column_id=NULL,bulk_touch_epoch=bulk_touch_epoch+1 WHERE board_id=? AND column_id=?", (board, column))
        db.execute("UPDATE cards SET updated_at=?,bulk_touch_epoch=bulk_touch_epoch+1 WHERE board_id=?", (stamp, board))
    elif strategy == "split_epoch_touch":
        db.execute("UPDATE cards SET column_id=NULL WHERE board_id=? AND column_id=?", (board, column))
        db.execute("UPDATE cards SET updated_at=?,bulk_touch_epoch=bulk_touch_epoch+1 WHERE board_id=?", (stamp, board))
    else:
        raise AssertionError(strategy)
    db.execute("DELETE FROM columns WHERE id=? AND board_id=?", (column, board))
    if strategy not in ("split_epoch", "split_epoch_nonnull_index"):
        db.execute("DELETE FROM card_bulk_touches WHERE board_id=?", (board,))
    db.execute("UPDATE boards SET updated_at=? WHERE id=?", (stamp, board))
    db.execute("COMMIT")
    elapsed = (time.perf_counter() - started) * 1000
    assert db.execute("SELECT COUNT(*) FROM cards WHERE board_id=? AND updated_at=?", (board, stamp)).fetchone()[0] == item["size"]
    assert db.execute("SELECT COUNT(*) FROM cards WHERE board_id=? AND column_id=?", (board, column)).fetchone()[0] == 0
    assert db.execute("SELECT COUNT(*) FROM cards WHERE board_id=? AND column_id=?", (board, item["keep_id"])).fetchone()[0] == 1 + (item["size"] - 2) // 3
    assert db.execute("SELECT COUNT(*) FROM cards WHERE board_id=? AND last_active_at=?", (board, item["marker"])).fetchone()[0] == item["size"]
    assert db.execute("SELECT updated_at FROM columns WHERE id=?", (item["keep_id"],)).fetchone()[0] == item["keep_stamp"]
    assert db.execute("SELECT updated_at FROM boards WHERE id=?", (board,)).fetchone()[0] == stamp
    return round(elapsed, 3)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=30000)
    parser.add_argument("--trials", type=int, default=5)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-column-delete-strategy-") as temporary:
        directory = Path(temporary)
        env, credentials = prepare_rust(directory)
        process, url = start_server("rizzy", directory, env, sorted(os.sched_getaffinity(0))[:2])
        try:
            target = {"name": "rizzy", "url": url, "account": str(credentials["account"]),
                      "token": credentials["token"], "database": directory / "rizzy.sqlite3"}
            item = fixture(target, args.cards, 0)
            samples = {name: [] for name in ("split", "disjoint", "single", "split_without_update_triggers", "split_epoch", "split_epoch_touch", "split_nonnull_index", "split_epoch_nonnull_index")}
            with sqlite3.connect(target["database"]) as source:
                for trial in range(args.trials):
                    for name in list(samples)[trial % len(samples):] + list(samples)[:trial % len(samples)]:
                        with sqlite3.connect(directory / f"{name}-{trial}.sqlite3") as db:
                            source.backup(db)
                            db.execute("PRAGMA synchronous=NORMAL")
                            if name == "split_without_update_triggers":
                                db.execute("DROP TRIGGER cards_touch_board_update")
                                db.execute("DROP TRIGGER cards_touch_column_update")
                            elif name in ("split_epoch", "split_epoch_touch", "split_epoch_nonnull_index"):
                                for trigger_name in ("cards_touch_board_update", "cards_touch_column_update"):
                                    definition = db.execute("SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?", (trigger_name,)).fetchone()[0]
                                    db.execute("DROP TRIGGER " + trigger_name)
                                    db.execute(definition.replace(" WHEN ", " WHEN NEW.bulk_touch_epoch=OLD.bulk_touch_epoch AND ", 1))
                            if name in ("split_nonnull_index", "split_epoch_nonnull_index"):
                                db.execute("DROP INDEX cards_board_column_active")
                                db.execute("CREATE INDEX cards_board_column_active ON cards(board_id,column_id,status,closed,postponed,golden DESC,last_active_at DESC,id DESC) WHERE column_id IS NOT NULL")
                            samples[name].append(run(db, item, name))
            binary = ROOT / "target/release/rizzy"
            with binary.open("rb") as executable:
                binary_hash = hashlib.file_digest(executable, "sha256").hexdigest()
            result = {"cards": args.cards, "trials": args.trials,
                      "rizzy_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                      "rizzy_binary_sha256": binary_hash,
                      "note": "Direct SQLite transaction probe; dropping update triggers is an unattainable upper-bound comparison, not an application implementation.",
                      "samples_ms": samples,
                      "median_ms": {name: statistics.median(values) for name, values in samples.items()}}
            if args.output:
                args.output.parent.mkdir(parents=True, exist_ok=True)
                args.output.write_text(json.dumps(result, indent=2) + "\n")
            print(json.dumps(result, indent=2))
        finally:
            stop_server(process)


if __name__ == "__main__":
    main()
