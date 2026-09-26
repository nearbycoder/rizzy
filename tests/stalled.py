#!/usr/bin/env python3
"""Verify activity-spike detection and the stalled card index."""
import hashlib
import os
import re
import sqlite3
import subprocess
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from join import call


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "stalled.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        seed = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seed.stdout).group(1)
        token = re.search(r"Bearer token: (\w+)", seed.stdout).group(1)
        port, server = start(env)
        try:
            board, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Spike detector"}}, bearer=token, status=201)
            owner = board["creator"]["id"]

            def card(title):
                value, _ = call(port, f"/{account}/boards/{board['id']}/cards", "POST", {"card": {"title": title}}, bearer=token, status=201)
                return value

            def stalled_titles():
                values, _ = call(port, f"/{account}/cards?board_ids%5B%5D={board['id']}&indexed_by=stalled", bearer=token)
                return {value["title"] for value in values}

            first = card("Stall then resume")
            assert stalled_titles() == set()
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM card_activity_spikes WHERE card_id=?", (first["id"],)).fetchone()[0] == 0
            call(port, f"/{account}/cards/{first['number']}/assignments", "POST", {"assignee_id": owner}, bearer=token, status=204)
            with sqlite3.connect(database) as db:
                first_spike = db.execute("SELECT id FROM card_activity_spikes WHERE card_id=?", (first["id"],)).fetchone()
                assert first_spike and first_spike[0]
            with sqlite3.connect(database) as db:
                old = (datetime.now(timezone.utc) - timedelta(days=15)).isoformat(timespec="milliseconds").replace("+00:00", "Z")
                db.execute("UPDATE card_activity_spikes SET updated_at=? WHERE card_id=?", (old, first["id"]))
                db.execute("UPDATE cards SET updated_at=? WHERE id=?", (old, first["id"]))
            assert stalled_titles() == {"Stall then resume"}
            call(port, f"/{account}/cards/{first['number']}/comments", "POST", {"comment": {"body": "Fresh activity"}}, bearer=token, status=201)
            assert stalled_titles() == set()
            with sqlite3.connect(database) as db:
                db.execute("UPDATE cards SET updated_at=? WHERE id=?", (old, first["id"]))
            assert stalled_titles() == {"Stall then resume"}
            call(port, f"/{account}/cards/{first['number']}/not_now", "POST", bearer=token, status=204)
            assert stalled_titles() == set()
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM card_activity_spikes WHERE card_id=?", (first["id"],)).fetchone()[0] == 0
            call(port, f"/{account}/cards/{first['number']}/not_now", "DELETE", bearer=token, status=204)
            call(port, f"/{account}/cards/{first['number']}/closure", "POST", bearer=token, status=204)
            call(port, f"/{account}/cards/{first['number']}/closure", "DELETE", bearer=token, status=204)
            with sqlite3.connect(database) as db:
                replacement = db.execute("SELECT id FROM card_activity_spikes WHERE card_id=?", (first["id"],)).fetchone()
                assert replacement and replacement[0] != first_spike[0]

            with sqlite3.connect(database) as db:
                account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?", (account,)).fetchone()[0]
                db.execute("INSERT INTO identities(id,email_address,created_at) VALUES('spike-identity','spike@example.com','2026-01-01T00:00:00Z')")
                db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('spike-user',?,'spike-identity','Spike Member','member','2026-01-01T00:00:00Z')", (account_id,))
                db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES('spike-token','spike-identity',?,'Test','write','2026-01-01T00:00:00Z')", (hashlib.sha256(b'spike-token').hexdigest(),))
                db.execute("INSERT INTO accesses(board_id,user_id) VALUES(?,'spike-user')", (board["id"],))
            second = card("Multi-person comments")
            for bearer, message in ((token, "First"), ("spike-token", "Second"), (token, "Third")):
                call(port, f"/{account}/cards/{second['number']}/comments", "POST", {"comment": {"body": message}}, bearer=bearer, status=201)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM card_activity_spikes WHERE card_id=?", (second["id"],)).fetchone()[0] == 1
            third = card("One-person comments")
            for index in range(3):
                call(port, f"/{account}/cards/{third['number']}/comments", "POST", {"comment": {"body": f"Comment {index}"}}, bearer=token, status=201)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM card_activity_spikes WHERE card_id=?", (third["id"],)).fetchone()[0] == 0
            repeat = card("Repeated assignment spike")
            call(port, f"/{account}/cards/{repeat['number']}/assignments", "POST",
                 {"assignee_id": owner}, bearer=token, status=204)
            with sqlite3.connect(database) as db:
                repeat_spike = db.execute("SELECT id FROM card_activity_spikes WHERE card_id=?",
                                          (repeat["id"],)).fetchone()
            assert repeat_spike
            for _ in range(2):
                call(port, f"/{account}/cards/{repeat['number']}/assignments", "POST",
                     {"assignee_id": owner}, bearer=token, status=204)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT id FROM card_activity_spikes WHERE card_id=?",
                                  (repeat["id"],)).fetchone() == repeat_spike
            call(port, f"/{account}/cards/{first['number']}", "DELETE", bearer=token, status=204)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM card_activity_spikes WHERE card_id=?", (first["id"],)).fetchone()[0] == 0
            call(port, f"/{account}/boards/{board['id']}", "DELETE", bearer=token, status=204)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM card_activity_spikes WHERE account_id=?", (account_id,)).fetchone()[0] == 0
        finally:
            stop(server)
    print("activity-spike and stalled-index checks passed")


if __name__ == "__main__":
    main()
