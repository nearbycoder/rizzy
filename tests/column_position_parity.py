#!/usr/bin/env python3
"""Compare column position actions with a disposable board on each server."""
import json
import os
import time
import urllib.error
import urllib.request


def request(target, path, method="GET", payload=None):
    headers = {"Authorization": f"Bearer {target['token']}", "Accept": "application/json"}
    if payload is not None:
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(
        f"{target['url']}/{target['account']}{path}",
        method=method,
        headers=headers,
        data=json.dumps(payload).encode() if payload is not None else None,
    )
    try:
        response = urllib.request.urlopen(req, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read()
        return response.status, json.loads(raw) if raw and response.headers.get_content_type() == "application/json" else raw


def run(target):
    status, board = request(target, "/boards", "POST", {"board": {"name": f"Column position parity {time.time_ns()}"}})
    assert status == 201, (status, board)
    board_path = f"/boards/{board['id']}"
    try:
        columns = []
        for name in ("Alpha", "Beta", "Gamma"):
            status, column = request(target, board_path + "/columns", "POST", {"column": {"name": name}})
            assert status == 201, (status, column)
            columns.append(column)

        def order():
            status, values = request(target, board_path + "/columns")
            assert status == 200, (status, values)
            return [value["name"] for value in values]

        outcome = [order()]
        for index, direction in ((1, "left"), (1, "left"), (1, "right"), (2, "right")):
            status, body = request(target, f"/columns/{columns[index]['id']}/{direction}_position.json", "POST")
            assert status == 201 and body == b"", (status, body)
            outcome.append(order())
        assert outcome == [
            ["Alpha", "Beta", "Gamma"],
            ["Beta", "Alpha", "Gamma"],
            ["Beta", "Alpha", "Gamma"],
            ["Alpha", "Beta", "Gamma"],
            ["Alpha", "Beta", "Gamma"],
        ], outcome
        status, _ = request(target, "/columns/nonexistent/left_position.json", "POST")
        assert status == 404, status
        return outcome, status
    finally:
        status, _ = request(target, board_path, "DELETE")
        assert status == 204, status


def main():
    targets = [
        {"url": os.environ[f"{name}_URL"], "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"]}
        for name in ("FIZZY", "RIZZY")
    ]
    source, rust = (run(target) for target in targets)
    assert source == rust, (source, rust)
    print("Column position actions matched: left, right, boundaries, and unknown ID")


if __name__ == "__main__":
    main()
