#!/usr/bin/env python3
"""Compare Fizzy's search-indexing response header across route types."""

import json
import os
import time
import urllib.error
import urllib.request


def request(target, path, method="GET", payload=None, authenticated=False):
    headers = {"Accept": "application/json" if authenticated else "text/html"}
    if authenticated:
        headers["Authorization"] = "Bearer " + target["token"]
    if payload is not None:
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(
        target["url"] + path,
        data=json.dumps(payload).encode() if payload is not None else None,
        headers=headers,
        method=method,
    )
    try:
        response = urllib.request.urlopen(req, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        body = response.read()
        return response.status, response.headers.get("X-Robots-Tag"), (
            json.loads(body) if authenticated and body else body
        )


def run(target):
    outcome = {}
    for name, path, expected in (
        ("sign_in", "/session/new", "none"),
        ("health", "/up", None),
        ("error_css", "/error.css", None),
        ("service_worker", "/service-worker", "none"),
        ("client_config", "/client_configurations/android_v1.json", "none"),
    ):
        status, robots, _ = request(target, path)
        assert status == 200 and robots == expected, (target["name"], name, status, robots)
        outcome[name] = (status, robots)

    root = "/" + target["account"]
    status, robots, _ = request(target, root + "/boards", authenticated=True)
    assert (status, robots) == (200, "none"), (target["name"], "boards", status, robots)
    outcome["boards"] = (status, robots)

    status, robots, board = request(
        target, root + "/boards", "POST",
        {"board": {"name": f"Indexing parity {time.time_ns()}"}}, True,
    )
    assert (status, robots) == (201, "none"), (target["name"], "create", status, robots)
    board_id = board["id"]
    try:
        status, robots, published = request(
            target, root + f"/boards/{board_id}/publication", "POST", {}, True,
        )
        assert (status, robots) == (201, "none"), (target["name"], "publish", status, robots)
        key = published["public_url"].split("/public/boards/")[1]
        status, robots, _ = request(target, root + f"/public/boards/{key}")
        assert (status, robots) == (200, "none"), (target["name"], "public_board", status, robots)
        outcome["public_board"] = (status, robots)
    finally:
        status, _, _ = request(target, root + f"/boards/{board_id}", "DELETE", authenticated=True)
        assert status == 204, (target["name"], "cleanup", status)
    return outcome


def main():
    targets = [
        {"name": name.lower(), "url": os.environ[f"{name}_URL"].rstrip("/"),
         "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"]}
        for name in ("FIZZY", "RIZZY")
    ]
    source, rust = (run(target) for target in targets)
    assert source == rust, (source, rust)
    print(f"Search-indexing headers matched on {len(source)} application and static route types")


if __name__ == "__main__":
    main()
