#!/usr/bin/env python3
"""Compare signed-in board pages and validators on disposable databases."""

import os
import re
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from events_html import browser_cookie, html_response  # noqa: E402
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
                board, _, _ = fixture(target, 20)
                assert pages(target, board) == [15, 5]
                print(f"{name}: 15 and 5 cards, page-specific 304, no HTML total header")
        finally:
            for process in processes:
                stop_server(process)


if __name__ == "__main__":
    main()
