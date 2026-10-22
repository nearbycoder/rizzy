#!/usr/bin/env python3
"""Compare named-column index and show conditional responses with Fizzy."""

import http.cookiejar
import os
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, start_server,
                             stop_server)  # noqa: E402
from column_cards_etag_parity import request  # noqa: E402
from board_special_column_etag_parity import html_request  # noqa: E402
from draft_workflow_parity import NoRedirect, login  # noqa: E402


def run(target, signed_in=False):
    name = target["name"]
    if not signed_in:
        login(target)
    board, _ = expect(target, "/boards", "POST",
                      {"board": {"name": "Named column response"}}, 201)
    column, _ = expect(target, f"/boards/{board['id']}/columns", "POST",
                       {"column": {"name": "In progress"}}, 201)
    index_path = f"/boards/{board['id']}/columns"
    show_path = index_path + f"/{column['id']}"
    index_status, index_headers, columns = request(target, index_path)
    assert index_status == 200 and [item["id"] for item in columns] == [column["id"]]
    index_etag = index_headers.get("ETag")
    assert index_etag and index_etag.startswith('W/"'), (name, index_etag)
    assert request(target, index_path, index_etag)[0] == 304
    show_status, show_headers, shown = request(target, show_path)
    assert show_status == 200 and shown["id"] == column["id"]
    show_etag = show_headers.get("ETag")
    assert show_etag and show_etag.startswith('W/"'), (name, show_etag)
    assert show_headers.get("X-Total-Count") == "0", (name, show_headers)
    assert request(target, show_path, show_etag)[0] == 304
    html_status, html_headers, html_body = html_request(target, show_path)
    assert html_status == 200 and b"Column: In progress" in html_body, (name, html_status)
    html_etag = html_headers.get("ETag")
    assert html_etag and html_request(target, show_path, html_etag)[0] == 304
    index_html_status = html_request(target, index_path)[0]
    card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                     {"card": {"title": "Named column card"}}, 201)
    expect(target, f"/cards/{card['number']}/triage", "POST",
           {"column_id": column["id"]}, 204)
    changed_status, changed_headers, changed = request(target, show_path, show_etag)
    assert changed_status == 200 and changed["id"] == column["id"]
    assert changed_headers.get("ETag") != show_etag
    assert changed_headers.get("X-Total-Count") == "1"
    changed_index_status, changed_index_headers, _ = request(target, index_path, index_etag)
    assert changed_index_status == 200 and changed_index_headers.get("ETag") != index_etag
    expect(target, show_path, "PATCH", {"column": {"name": "Renamed"}}, 200)
    assert request(target, index_path, changed_index_headers["ETag"])[0] == 200
    return {"index_html": index_html_status, "show_html": html_status,
            "show_total": (show_headers["X-Total-Count"], changed_headers["X-Total-Count"])}


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    if name == "rizzy":
        env["RIZZY_DEV_MAGIC_LINKS"] = "1"
    process, url = start_server(name, directory, env,
                                sorted(os.sched_getaffinity(0))[:2])
    try:
        target = {"name": name, "url": url, "account": str(credentials["account"]),
                  "token": credentials["token"], "email": "owner@example.com",
                  "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                  "opener": urllib.request.build_opener(
                      urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
        return run(target)
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-named-column-show-") as temporary:
        root = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = root / name
            directory.mkdir()
            results[name] = check(name, directory)
        assert results["fizzy"] == results["rizzy"], results
    print("Named-column index and show validators matched pinned Fizzy")


if __name__ == "__main__":
    main()
