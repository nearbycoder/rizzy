#!/usr/bin/env python3
"""Measure account ZIP builds on equal fresh Fizzy and Rizzy card fixtures."""
import argparse
import io
import json
import os
import statistics
import sys
import time
import urllib.request
import zipfile
from collections import Counter


def request(target, path, method="GET", payload=None):
    headers = {"Authorization": f"Bearer {target['token']}", "Accept": "application/json"}
    data = None
    if payload is not None:
        data = json.dumps(payload).encode()
        headers["Content-Type"] = "application/json"
    url = f"{target['url']}/{target['account']}{path}"
    with urllib.request.urlopen(urllib.request.Request(url, data=data, headers=headers, method=method), timeout=60) as response:
        raw = response.read()
        return response.status, json.loads(raw) if raw else None


def fixture(target, count):
    status, board = request(target, "/boards", "POST", {"board": {"name": f"Account export benchmark {int(time.time())}"}})
    assert status == 201
    for index in range(count):
        status, card = request(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": f"Export card {index}", "description": "The same formatted <strong>body</strong> on both servers."}})
        assert status == 201
        status, _ = request(target, f"/cards/{card['number']}/comments", "POST", {
            "comment": {"body": f"Export comment {index}"}})
        assert status == 201
        if (index + 1) % 100 == 0:
            print(f"{target['name']} fixture {index + 1}/{count}", file=sys.stderr, flush=True)


def export(target):
    start = time.perf_counter()
    status, created = request(target, "/account/exports", "POST")
    assert status == 201 and created["status"] == "pending", created
    for _ in range(2400):
        _, state = request(target, f"/account/exports/{created['id']}")
        if state["status"] in ("completed", "failed"):
            break
        time.sleep(.025)
    assert state["status"] == "completed", state
    ready_seconds = round(time.perf_counter() - start, 3)
    download_start = time.perf_counter()
    request_url = urllib.request.Request(state["download_url"], headers={
        "Authorization": f"Bearer {target['token']}", "Accept": "application/zip"})
    with urllib.request.urlopen(request_url, timeout=60) as response:
        assert response.status == 200 and response.headers.get_content_type() == "application/zip"
        archive_bytes = response.read()
    download_seconds = round(time.perf_counter() - download_start, 3)
    with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
        groups = Counter(name.split("/")[1] for name in archive.namelist() if name.startswith("data/"))
        files = sum(name.startswith("storage/") for name in archive.namelist())
        assert "data/account.json" in archive.namelist()
    return {"ready_seconds": ready_seconds, "download_seconds": download_seconds,
            "zip_bytes": len(archive_bytes), "entries": sum(groups.values()) + files,
            "storage_files": files, "groups": dict(groups)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=500)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--reuse-fixture", action="store_true")
    args = parser.parse_args()
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        targets[name.lower()] = {"name": name.lower(), "url": os.environ[f"{name}_URL"],
                                 "account": os.environ[f"{name}_ACCOUNT"],
                                 "token": os.environ[f"{name}_TOKEN"]}
    if not args.reuse_fixture:
        for target in targets.values():
            fixture(target, args.cards)
    results = {"fixture_cards": args.cards, "fixture_comments": args.cards,
               "reused_existing_fixture": args.reuse_fixture,
               "runs": {"fizzy": [], "rizzy": []}}
    for index in range(args.runs):
        names = ("fizzy", "rizzy") if index % 2 == 0 else ("rizzy", "fizzy")
        for name in names:
            run = export(targets[name])
            results["runs"][name].append(run)
            print(f"{name} run {index + 1}: ready={run['ready_seconds']}s entries={run['entries']}",
                  file=sys.stderr, flush=True)
    comparable = ("cards", "comments", "events", "watches", "action_text_rich_texts")
    results["comparable_record_counts"] = {
        group: {name: results["runs"][name][0]["groups"].get(group, 0) for name in targets}
        for group in comparable
    }
    results["record_count_parity"] = all(len(set(counts.values())) == 1 for counts in results["comparable_record_counts"].values())
    results["median_ready_seconds"] = {
        name: round(statistics.median(run["ready_seconds"] for run in runs), 3)
        for name, runs in results["runs"].items()
    }
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
