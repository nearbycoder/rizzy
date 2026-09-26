#!/usr/bin/env python3
"""Check resumable storage totals and concurrent materialization commands."""

import os
import re
import sqlite3
import subprocess
import tempfile
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from card_image_parity import IMAGE
from storage_ledger_parity import api, upload


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-storage-totals-") as root:
        database = Path(root) / "rizzy.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}",
               "RIZZY_STORAGE_DIR": str(Path(root) / "files"),
               "RIZZY_DISABLE_STORAGE_MATERIALIZATION": "1",
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
               "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True).stdout
        account = re.search(r"Account: (\d+)", seeded).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded).group(1)
        port, server = start(env)
        target = {"name": "rizzy", "url": f"http://127.0.0.1:{port}",
                  "account": account, "token": token}
        try:
            board = api(target, "/boards", "POST", {"board": {"name": "Storage totals"}}, 201)
            card = api(target, f"/boards/{board['id']}/cards", "POST",
                       {"card": {"title": "Tracked image"}}, 201)
            image = upload(target, IMAGE, "moon.jpg", "image/jpeg")
            api(target, f"/cards/{card['number']}.json", "PATCH",
                {"card": {"image": image["signed_id"]}})

            def total(owner_type, owner_id):
                with sqlite3.connect(database) as db:
                    return db.execute(
                        "SELECT bytes_stored,last_entry_id,updated_at FROM storage_totals "
                        "WHERE owner_type=? AND owner_id=?", (owner_type, owner_id)
                    ).fetchone()

            with sqlite3.connect(database) as db:
                account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?",
                                        (int(account),)).fetchone()[0]
                # Model an upgraded database whose existing attachment predates the ledger.
                db.execute("DELETE FROM storage_entries")
            subprocess.run([BINARY, "reconcile-storage"], cwd=ROOT, env=env,
                           check=True, capture_output=True)
            with sqlite3.connect(database) as db:
                entries = db.execute(
                    "SELECT board_id,delta,operation FROM storage_entries").fetchall()
                assert entries == [(board["id"], len(IMAGE), "reconcile")], entries
                db.execute(
                    "INSERT INTO storage_entries(id,account_id,board_id,created_at,delta,operation) "
                    "VALUES(?,?,?,?,?,'attach')",
                    ("0" * 25, account_id, board["id"], "2020-01-01T00:00:00Z", 17))
            subprocess.run([BINARY, "reconcile-storage"], cwd=ROOT, env=env,
                           check=True, capture_output=True)
            with sqlite3.connect(database) as db:
                assert db.execute(
                    "SELECT delta FROM storage_entries WHERE operation='reconcile' "
                    "ORDER BY created_at DESC LIMIT 1").fetchone()[0] == -17
            subprocess.run([BINARY, "reconcile-storage"], cwd=ROOT, env=env,
                           check=True, capture_output=True)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM storage_entries").fetchone()[0] == 3
            assert total("Account", account_id) is None
            assert total("Board", board["id"]) is None
            workers = [subprocess.Popen([BINARY, "materialize-storage"], cwd=ROOT, env=env,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                       for _ in range(2)]
            for worker in workers:
                out, err = worker.communicate(timeout=15)
                assert worker.returncode == 0, (out, err)
            account_total = total("Account", account_id)
            board_total = total("Board", board["id"])
            assert account_total and board_total
            assert account_total[0] == board_total[0] == len(IMAGE)
            assert account_total[1] and board_total[1]

            subprocess.run([BINARY, "materialize-storage"], cwd=ROOT, env=env,
                           check=True, capture_output=True)
            assert total("Account", account_id) == account_total
            assert total("Board", board["id"]) == board_total

            api(target, f"/cards/{card['number']}/image.json", "DELETE", expected=204)
            subprocess.run([BINARY, "materialize-storage"], cwd=ROOT, env=env,
                           check=True, capture_output=True)
            assert total("Account", account_id)[0] == 0
            assert total("Board", board["id"])[0] == 0
            assert total("Account", account_id)[1] != account_total[1]
            assert total("Board", board["id"])[1] != board_total[1]

            api(target, f"/boards/{board['id']}", "DELETE", expected=204)
            assert total("Board", board["id"]) is None
            assert total("Account", account_id)[0] == 0
        finally:
            stop(server)
    print("Storage materialization passed: concurrent claims, exact totals, idempotency, and board cleanup")


if __name__ == "__main__":
    main()
