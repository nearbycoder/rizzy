#!/usr/bin/env python3
"""Compare triage behavior after a card crosses boards with pinned Fizzy."""

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
from activity_event_board_parity import database_id  # noqa: E402


def status(target, path, method, payload=None):
    body = json.dumps(payload).encode() if payload is not None else None
    request = urllib.request.Request(
        f"{target['url']}/{target['account']}{path}", body, method=method,
        headers={"Authorization": "Bearer " + target["token"],
                 "Accept": "application/json", "Content-Type": "application/json"},
    )
    try:
        response = urllib.request.urlopen(request, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        response.read()
        return response.status


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    process, url = start_server(name, directory, env,
                                sorted(os.sched_getaffinity(0))[:2])
    try:
        target = {"name": name, "url": url, "account": str(credentials["account"]),
                  "token": credentials["token"]}
        first, _ = expect(target, "/boards", "POST",
                          {"board": {"name": "First"}}, 201)
        second, _ = expect(target, "/boards", "POST",
                           {"board": {"name": "Second"}}, 201)
        first_column, _ = expect(target, f"/boards/{first['id']}/columns", "POST",
                                 {"column": {"name": "First column"}}, 201)
        second_column, _ = expect(target, f"/boards/{second['id']}/columns", "POST",
                                  {"column": {"name": "Second column"}}, 201)
        card, _ = expect(target, f"/boards/{first['id']}/cards", "POST",
                         {"card": {"title": "Transfer triage"}}, 201)
        path = f"/cards/{card['number']}"
        database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")

        def location():
            with sqlite3.connect(database) as db:
                board_id, column_id = db.execute(
                    "SELECT board_id,column_id FROM cards WHERE number=?",
                    (card["number"],)).fetchone()
            for label, board, column in (
                ("first", first, first_column), ("second", second, second_column)):
                if board_id == database_id(target, board["id"]):
                    return label, column_id == database_id(target, column["id"]), column_id is None
            raise AssertionError((name, board_id))

        outcomes = []
        outcomes.append(("triage_first", status(target, path + "/triage", "POST",
                                                 {"column_id": first_column["id"]}), location()))
        outcomes.append(("move_second", status(target, path + "/board", "PATCH",
                                                {"board_id": second["id"]}), location()))
        outcomes.append(("stale_column", status(target, path + "/triage", "POST",
                                                {"column_id": first_column["id"]}), location()))
        outcomes.append(("triage_second", status(target, path + "/triage", "POST",
                                                  {"column_id": second_column["id"]}), location()))
        outcomes.append(("untriage", status(target, path + "/triage", "DELETE"), location()))
        return outcomes
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-triage-board-") as temporary:
        root = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = root / name
            directory.mkdir()
            results[name] = check(name, directory)
        assert results["fizzy"] == results["rizzy"], results
        assert results["rizzy"] == [
            ("triage_first", 204, ("first", True, False)),
            ("move_second", 200, ("second", False, True)),
            ("stale_column", 404, ("second", False, True)),
            ("triage_second", 204, ("second", True, False)),
            ("untriage", 204, ("second", False, True)),
        ], results
    print("Triage across boards matched pinned Fizzy")


if __name__ == "__main__":
    main()
