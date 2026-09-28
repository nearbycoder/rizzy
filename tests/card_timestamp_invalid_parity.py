#!/usr/bin/env python3
"""Compare card date-string fallback and failed-update side effects with Fizzy."""

import datetime as dt
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from card_workflow import expect  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402


CASES = (("bad", "not-a-date"), ("empty", ""), ("numeric_text", "1"), ("null", None))
VALID_CASES = (
    ("date", "2024-01-15", "2024-01-15T00:00:00.000Z"),
    ("space", "2024-01-15 10:30:00", "2024-01-15T10:30:00.000Z"),
    ("local_iso", "2024-01-15T10:30:00", "2024-01-15T10:30:00.000Z"),
    ("slash", "2024/01/15 10:30:00", "2024-01-15T10:30:00.000Z"),
    ("utc_suffix", "2024-01-15 10:30:00 UTC", "2024-01-15T10:30:00.000Z"),
    ("month_name", "January 15, 2024 10:30", "2024-01-15T10:30:00.000Z"),
)


def request(target, path, method, payload):
    req = urllib.request.Request(
        f"{target['url']}/{target['account']}{path}",
        data=json.dumps(payload).encode(), method=method,
        headers={"Authorization": "Bearer " + target["token"],
                 "Accept": "application/json", "Content-Type": "application/json"},
    )
    try:
        response = urllib.request.urlopen(req, timeout=20)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, json.loads(response.read())


def utc(raw):
    return dt.datetime.fromisoformat(raw.replace("Z", "+00:00")).astimezone(dt.timezone.utc)


def check(name, directory):
    env, credentials = (prepare_source(directory, 2)
                        if name == "fizzy" else prepare_rust(directory))
    process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
    target = {"url": url, "account": str(credentials["account"]),
              "token": credentials["token"]}
    database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
    try:
        board, _ = expect(target, "/boards", "POST",
                          {"board": {"name": "Invalid card date parity"}}, 201)
        results = []
        for key in ("created_at", "last_active_at"):
            for label, value in CASES:
                before = dt.datetime.now(dt.timezone.utc)
                status, card = request(target, f"/boards/{board['id']}/cards", "POST",
                                       {"card": {"title": f"Date {key} {label}", key: value}})
                after = dt.datetime.now(dt.timezone.utc)
                assert status == 201, (name, key, label, status, card)
                created, active = utc(card["created_at"]), utc(card["last_active_at"])
                assert before - dt.timedelta(seconds=1) <= created <= after + dt.timedelta(seconds=1)
                assert created == active, (name, key, label, created, active)
                original, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                                     {"card": {"title": "Original"}}, 201)
                path = f"/cards/{original['number']}"
                with sqlite3.connect(database) as db:
                    count_before = db.execute("SELECT cards_count FROM accounts").fetchone()[0]
                status, body = request(target, path, "PATCH", {"card": {key: value}})
                assert (status, body) == (500, {"status": 500, "error": "Internal Server Error"}), (
                    name, key, label, status, body)
                stored, _ = expect(target, path)
                assert (stored["title"], stored["created_at"], stored["last_active_at"]) == (
                    original["title"], original["created_at"], original["last_active_at"]), (
                    name, key, label, stored)
                with sqlite3.connect(database) as db:
                    assert db.execute("SELECT cards_count FROM accounts").fetchone()[0] == count_before
                results.append((key, label, status, card["title"], stored["title"]))
        for label, value, expected in VALID_CASES:
            status, card = request(target, f"/boards/{board['id']}/cards", "POST",
                                   {"card": {"title": f"Parsed {label}", "created_at": value}})
            assert status == 201, (name, label, status, card)
            assert (card["created_at"], card["last_active_at"]) == (expected, expected), (
                name, label, card)
            results.append(("parsed", label, card["created_at"], card["last_active_at"]))
        return results
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-card-invalid-dates-") as temporary:
        runs = {}
        for name in ("fizzy", "rizzy"):
            directory = Path(temporary) / name
            directory.mkdir()
            runs[name] = check(name, directory)
    assert runs["fizzy"] == runs["rizzy"], runs
    print("Matched Fizzy on eight invalid card date cases and six accepted date formats")


if __name__ == "__main__":
    main()
