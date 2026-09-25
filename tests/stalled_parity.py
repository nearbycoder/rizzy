#!/usr/bin/env python3
"""Compare Fizzy and Rizzy stalled results after an old activity spike."""
import json
import os
import sqlite3
import time
import urllib.parse
from datetime import datetime, timedelta, timezone
from pathlib import Path

from parity import request
from saved_filter_parity import ROOT, rails_runner


def checked(target, path, method="GET", payload=None, expected=200):
    status, value = request(target, path, method, payload)
    assert status == expected, (path, status, value)
    return value


def seed_spike(name, board_name, card_title, card_id, old, db_path):
    if name == "fizzy":
        script = '''board = Board.find_by!(name: ENV.fetch("PARITY_BOARD_NAME"))
          card = board.cards.find_by!(title: ENV.fetch("PARITY_CARD_TITLE"))
          spike = Card::ActivitySpike.find_or_create_by!(card: card)
          spike.update_columns(updated_at: Time.iso8601(ENV.fetch("PARITY_OLD_TIME")))
          card.update_columns(updated_at: Time.iso8601(ENV.fetch("PARITY_OLD_TIME")))'''
        rails_runner(script, PARITY_BOARD_NAME=board_name, PARITY_CARD_TITLE=card_title, PARITY_OLD_TIME=old)
    else:
        with sqlite3.connect(db_path) as db:
            account_id = db.execute("SELECT account_id FROM cards WHERE id=?", (card_id,)).fetchone()[0]
            db.execute("INSERT INTO card_activity_spikes(card_id,account_id,created_at,updated_at) VALUES(?,?,?,?) ON CONFLICT(card_id) DO UPDATE SET updated_at=excluded.updated_at",
                       (card_id, account_id, old, old))
            db.execute("UPDATE cards SET updated_at=? WHERE id=?", (old, card_id))


def main():
    targets = {name.lower(): {key.lower(): os.environ[f"{name}_{key}"] for key in ("URL", "ACCOUNT", "TOKEN")}
               for name in ("FIZZY", "RIZZY")}
    rust_db = Path(os.getenv("RIZZY_DATABASE", ROOT / "data/rizzy.sqlite3"))
    output = {}
    old = (datetime.now(timezone.utc) - timedelta(days=15)).isoformat().replace("+00:00", "Z")
    for name, target in targets.items():
        board_name = "Stalled parity " + str(time.time_ns())
        title = "Stalled candidate " + str(time.time_ns())
        board = checked(target, "/boards", "POST", {"board": {"name": board_name}}, 201)
        try:
            card = checked(target, f"/boards/{board['id']}/cards", "POST", {"card": {"title": title}}, 201)
            path = "/cards?" + urllib.parse.urlencode({"board_ids[]": [board["id"]], "indexed_by": "stalled"}, doseq=True)
            output[name] = {}
            output[name]["before"] = len(checked(target, path))
            seed_spike(name, board_name, title, card["id"], old, rust_db)
            output[name]["stalled"] = len(checked(target, path))
            checked(target, f"/cards/{card['number']}/comments", "POST", {"comment": {"body": "Recent activity"}}, 201)
            output[name]["after_comment"] = len(checked(target, path))
            checked(target, f"/cards/{card['number']}/not_now", "POST", expected=204)
            output[name]["postponed"] = len(checked(target, path))
        finally:
            checked(target, f"/boards/{board['id']}", "DELETE", expected=204)
    assert output["fizzy"] == output["rizzy"] == {"before": 0, "stalled": 1, "after_comment": 0, "postponed": 0}, output
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
