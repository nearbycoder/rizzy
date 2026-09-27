#!/usr/bin/env python3
"""Compare record field sets in fresh paired Fizzy and Rizzy account exports."""

import argparse
import io
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from sustained_mixed import SOURCE, prepare_rust, prepare_source, start_server, stop_server  # noqa: E402


def fixture(target):
    board, _ = expect(target, "/boards", "POST", {"board": {"name": "Export fields"}}, 201)
    card, _ = expect(target, f"/boards/{board['id']}/cards", "POST", {"card": {"title": "Record fields"}}, 201)
    number = card["number"]
    expect(target, f"/cards/{number}/comments", "POST", {"comment": {"body": "Field comparison"}}, 201)
    expect(target, f"/cards/{number}/steps", "POST", {"step": {"content": "One step"}}, 201)
    expect(target, f"/cards/{number}/taggings", "POST", {"tag_title": "#fields"}, 204)
    expect(target, f"/cards/{number}/pin", "POST", status=204)
    expect(target, f"/cards/{number}/reactions", "POST", {"reaction": {"content": "👍"}}, 201)
    return board, card


def export(target, env):
    created, _ = expect(target, "/account/exports.json", "POST", status=201)
    if target["name"] == "fizzy":
        script = f'Account::Export.find("{created["id"]}").build'
        subprocess.run(["bin/rails", "runner", script], cwd=SOURCE, env=env,
                       check=True, capture_output=True, text=True)
    for _ in range(300):
        state, _ = expect(target, f"/account/exports/{created['id']}.json")
        if state["status"] in ("completed", "failed"):
            break
        time.sleep(.1)
    assert state["status"] == "completed", (target["name"], state)
    request = urllib.request.Request(state["download_url"], headers={
        "Authorization": f"Bearer {target['token']}", "Accept": "application/zip"})
    with urllib.request.urlopen(request, timeout=45) as response:
        archive = response.read()
    with zipfile.ZipFile(io.BytesIO(archive)) as zipped:
        fields = {}
        counts = {}
        for name in zipped.namelist():
            if not name.startswith("data/") or not name.endswith(".json"):
                continue
            group = name.split("/")[1] if name != "data/account.json" else "account"
            fields.setdefault(group, set()).update(json.loads(zipped.read(name)))
            counts[group] = counts.get(group, 0) + 1
        return {group: sorted(keys) for group, keys in fields.items()}, counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--strict", action="store_true", help="fail when export fields or record counts differ")
    args = parser.parse_args()
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-account-field-inventory-") as temporary:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy" else prepare_rust(directory))
                process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url, "account": str(credentials["account"]),
                          "token": credentials["token"]}
                fixture(target)
                results[name] = export(target, env)
            fizzy, rust = results["fizzy"], results["rizzy"]
            differences = {}
            for group in sorted(set(fizzy[0]) | set(rust[0])):
                source_fields, rust_fields = set(fizzy[0].get(group, [])), set(rust[0].get(group, []))
                source_count, rust_count = fizzy[1].get(group, 0), rust[1].get(group, 0)
                if source_fields != rust_fields or source_count != rust_count:
                    differences[group] = {"fizzy_only": sorted(source_fields - rust_fields),
                                          "rizzy_only": sorted(rust_fields - source_fields),
                                          "counts": [source_count, rust_count]}
            print(json.dumps({"differences": differences, "counts": {
                name: result[1] for name, result in results.items()}}, indent=2))
            if args.strict and differences:
                raise SystemExit(1)
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
