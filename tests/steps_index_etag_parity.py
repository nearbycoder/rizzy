#!/usr/bin/env python3
"""Compare conditional step-list JSON responses with pinned Fizzy."""
import json
import os
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402


def get(target, path, validator=None):
    headers = {"Authorization": f"Bearer {target['token']}", "Accept": "application/json"}
    if validator is not None:
        headers["If-None-Match"] = validator
    url = f"{target['url']}/{target['account']}{path}"
    try:
        response = urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        body = response.read()
        return response.status, response.headers, json.loads(body) if body else None


def check(name, directory):
    env, credentials = (prepare_source(directory, 1) if name == "fizzy"
                        else prepare_rust(directory))
    process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
    try:
        target = {"url": url, "account": str(credentials["account"]),
                  "token": credentials["token"]}
        board, _ = expect(target, "/boards", "POST", {"board": {"name": "Step validators"}}, 201)
        try:
            card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                             {"card": {"title": "Step validator card"}}, 201)
            path = f"/cards/{card['number']}/steps"
            status, headers, empty = get(target, path)
            assert status == 200 and empty == [], (name, status, empty)
            initial = headers.get("ETag")
            assert initial and initial.startswith('W/"'), (name, initial)
            assert headers.get("Cache-Control") == "max-age=0, private, must-revalidate"
            assert headers.get_content_type() == "application/json"
            other, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                              {"card": {"title": "Another step list"}}, 201)
            other_path = f"/cards/{other['number']}/steps"
            other_etag = get(target, other_path)[1]["ETag"]
            assert other_etag != initial
            assert get(target, other_path, initial)[0] == 200
            for validator in (initial, f'W/"stale", {initial}', "*"):
                cached, cached_headers, body = get(target, path, validator)
                assert cached == 304 and body is None, (name, validator, cached)
                assert cached_headers.get("ETag") == initial
                assert cached_headers.get("Content-Type") is None
            assert get(target, path, initial[2:])[0] == 200

            step, _ = expect(target, path, "POST", {"step": {"content": "Draft plan"}}, 201)
            changed, headers, items = get(target, path, initial)
            assert changed == 200 and len(items) == 1 and items[0]["content"] == "Draft plan"
            created = headers["ETag"]
            assert created != initial
            assert get(target, path, created)[0] == 304
            expect(target, f"{path}/{step['id']}", "PUT", {"step": {"content": "Final plan"}}, 200)
            changed, headers, items = get(target, path, created)
            assert changed == 200 and items[0]["content"] == "Final plan"
            edited = headers["ETag"]
            assert edited != created
            expect(target, f"{path}/{step['id']}", "PUT", {"step": {"content": "Final plan"}}, 200)
            assert get(target, path, edited)[0] == 304
            assert get(target, other_path, other_etag)[0] == 304
            expect(target, f"{path}/{step['id']}", "DELETE", None, 204)
            changed, headers, items = get(target, path, edited)
            assert changed == 200 and items == [] and headers.get("ETag"), (name, changed, items)
            return {"empty": True, "conditional": 304, "cross_card": 200,
                    "create": True, "edit": True, "noop_edit": 304,
                    "delete": True, "content_type": "application/json"}
        finally:
            expect(target, f"/boards/{board['id']}", "DELETE", None, 204)
    finally:
        stop_server(process)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-steps-etag-") as temporary:
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = Path(temporary) / name
            directory.mkdir()
            results[name] = check(name, directory)
        assert results["fizzy"] == results["rizzy"], results
    print("Step index validators matched pinned Fizzy")


if __name__ == "__main__":
    main()
