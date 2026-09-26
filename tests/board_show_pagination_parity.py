#!/usr/bin/env python3
"""Compare board pages, filtered JSON, and validators on disposable databases."""

import json
import os
import re
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from events_html import browser_cookie, html_response  # noqa: E402
from card_workflow import expect  # noqa: E402
from sustained_mixed import (fixture, prepare_rust, prepare_source,
                             start_server, stop_server)  # noqa: E402


def pages(target, board):
    target["cookie"] = browser_cookie(target)
    results = []
    for page in (1, 2):
        path = f"/boards/{board}?page={page}"
        status, headers, body = html_response(target, path)
        assert status == 200 and body, (target["name"], page, status)
        assert "X-Total-Count" not in headers, (target["name"], page, headers)
        titles = [f"Fixture card {number}".encode() for number in
                  (range(19, 4, -1) if page == 1 else range(4, -1, -1))]
        positions = [body.index(title) for title in titles]
        assert positions == sorted(positions), (target["name"], page, positions)
        assert len(re.findall(rb'id="article_card_[^"]+"', body)) == len(titles), (
            target["name"], page)
        etag = headers.get("ETag")
        assert etag, (target["name"], page)
        repeat_status, repeat_headers, repeat_body = html_response(target, path, etag)
        assert repeat_status == 304 and not repeat_body, (target["name"], page)
        assert "X-Total-Count" not in repeat_headers
        results.append((etag, len(titles)))
    assert results[0][0] != results[1][0], target["name"]
    return [count for _, count in results]


def board_json(target, board, suffix="", etag=None):
    headers = {"Authorization": f"Bearer {target['token']}",
               "Accept": "application/json"}
    if etag:
        headers["If-None-Match"] = etag
    request = urllib.request.Request(
        f"{target['url']}/{target['account']}/boards/{board}{suffix}", headers=headers)
    try:
        response = urllib.request.urlopen(request, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read()
        return response.status, response.headers, json.loads(raw) if raw else None


def json_pages(target, board, first_card):
    initial = []
    for suffix, total, has_next in (("", 20, True), ("?page=2", 20, False)):
        status, headers, body = board_json(target, board, suffix)
        assert status == 200 and body["name"] == "Mixed workload"
        assert int(headers["X-Total-Count"]) == total
        assert ("Link" in headers) == has_next
        assert headers.get("ETag")
        conditional_status, conditional_headers, conditional_body = board_json(
            target, board, suffix, headers["ETag"])
        assert conditional_status == 304 and conditional_body is None
        assert int(conditional_headers["X-Total-Count"]) == total
        initial.append(headers["ETag"])
    assert initial[0] != initial[1], target["name"]

    expect(target, f"/cards/{first_card}/closure", "POST", status=204)
    stale_status, _, stale_body = board_json(target, board, etag=initial[0])
    assert stale_status == 200 and stale_body is not None, target["name"]
    for suffix, total, has_next in (("", 19, True), ("?page=2", 19, False),
                                    ("?indexed_by=closed", 1, False),
                                    ("?indexed_by=closed&page=2", 1, True),
                                    ("?terms%5B%5D=Fixture", 19, True)):
        status, headers, body = board_json(target, board, suffix)
        assert status == 200 and body["name"] == "Mixed workload", (target["name"], suffix, status)
        assert int(headers["X-Total-Count"]) == total, (target["name"], suffix, headers)
        assert ("Link" in headers) == has_next, (target["name"], suffix, headers)
        assert headers.get("ETag"), (target["name"], suffix)
        conditional_status, conditional_headers, conditional_body = board_json(
            target, board, suffix, headers["ETag"])
        assert conditional_status == 304 and conditional_body is None, (
            target["name"], suffix, conditional_status)
        assert int(conditional_headers["X-Total-Count"]) == total

    _, before_headers, before_body = board_json(target, board)
    expect(target, "/account/settings", "PUT",
           {"account": {"name": "Renamed benchmark"}}, status=204)
    status, renamed_headers, renamed_body = board_json(target, board, etag=before_headers["ETag"])
    assert status == 200 and renamed_body == before_body
    assert renamed_headers["ETag"] != before_headers["ETag"], target["name"]
    expect(target, "/account/settings", "PUT",
           {"account": {"name": "Renamed benchmark"}}, status=204)
    repeated_status, repeated_headers, repeated_body = board_json(
        target, board, etag=renamed_headers["ETag"])
    assert repeated_status == 304 and repeated_body is None, target["name"]
    assert repeated_headers["ETag"] == renamed_headers["ETag"]


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-board-page-parity-") as temporary:
        root = Path(temporary)
        processes = []
        try:
            for name in ("fizzy", "rizzy"):
                directory = root / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                if name == "rizzy":
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url,
                          "account": str(credentials["account"]),
                          "token": credentials["token"],
                          "email": ("mixed-benchmark@example.com" if name == "fizzy"
                                    else "owner@example.com"),
                          "db": str(directory / ("db.sqlite3" if name == "fizzy"
                                                 else "rizzy.sqlite3"))}
                board, paths, _ = fixture(target, 20)
                assert pages(target, board) == [15, 5]
                first_card = int(paths[1].rsplit("/", 1)[1])
                json_pages(target, board, first_card)
                print(f"{name}: HTML pages and filtered JSON counts, links, and 304 responses match")
        finally:
            for process in processes:
                stop_server(process)


if __name__ == "__main__":
    main()
