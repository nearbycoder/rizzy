#!/usr/bin/env python3
"""Compare named-column card JSON validators with pinned Fizzy."""

import json
import os
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, start_server,
                             stop_server)  # noqa: E402


def request(target, path, validator=None):
    headers = {"Authorization": f"Bearer {target['token']}", "Accept": "application/json"}
    if validator is not None:
        headers["If-None-Match"] = validator
    url = f"{target['url']}/{target['account']}{path}"
    try:
        response = urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=20)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        body = response.read()
        return response.status, response.headers, json.loads(body) if body else None


def run(target):
    name = target["name"]
    board, _ = expect(target, "/boards", "POST",
                      {"board": {"name": "Column validators"}}, 201)
    column, _ = expect(target, f"/boards/{board['id']}/columns", "POST",
                       {"column": {"name": "In progress"}}, 201)
    cards = []
    for index in range(16):
        card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                         {"card": {"title": f"Column card {index:02d}"}}, 201)
        expect(target, f"/cards/{card['number']}/triage", "POST",
               {"column_id": column["id"]}, 204)
        cards.append(card)
    path = f"/boards/{board['id']}/columns/{column['id']}/cards"
    status, headers, first = request(target, path)
    assert status == 200 and len(first) == 15, (name, status, len(first))
    etag = headers.get("ETag")
    assert etag and etag.startswith('W/"'), (name, etag)
    assert headers.get("Cache-Control") == "max-age=0, private, must-revalidate"
    assert headers.get("Content-Type") == "application/json; charset=utf-8"
    assert headers.get("X-Total-Count") == "16"
    assert 'rel="next"' in headers.get("Link", "")
    for conditional in (etag, f'W/"stale", {etag}', "*"):
        cached_status, cached_headers, cached_body = request(target, path, conditional)
        assert cached_status == 304 and cached_body is None, (name, conditional, cached_status)
        assert cached_headers.get("ETag") == etag
        assert cached_headers.get("X-Total-Count") == "16"
        assert cached_headers.get("Link") == headers.get("Link")
        assert cached_headers.get("Content-Type") is None
        assert cached_headers.get("Cache-Control") == headers.get("Cache-Control")
    assert request(target, path, etag[2:])[0] == 200
    second_path = path + "?page=2"
    second_status, second_headers, second = request(target, second_path)
    assert second_status == 200 and len(second) == 1
    second_etag = second_headers.get("ETag")
    assert second_etag and second_etag != etag
    assert request(target, second_path, second_etag)[0] == 304
    first_number = first[0]["number"]
    expect(target, f"/cards/{first_number}", "PATCH",
           {"card": {"title": "Changed first-page card"}}, 200)
    changed_status, changed_headers, changed = request(target, path, etag)
    assert changed_status == 200 and changed_headers.get("ETag") != etag
    assert changed[0]["title"] == "Changed first-page card"
    assert request(target, second_path, second_etag)[0] == 304
    new_first_etag = changed_headers["ETag"]
    expect(target, f"/cards/{second[0]['number']}", "PATCH",
           {"card": {"title": "Changed second-page card"}}, 200)
    second_changed, new_second_headers, new_second = request(target, second_path, second_etag)
    assert second_changed == 200 and new_second_headers.get("ETag") != second_etag
    assert new_second[0]["number"] != second[0]["number"], (name, new_second)
    moved_status, moved_headers, moved_first = request(target, path, new_first_etag)
    assert moved_status == 200 and moved_headers.get("ETag") != new_first_etag
    assert moved_first[0]["title"] == "Changed second-page card", (name, moved_first)
    return {"page_sizes": (len(first), len(second)), "conditional": 304,
            "changed": changed_status, "total": headers["X-Total-Count"]}


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    process, url = start_server(name, directory, env,
                                sorted(os.sched_getaffinity(0))[:2])
    try:
        target = {"name": name, "url": url, "account": str(credentials["account"]),
                  "token": credentials["token"]}
        return run(target)
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-column-etag-") as temporary:
        root = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = root / name
            directory.mkdir()
            results[name] = check(name, directory)
        assert results["fizzy"] == results["rizzy"], results
    print("Column card JSON validators matched pinned Fizzy")


if __name__ == "__main__":
    main()
