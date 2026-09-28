#!/usr/bin/env python3
"""Compare Fizzy's persisted card state when JSON date parameters are scalars."""

import datetime as dt
import os
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
sys.path.insert(0, str(ROOT / "tests"))

from card_workflow import expect  # noqa: E402
from card_timestamp_invalid_parity import request, utc  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402


CASES = (("one", 1, "1"), ("zero", 0, "0"), ("decimal", 1.5, "1.5"),
         ("true", True, "1"), ("false", False, "0"))
ERROR = (500, {"status": 500, "error": "Internal Server Error"})


def stored(database, title):
    with sqlite3.connect(database) as db:
        row = db.execute(
            "SELECT number,created_at,last_active_at,updated_at FROM cards WHERE title=?", (title,)
        ).fetchone()
        count = db.execute("SELECT cards_count FROM accounts").fetchone()[0]
    return row, count


def run(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
    target = {"url": url, "account": str(credentials["account"]),
              "token": credentials["token"]}
    database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
    try:
        board, _ = expect(target, "/boards", "POST", {"board": {"name": "Scalar card dates"}}, 201)
        observations = []
        for key in ("created_at", "last_active_at"):
            for label, value, cast in CASES:
                title = f"create-{key}-{label}"
                before = dt.datetime.now(dt.timezone.utc)
                count_before = stored(database, title)[1]
                result = request(target, f"/boards/{board['id']}/cards", "POST",
                                 {"card": {"title": title, key: value}})
                after = dt.datetime.now(dt.timezone.utc)
                row, count = stored(database, title)
                assert row and count == count_before + 1, (name, key, label, row, count)
                if label == "false":
                    assert result[0] == 201, (name, key, label, result)
                    created = utc(result[1]["created_at"])
                    assert before - dt.timedelta(seconds=1) <= created <= after + dt.timedelta(seconds=1)
                    assert result[1]["created_at"] == result[1]["last_active_at"]
                    observations.append((key, label, "create", 201, "fresh"))
                else:
                    assert result == ERROR, (name, key, label, result)
                    assert str(row[1 if key == "created_at" else 2]) == cast, (name, key, label, row)
                    if key == "created_at":
                        assert str(row[2]) == cast, (name, key, label, row)
                    observations.append((key, label, "create", 500, cast))

                original, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                                     {"card": {"title": f"update-{key}-{label}"}}, 201)
                old, count_before = stored(database, original["title"])
                time.sleep(.01)
                result = request(target, f"/cards/{original['number']}", "PATCH",
                                 {"card": {key: value}})
                row, count = stored(database, original["title"])
                assert result == ERROR and count == count_before, (name, key, label, result, count)
                assert str(row[1 if key == "created_at" else 2]) == cast, (name, key, label, row)
                assert row[2 if key == "created_at" else 1] == old[2 if key == "created_at" else 1]
                assert row[3] != old[3], (name, key, label, old, row)
                status, body = request(target, f"/cards/{original['number']}", "GET", None)
                assert (status, body) == ERROR, (name, key, label, status, body)
                observations.append((key, label, "update", 500, cast))
        status, body = request(target, f"/cards?board_ids%5B%5D={board['id']}", "GET", None)
        assert (status, body) == ERROR, (name, "card listing", status, body)
        observations.append(("listing", 500))
        return observations
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-card-scalar-dates-") as temporary:
        runs = {}
        for name in ("fizzy", "rizzy"):
            directory = Path(temporary) / name
            directory.mkdir()
            runs[name] = run(name, directory)
    assert runs["fizzy"] == runs["rizzy"], runs
    print("Matched Fizzy card date scalar create and update persistence across 20 cases")


if __name__ == "__main__":
    main()
