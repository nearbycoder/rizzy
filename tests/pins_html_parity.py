#!/usr/bin/env python3
"""Compare pinned-card browser frames, validators, and JSON with Fizzy."""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench"))

from card_image_parity import call
from draft_workflow_parity import login, request
from events_html_parity import target
from sustained_mixed import ROOT, prepare_rust, prepare_source, start_server, stop_server


def api(item, path, method="GET", payload=None):
    status, _, body = call(item, f"/{item['account']}{path}", method, payload)
    assert status in (200, 201, 204), (item["name"], path, status, body[:160])
    return json.loads(body) if body else None


def check(item):
    board = api(item, "/boards", "POST", {"board": {"name": "Pin browser parity"}})
    cards = [api(item, f"/boards/{board['id']}/cards", "POST",
                 {"card": {"title": f"Pin browser card {index}"}}) for index in (1, 2)]
    for card in cards:
        api(item, f"/cards/{card['number']}/pin", "POST")
    login(item)
    path = f"/{item['account']}/my/pins"
    status, headers, body = request(item, path, accept="text/html")
    assert status == 200 and b'<turbo-frame id="pins">' in body, (item["name"], status)
    assert body.count(b'class="tray__item tray__item--pin"') == 2, item["name"]
    assert body.count(b'id="article_card_') == 2, item["name"]
    assert body.index(b"Pin browser card 2") < body.index(b"Pin browser card 1"), item["name"]
    assert b"Unpin this card" in body, item["name"]
    etag = headers.get("ETag")
    assert etag, item["name"]
    item["extra_headers"] = {**item.get("extra_headers", {}), "If-None-Match": etag}
    status, _, unchanged = request(item, path, accept="text/html")
    assert status == 304 and not unchanged, (item["name"], status)
    api(item, f"/cards/{cards[1]['number']}", "PATCH",
        {"card": {"title": "Renamed pinned card"}})
    status, changed_headers, changed = request(item, path, accept="text/html")
    assert status == 200 and b"Renamed pinned card" in changed, (item["name"], status)
    assert changed_headers.get("ETag") != etag, item["name"]
    item["extra_headers"].pop("If-None-Match")
    status, _, raw = request(item, path, accept="application/json")
    assert status == 200, (item["name"], status)
    values = json.loads(raw)
    assert len(values) == 2, (item["name"], values)
    assert [card["title"] for card in values] == ["Renamed pinned card", "Pin browser card 1"], (
        item["name"], values)
    api(item, f"/cards/{cards[1]['number']}/pin", "DELETE")
    status, _, body = request(item, path, accept="text/html")
    assert status == 200 and body.count(b'class="tray__item tray__item--pin"') == 1, (
        item["name"], status)
    return {"initial": 2, "after_unpin": 1, "conditional": 304,
            "rename_invalidation": 200, "json_count": len(values)}


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-pins-html-") as temp:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temp) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                if name == "rizzy":
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                item = target(name, url, str(credentials["account"]), credentials["token"],
                              "owner@example.com", str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")))
                results[name] = check(item)
            assert results["fizzy"] == results["rizzy"], results
            print("PASS pin HTML, ETag, JSON, and unpin parity", results["rizzy"])
        finally:
            for process in processes:
                stop_server(process)


if __name__ == "__main__":
    main()
