#!/usr/bin/env python3
"""Compare source and Rust tag-title sanitization and stored titles."""

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402


CASES = (None, "", " ", "##double", "# spaced", "#", " # ", "##", "# #", " A B ", "  #  blank")


def send(target, path, title):
    request = urllib.request.Request(
        target["url"] + f"/{target['account']}{path}",
        data=json.dumps({} if title is None else {"tag_title": title}).encode(), method="POST",
        headers={"Authorization": f"Bearer {target['token']}",
                 "Accept": "application/json", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.status
    except urllib.error.HTTPError as error:
        error.read()
        return error.code


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy" else prepare_rust(directory))
    process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
    target = {"name": name, "url": url, "account": str(credentials["account"]),
              "token": credentials["token"]}
    try:
        board, _ = expect(target, "/boards", "POST", {"board": {"name": "Tag edge probe"}}, 201)
        card, _ = expect(target, f"/boards/{board['id']}/cards", "POST", {"card": {"title": "Tag me"}}, 201)
        database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
        results = []
        for title in CASES:
            status = send(target, f"/cards/{card['number']}/taggings", title)
            with sqlite3.connect(database) as db:
                tags = [row[0] for row in db.execute("SELECT title FROM tags WHERE account_id=(SELECT account_id FROM cards WHERE number=?) ORDER BY title", (card["number"],))]
            results.append({"input": title, "status": status, "tags": tags})
        return results
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-tag-title-edge-") as temporary:
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = Path(temporary) / name
            directory.mkdir()
            results[name] = check(name, directory)
        assert results["fizzy"] == results["rizzy"], results
        print(f"Tag title status and stored-title parity passed for {len(CASES)} inputs")


if __name__ == "__main__":
    main()
