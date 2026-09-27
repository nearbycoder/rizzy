#!/usr/bin/env python3
"""Verify imported card timestamps and date-dependent card filtering."""
import os
import re
import sqlite3
import subprocess
import tempfile
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from join import call


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "timestamps.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        seed = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seed.stdout).group(1)
        token = re.search(r"Bearer token: (\w+)", seed.stdout).group(1)
        port, server = start(env)
        try:
            board, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Import dates"}}, bearer=token, status=201)
            path = f"/{account}/boards/{board['id']}/cards"
            date = "2024-01-15T10:30:00.000Z"
            active = "2024-06-01T12:00:00.000Z"
            old, _ = call(port, path, "POST", {"card": {"title": "Old", "created_at": date}}, bearer=token, status=201)
            assert old["created_at"] == date and old["last_active_at"] == date
            restored, _ = call(port, path, "POST", {"card": {"title": "Restored", "created_at": date, "last_active_at": active}}, bearer=token, status=201)
            assert restored["created_at"] == date and restored["last_active_at"] == active
            call(port, f"/{account}/cards/{restored['number']}/comments", "POST", {"comment": {"body": "Touch activity"}}, bearer=token, status=201)
            touched, _ = call(port, f"/{account}/cards/{restored['number']}", bearer=token)
            assert touched["last_active_at"] != active
            fixed, _ = call(port, f"/{account}/cards/{restored['number']}", "PUT", {"card": {"last_active_at": active}}, bearer=token)
            assert fixed["last_active_at"] == active and fixed["created_at"] == date
            fresh, _ = call(port, path, "POST", {"card": {"title": "Fresh"}}, bearer=token, status=201)
            cards, _ = call(port, f"/{account}/cards?board_ids%5B%5D={board['id']}&creation=thisweek", bearer=token)
            assert [card["id"] for card in cards] == [fresh["id"]]
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT created_at,last_active_at FROM cards WHERE id=?", (restored["id"],)).fetchone() == (
                    date.replace("Z", "000Z"), active.replace("Z", "000Z"))
            call(port, path, "POST", {"card": {"title": "Invalid", "created_at": "not-a-time"}}, bearer=token, status=422)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM cards WHERE board_id=?", (board["id"],)).fetchone()[0] == 3
        finally:
            stop(server)
    print("card timestamp checks passed")


if __name__ == "__main__":
    main()
