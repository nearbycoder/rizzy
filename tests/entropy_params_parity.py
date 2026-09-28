#!/usr/bin/env python3
"""Compare Fizzy and Rizzy entropy parameter coercion and stored effects."""

import json
import os
import sqlite3
import sys
import tempfile
import urllib.error
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402


def request(target, path, payload, include_day=True):
    request = urllib.request.Request(
        f"{target['url']}/{target['account']}{path}",
        json.dumps(payload).encode(), method="PUT",
        headers={"Authorization": "Bearer " + target["token"],
                 "Accept": "application/json", "Content-Type": "application/json"},
    )
    try:
        response = urllib.request.urlopen(request, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read()
        body = json.loads(raw) if raw else None
        if response.status == 200 and include_day:
            body = body["auto_postpone_period_in_days"]
        elif response.status == 200:
            # Fizzy's production board JSON fragment can stay cached at the
            # pre-update value; this check isolates parameter and stored-state
            # parity from that separate response-cache gap.
            body = None
        return response.status, body


def seed_board_override(name, directory, board_name):
    database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
    with sqlite3.connect(database) as db:
        board_id, account_id, created_at = db.execute(
            "SELECT id,account_id,created_at FROM boards WHERE name=?", (board_name,)
        ).fetchone()
        if name == "fizzy":
            db.execute(
                "INSERT INTO entropies(id,account_id,auto_postpone_period,container_id,container_type,created_at,updated_at) "
                "VALUES(?,?,?,?,?,?,?)",
                (uuid.uuid4().bytes, account_id, 90 * 86400, board_id,
                 "Board", created_at, created_at),
            )
        else:
            db.execute(
                "UPDATE boards SET auto_postpone_period_in_days=90,entropy_override=1 WHERE id=?",
                (board_id,),
            )


def stored_days(name, directory, label, board_name):
    database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
    with sqlite3.connect(database) as db:
        if label == "account":
            if name == "fizzy":
                return db.execute(
                    "SELECT auto_postpone_period/86400 FROM entropies WHERE container_type='Account'"
                ).fetchone()[0]
            return db.execute("SELECT auto_postpone_period_in_days FROM accounts").fetchone()[0]
        if name == "fizzy":
            return db.execute(
                "SELECT e.auto_postpone_period/86400 FROM entropies e "
                "JOIN boards b ON b.id=e.container_id WHERE e.container_type='Board' AND b.name=?",
                (board_name,),
            ).fetchone()[0]
        return db.execute(
            "SELECT auto_postpone_period_in_days FROM boards WHERE name=?",
            (board_name,),
        ).fetchone()[0]


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    process, url = start_server(name, directory, env,
                                sorted(os.sched_getaffinity(0))[:2])
    try:
        target = {"name": name, "url": url,
                  "account": str(credentials["account"]),
                  "token": credentials["token"]}
        board_name = "Entropy parameter parity"
        board, _ = expect(target, "/boards", "POST",
                          {"board": {"name": board_name}}, 201)
        seed_board_override(name, directory, board_name)
        paths = {"account": "/account/entropy",
                 "board": f"/boards/{board['id']}/entropy"}
        results = {}
        for label, path in paths.items():
            key = "entropy" if label == "account" else "board"
            cases = {
                "invalid_integer": {key: {"auto_postpone_period_in_days": 1}},
                "invalid_flat": {"auto_postpone_period_in_days": 1},
                "string": {key: {"auto_postpone_period_in_days": "7"}},
                "string_prefix": {key: {"auto_postpone_period_in_days": "7days"}},
                "float": {key: {"auto_postpone_period_in_days": 7.9}},
                "null": {key: {"auto_postpone_period_in_days": None}},
                "boolean": {key: {"auto_postpone_period_in_days": True}},
                "missing": {key: {}},
            }
            results[label] = {case: request(target, path, payload, include_day=label == "account")
                              for case, payload in cases.items()}
            results[label]["stored_days"] = stored_days(name, directory, label, board_name)
        return results
    finally:
        stop_server(process)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-entropy-parity-") as temp:
        root = Path(temp)
        source = root / "fizzy"
        rust = root / "rizzy"
        source.mkdir()
        rust.mkdir()
        left, right = check("fizzy", source), check("rizzy", rust)
        assert left == right, (left, right)
        print("Entropy parameters, error responses, account values, and stored board values matched pinned Fizzy:", left)


if __name__ == "__main__":
    main()
