#!/usr/bin/env python3
"""Compare a complete board-access user traversal with pinned Fizzy."""

import os
import sqlite3
import subprocess
import sys
import tempfile
import uuid
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import api, expect  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, start_server,
                             stop_server)  # noqa: E402
from activity_particulars_parity import stored_id  # noqa: E402
from activity_event_board_parity import database_id  # noqa: E402


def clone_users(target, database, board_id):
    with sqlite3.connect(database) as db:
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA foreign_keys=ON")
        owner = db.execute("SELECT * FROM users WHERE role='owner' LIMIT 1").fetchone()
        identity = db.execute("SELECT * FROM identities WHERE id=?", (owner["identity_id"],)).fetchone()
        board_key = database_id(target, board_id)
        owner_access = db.execute("SELECT * FROM accesses WHERE board_id=? AND user_id=?",
                                  (board_key, owner["id"])).fetchone()
        def insert(table, values):
            columns = list(values)
            db.execute(f"INSERT INTO {table} ({','.join(columns)}) VALUES ({','.join('?' for _ in columns)})",
                       [values[column] for column in columns])

        for index in range(200):
            identity_values = dict(identity)
            identity_values["id"] = stored_id(target, uuid.uuid4().int)
            identity_values["email_address"] = f"access-{index:03d}@example.test"
            insert("identities", identity_values)
            user_values = dict(owner)
            user_values["id"] = stored_id(target, uuid.uuid4().int)
            user_values["identity_id"] = identity_values["id"]
            user_values["name"] = f"Member {index:03d}"
            user_values["role"] = "member"
            insert("users", user_values)
            if index < 2:
                access_values = dict(owner_access)
                if "id" in access_values:
                    access_values["id"] = stored_id(target, uuid.uuid4().int)
                access_values["user_id"] = user_values["id"]
                access_values["involvement"] = "access_only" if index == 0 else "watching"
                insert("accesses", access_values)


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    process, url = start_server(name, directory, env,
                                sorted(os.sched_getaffinity(0))[:2])
    try:
        target = {"name": name, "url": url, "account": str(credentials["account"]),
                  "token": credentials["token"]}
        board, _ = expect(target, "/boards", "POST",
                          {"board": {"name": "Access traversal", "all_access": False}}, 201)
        database = directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")
        clone_users(target, database, board["id"])
        pages = []
        names = []
        access = []
        for number in range(1, 7):
            status, value, headers = api(target, f"/boards/{board['id']}/accesses?page={number}")
            assert status == 200, (name, number, status)
            users = value["users"]
            assert value["all_access"] is False
            assert value["board_id"] == board["id"]
            for user in users:
                assert set(user) == {"id", "name", "role", "active", "email_address",
                                     "created_at", "url", "avatar_url", "has_access",
                                     "involvement"}, (name, user)
                assert user["active"] is True and user["role"] in ("owner", "member")
                assert user["url"] == f"{url}/{target['account']}/users/{user['id']}"
                assert user["avatar_url"] == f"{user['url']}/avatar"
                datetime.fromisoformat(user["created_at"].replace("Z", "+00:00"))
            pages.append((len(users), int(headers["X-Total-Count"]), "Link" in headers))
            names.extend(user["name"] for user in users)
            access.extend((user["name"], user["has_access"], user["involvement"])
                          for user in users)
        assert len(names) == len(set(names)) == 201, (name, len(names))
        assert names == ["Member %03d" % index for index in range(200)] + ["Owner"], name
        assert access[0] == ("Member 000", True, "access_only"), access[0]
        assert access[1] == ("Member 001", True, "watching"), access[1]
        assert all(not has_access and involvement is None for _, has_access, involvement in access[2:-1])
        assert access[-1] == ("Owner", True, "watching"), access[-1]
        return pages, access
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-board-accesses-") as temporary:
        root = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = root / name
            directory.mkdir()
            results[name] = check(name, directory)
        assert results["fizzy"] == results["rizzy"], results
        assert results["fizzy"][0] == [(15, 201, True), (30, 201, True),
                                        (50, 201, True), (100, 201, True),
                                        (6, 201, False), (0, 201, True)], results["fizzy"][0]
    print("Board access pagination and membership matched pinned Fizzy")


if __name__ == "__main__":
    main()
