#!/usr/bin/env python3
"""Compare Rails-style .json URLs across the two running applications."""
import json
import os
import time
import urllib.error
import urllib.request


def request(target, path, *, method="GET", payload=None, token=None, extra_headers=None):
    root = f"{target['url']}/{target['account']}"
    headers = {"Accept": "text/html", "Authorization": f"Bearer {token or target['token']}"}
    if payload is not None:
        headers["Content-Type"] = "application/json"
    if extra_headers:
        headers.update(extra_headers)
    req = urllib.request.Request(
        root + path,
        data=json.dumps(payload).encode() if payload is not None else None,
        headers=headers,
        method=method,
    )
    try:
        response = urllib.request.urlopen(req, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read()
        return response.status, response.headers, json.loads(raw) if raw and response.headers.get_content_type() == "application/json" else raw


def run(target):
    checks = {}
    paths = (
        "/boards.json",
        "/cards.json?page=1",
        "/users.json",
        "/tags.json",
        "/notifications.json",
        "/search.json?q=missing",
        "/account/settings.json",
        "/client_configurations/android_v1.json",
    )
    for path in paths:
        status, headers, body = request(target, path)
        assert status == 200, (path, status, body)
        assert headers.get_content_type() == "application/json", (path, headers)
        checks[path] = (status, type(body).__name__)
        if path.startswith("/cards.json"):
            assert "/cards.json?page=2" in headers.get("Link", ""), headers

    status, headers, _ = request(target, "/cards.json?page=1", extra_headers={"X-Rizzy-Original-Path": "/spoofed.json"})
    assert status == 200 and "/cards.json?page=2" in headers.get("Link", ""), headers
    checks["pagination_header_spoof"] = status

    _, _, boards = request(target, "/boards.json")
    board_id = boards[0]["id"]
    status, headers, board = request(target, f"/boards/{board_id}.json")
    assert status == 200 and headers.get_content_type() == "application/json" and board["id"] == board_id
    checks["board_show"] = status

    status, headers, body = request(target, "/boards.json", token="invalid-bearer-token")
    assert status == 401 and body == b"HTTP Token: Access denied.\n", (status, body)
    checks["invalid_token"] = status

    status, headers, board = request(target, "/boards.json", method="POST", payload={"board": {"name": f"JSON suffix parity {time.time_ns()}"}})
    assert status == 201 and headers.get_content_type() == "application/json" and board["id"]
    checks["board_create"] = status
    status, _, _ = request(target, f"/boards/{board['id']}.json", method="DELETE")
    assert status == 204, status
    checks["board_delete"] = status
    return checks


def main():
    targets = [
        {"url": os.environ[f"{name}_URL"], "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"]}
        for name in ("FIZZY", "RIZZY")
    ]
    source, rust = (run(target) for target in targets)
    assert source == rust, (source, rust)
    print(f"JSON suffix behavior matched on {len(source)} routes and operations")


if __name__ == "__main__":
    main()
