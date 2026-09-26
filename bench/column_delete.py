#!/usr/bin/env python3
"""Compare Fizzy and Rizzy column deletion on equal SQLite card counts.

Cards are cloned from records created through each app. Fixture insertion is
outside the timed request; only the authenticated DELETE is measured.
"""

import argparse
import json
import os
import sqlite3
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

from card_workflow import expect
from sustained_mixed import ROOT, prepare_rust, prepare_source, start_server, stop_server


def fixture(target, size, trial):
    board, _ = expect(target, "/boards", "POST",
                      {"board": {"name": f"Column delete {size} #{trial}"}}, 201)
    path = f"/boards/{board['id']}"
    columns = []
    for label in ("Delete", "Keep"):
        column, _ = expect(target, path + "/columns", "POST",
                           {"column": {"name": label}}, 201)
        columns.append(column)
    cards = []
    for label in ("Delete card", "Keep card", "Maybe card"):
        card, _ = expect(target, path + "/cards", "POST",
                         {"card": {"title": label}}, 201)
        cards.append(card)
    for card, column in zip(cards[:2], columns):
        expect(target, f"/cards/{card['number']}/triage", "POST",
               {"column_id": column["id"]}, 204)

    database = target["database"]
    with sqlite3.connect(database) as db:
        db.row_factory = sqlite3.Row
        templates = [dict(db.execute("SELECT * FROM cards WHERE number=?",
                                     (card["number"],)).fetchone()) for card in cards]
        board_id = templates[0]["board_id"]
        delete_id, keep_id = templates[0]["column_id"], templates[1]["column_id"]
        names = list(templates[0])
        placeholders = ",".join("?" for _ in names)
        sql = f"INSERT INTO cards({','.join(names)}) VALUES({placeholders})"
        next_number = db.execute(
            "SELECT COALESCE(MAX(number),0)+1 FROM cards WHERE account_id=?",
            (templates[0]["account_id"],)).fetchone()[0]
        rows = []
        for index in range(size - 3):
            row = templates[index % 3].copy()
            row["id"] = uuid.uuid4().bytes if target["name"] == "fizzy" else "0" + uuid.uuid4().hex[:24]
            row["number"] = next_number + index
            row["title"] = f"Fixture card {index}"
            rows.append(tuple(row[name] for name in names))
        db.executemany(sql, rows)
        db.execute("UPDATE accounts SET cards_count=? WHERE id=?",
                   (next_number + size - 4, templates[0]["account_id"]))
        marker = ("2020-01-01 00:00:00.000000" if target["name"] == "fizzy"
                  else "2020-01-01T00:00:00.000Z")
        db.execute("UPDATE cards SET updated_at=?,last_active_at=? WHERE board_id=?",
                   (marker, marker, board_id))
        assert db.execute("SELECT COUNT(*) FROM cards WHERE board_id=?",
                          (board_id,)).fetchone()[0] == size
        keep_stamp = db.execute("SELECT updated_at FROM columns WHERE id=?",
                                (keep_id,)).fetchone()[0]
    return {"path": path, "column": columns[0]["id"], "board_id": board_id,
            "delete_id": delete_id, "keep_id": keep_id,
            "keep_stamp": keep_stamp, "marker": marker, "size": size}


def measure(target, item):
    before = time.perf_counter()
    expect(target, item["path"] + f"/columns/{item['column']}", "DELETE", None, 204)
    elapsed_ms = (time.perf_counter() - before) * 1000
    with sqlite3.connect(target["database"]) as db:
        board_id = item["board_id"]
        size = item["size"]
        assert db.execute("SELECT COUNT(*) FROM cards WHERE board_id=?",
                          (board_id,)).fetchone()[0] == size
        assert db.execute("SELECT COUNT(*) FROM cards WHERE board_id=? AND updated_at=?",
                          (board_id, item["marker"])).fetchone()[0] == 0
        assert db.execute("SELECT COUNT(*) FROM cards WHERE board_id=? AND last_active_at=?",
                          (board_id, item["marker"])).fetchone()[0] == size
        assert db.execute("SELECT COUNT(*) FROM cards WHERE board_id=? AND column_id=?",
                          (board_id, item["delete_id"])).fetchone()[0] == 0
        assert db.execute("SELECT COUNT(*) FROM cards WHERE board_id=? AND column_id=?",
                          (board_id, item["keep_id"])).fetchone()[0] == 1 + (size - 2) // 3
        assert db.execute("SELECT updated_at FROM columns WHERE id=?",
                          (item["keep_id"],)).fetchone()[0] == item["keep_stamp"]
    return round(elapsed_ms, 3)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--sizes", nargs="+", type=int, default=[1000, 10000])
    parser.add_argument("--trials", type=int, default=3)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    assert all(size >= 3 for size in args.sizes)
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    cpus = sorted(os.sched_getaffinity(0))[:2]
    with tempfile.TemporaryDirectory(prefix="rizzy-column-delete-") as temporary:
        root = Path(temporary)
        targets = {}
        processes = []
        try:
            for name in ("fizzy", "rizzy"):
                directory = root / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                process, url = start_server(name, directory, env, cpus)
                processes.append(process)
                targets[name] = {"name": name, "url": url,
                                 "account": str(credentials["account"]),
                                 "token": credentials["token"],
                                 "database": directory / ("db.sqlite3" if name == "fizzy"
                                                          else "rizzy.sqlite3")}
            result = {"source_revision": "477c943e0506f109e5bc83ae9dadbe519732c045",
                      "cpus": cpus, "workers": {"fizzy": 2, "rizzy": 1},
                      "trials": args.trials, "sizes": {}}
            for size in args.sizes:
                timings = {"fizzy": [], "rizzy": []}
                for trial in range(args.trials):
                    fixtures = {name: fixture(targets[name], size, trial)
                                for name in ("fizzy", "rizzy")}
                    order = ("fizzy", "rizzy") if trial % 2 == 0 else ("rizzy", "fizzy")
                    for name in order:
                        timings[name].append(measure(targets[name], fixtures[name]))
                result["sizes"][str(size)] = timings
                print(f"{size} cards: {timings}", flush=True)
            if args.output:
                args.output.parent.mkdir(parents=True, exist_ok=True)
                args.output.write_text(json.dumps(result, indent=2) + "\n")
            print(json.dumps(result, indent=2))
        finally:
            for process in processes:
                stop_server(process)


if __name__ == "__main__":
    main()
