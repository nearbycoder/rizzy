#!/usr/bin/env python3
"""Compare the card self-assignment toggle on disposable cards."""
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
        f"{target['url']}/{target['account']}{path}", method=method, headers=headers,
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
    status, board = request(target, "/boards", "POST", {"board": {"name": f"Self assignment parity {time.time_ns()}"}})
    assert status == 201, (status, board)
    board_path = f"/boards/{board['id']}"
    try:
        status, card = request(target, board_path + "/cards", "POST", {"card": {"title": "Assignment parity"}})
        assert status == 201, (status, card)
        path = f"/cards/{card['number']}"
        result = []
        for expected in (1, 0):
            status, body = request(target, path + "/self_assignment.json", "POST")
            assert status == 204 and body == b"", (status, body)
            status, current = request(target, path)
            assert status == 200 and len(current["assignees"]) == expected, (status, current)
            result.append((status, len(current["assignees"])))
        status, _ = request(target, "/cards/999999999/self_assignment.json", "POST")
        assert status == 404, status
        return result, status
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
    print("Self-assignment matched: assign, unassign, and unknown card")


if __name__ == "__main__":
    main()
