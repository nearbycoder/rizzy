#!/usr/bin/env python3
"""Compare named-column JSON parameter coercion and error responses."""

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402
from card_workflow import expect  # noqa: E402
from activity_event_board_parity import database_id  # noqa: E402


def request(target, path, method, payload):
    req = urllib.request.Request(
        f"{target['url']}/{target['account']}{path}",
        json.dumps(payload).encode(),
        method=method,
        headers={"Authorization": "Bearer " + target["token"],
                 "Accept": "application/json", "Content-Type": "application/json"},
    )
    try:
        response = urllib.request.urlopen(req, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read()
        return response.status, json.loads(raw) if raw else None


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    process, url = start_server(name, directory, env,
                                sorted(os.sched_getaffinity(0))[:2])
    try:
        target = {"name": name, "url": url, "account": str(credentials["account"]),
                  "token": credentials["token"]}
        board, _ = expect(target, "/boards", "POST",
                          {"board": {"name": "Column parameters"}}, 201)
        path = f"/boards/{board['id']}/columns"
        database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")

        def stored(column):
            with sqlite3.connect(database) as db:
                return db.execute("SELECT name,color FROM columns WHERE id=?",
                                  (database_id(target, column["id"]),)).fetchone()

        result = {}
        for label, payload in [
            ("empty", {"column": {"name": ""}}),
            ("spaces", {"column": {"name": "   "}}),
            ("number", {"column": {"name": 0}}),
            ("missing", {"column": {}}),
            ("null_name", {"column": {"name": None}}),
            ("color_only", {"column": {"color": "var(--color-card-2)"}}),
            ("invalid_color", {"column": {"name": "Invalid color", "color": "#abcdef"}}),
            ("valid", {"column": {"name": "Valid", "color": "var(--color-card-4)"}}),
        ]:
            status, body = request(target, path, "POST", payload)
            result[label] = (status, {"name": body["name"], "color": body["color"],
                                      "stored": stored(body)} if status == 201 else body)
            if label == "valid":
                valid = body

        card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                         {"card": {"title": "Color touch"}}, 201)
        expect(target, f"/cards/{card['number']}/triage", "POST",
               {"column_id": valid["id"]}, 204)

        def times():
            with sqlite3.connect(database) as db:
                return db.execute(
                    "SELECT c.updated_at,c.last_active_at,b.updated_at,co.updated_at "
                    "FROM cards c JOIN boards b ON b.id=c.board_id "
                    "JOIN columns co ON co.id=c.column_id WHERE c.number=?",
                    (card["number"],)).fetchone()

        before = times()

        for label, payload in [
            ("null_color", {"column": {"color": None}}),
            ("empty_name", {"column": {"name": ""}}),
            ("number_name", {"column": {"name": 123}}),
            ("missing_update", {"column": {}}),
            ("null_name_update", {"column": {"name": None}}),
        ]:
            time.sleep(.05)
            status, body = request(target, path + "/" + valid["id"], "PATCH", payload)
            result[label] = (status, {"name": body["name"], "color": body["color"],
                                      "stored": stored(valid)} if status == 200 else body)
            after = times()
            result[label + "_touch"] = tuple(old != new for old, new in zip(before, after))
            before = after
        return result
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-column-params-") as temporary:
        root = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = root / name
            directory.mkdir()
            results[name] = check(name, directory)
        assert results["fizzy"] == results["rizzy"], results
    print("Column name, color, and parameter errors matched pinned Fizzy")


if __name__ == "__main__":
    main()
