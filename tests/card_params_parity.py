#!/usr/bin/env python3
"""Compare JSON card parameter acceptance, scalar casting, and write effects."""

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


CASES = (
    ("missing", {}),
    ("null_card", {"card": None}),
    ("array_card", {"card": []}),
    ("string_card", {"card": "text"}),
    ("unknown_flat", {"unknown": 1}),
    ("empty_card", {"card": {}}),
    ("unknown_card", {"card": {"unknown": 1}}),
    ("flat_title", {"title": "Flat title"}),
    ("flat_description", {"description": "Flat body"}),
    ("null_fields", {"card": {"title": None, "description": None}}),
    ("number_title", {"card": {"title": 12}}),
    ("false_title", {"card": {"title": False}}),
    ("true_title", {"card": {"title": True}}),
    ("float_title", {"card": {"title": 1.5}}),
    ("array_title", {"card": {"title": ["x"]}}),
    ("array_title_with_description", {"card": {"title": ["x"], "description": "Body"}}),
    ("object_title_with_description", {"card": {"title": {"x": 1}, "description": "Body"}}),
    ("number_description", {"card": {"description": 12}}),
    ("true_description", {"card": {"description": True}}),
    ("array_description", {"card": {"description": ["x"]}}),
    ("array_description_with_title", {"card": {"title": "Kept", "description": ["x"]}}),
    ("object_description", {"card": {"description": {"x": 1}}}),
    ("object_description_with_title", {"card": {"title": "Kept", "description": {"x": 1}}}),
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


def result(status, body):
    if status in (200, 201):
        return status, {key: body[key] for key in ("number", "title", "description")}
    return status, body


def check(name, directory):
    env, credentials = (prepare_source(directory, 2)
                        if name == "fizzy" else prepare_rust(directory))
    process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
    target = {"url": url, "account": str(credentials["account"]),
              "token": credentials["token"]}
    database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
    try:
        board, _ = expect(target, "/boards", "POST",
                          {"board": {"name": "Card parameter parity"}}, 201)
        created = []
        for label, payload in CASES:
            status, body = request(target, f"/boards/{board['id']}/cards", "POST", payload)
            with sqlite3.connect(database) as db:
                cards_count = db.execute("SELECT cards_count FROM accounts").fetchone()[0]
            created.append((label, result(status, body), cards_count))
        sentinel, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                             {"card": {"title": "After parameter cases"}}, 201)
        updated = []
        for label, payload in CASES:
            card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                             {"card": {"title": "Original", "description": "Original body"}}, 201)
            path = f"/cards/{card['number']}"
            status, body = request(target, path, "PATCH", payload)
            fetched, _ = expect(target, path)
            updated.append((label, result(status, body),
                            (fetched["title"], fetched["description"])))
        return created, sentinel["number"], updated
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-card-params-") as temporary:
        runs = {}
        for name in ("fizzy", "rizzy"):
            directory = Path(temporary) / name
            directory.mkdir()
            runs[name] = check(name, directory)
    for index, case in enumerate(CASES):
        source = runs["fizzy"][0][index]
        rust = runs["rizzy"][0][index]
        assert source == rust, ("create", case[0], source, rust)
    assert runs["fizzy"][1] == runs["rizzy"][1], (
        "next card number after parameter cases", runs["fizzy"][1], runs["rizzy"][1])
    for index, case in enumerate(CASES):
        source = runs["fizzy"][2][index]
        rust = runs["rizzy"][2][index]
        assert source == rust, ("update", case[0], source, rust)
    print(f"Matched Fizzy on {len(CASES)} card-create and {len(CASES)} card-update parameter cases")


if __name__ == "__main__":
    main()
