#!/usr/bin/env python3
"""Compare chained and repeated board moves, events, and storage totals."""

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
import sustained_mixed as suite  # noqa: E402
from activity_event_board_parity import database_id  # noqa: E402
from card_image_parity import IMAGE  # noqa: E402
from storage_ledger_parity import api, upload  # noqa: E402


def check(target, database):
    names = ("Move source", "Move middle", "Move final")
    boards = [api(target, "/boards", "POST", {"board": {"name": name}}, 201)
              for name in names]
    card = api(target, f"/boards/{boards[0]['id']}/cards", "POST",
               {"card": {"title": "Move this twice"}}, 201)
    image = upload(target, IMAGE, "moon.jpg", "image/jpeg")
    api(target, f"/cards/{card['number']}.json", "PATCH",
        {"card": {"image": image["signed_id"]}})
    for board in (boards[1], boards[2], boards[2]):
        moved = api(target, f"/cards/{card['number']}/board", "PATCH",
                    {"board_id": board["id"]})
        assert moved["board"]["id"] == board["id"], (target["name"], moved)

    with sqlite3.connect(database) as db:
        card_id = database_id(target, card["id"])
        particulars = [json.loads(value) for (value,) in db.execute(
            "SELECT particulars FROM events WHERE eventable_type='Card' "
            "AND eventable_id=? AND action='card_board_changed' "
            "ORDER BY created_at,id", (card_id,))]
        particulars = [value.get("particulars", value) for value in particulars]
        amounts = [db.execute(
            "SELECT COALESCE(SUM(delta),0) FROM storage_entries WHERE board_id=?",
            (database_id(target, board["id"]),)).fetchone()[0]
            for board in boards]
    result = {"moves": [(value["old_board"], value["new_board"])
                        for value in particulars], "stored_bytes": amounts}
    assert result == {"moves": [(names[0], names[1]), (names[1], names[2])],
                      "stored_bytes": [0, 0, len(IMAGE)]}, (target["name"], result)
    return result


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    suite.RUST = ROOT / "target/debug/rizzy"
    with tempfile.TemporaryDirectory(prefix="rizzy-move-sequence-") as temporary:
        root = Path(temporary)
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = root / name
                directory.mkdir()
                env, credentials = (suite.prepare_source(directory, 2) if name == "fizzy"
                                    else suite.prepare_rust(directory))
                process, url = suite.start_server(name, directory, env,
                                                   sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url,
                          "account": str(credentials["account"]),
                          "token": credentials["token"]}
                database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
                results[name] = check(target, database)
            assert results["fizzy"] == results["rizzy"], results
        finally:
            for process in processes:
                suite.stop_server(process)
    print("Chained and repeated board moves matched pinned Fizzy")


if __name__ == "__main__":
    main()
