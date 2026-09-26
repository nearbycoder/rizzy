#!/usr/bin/env python3
"""Compare four pages of recently accessed board JSON with pinned Fizzy."""
import json
import os
import sqlite3
import urllib.request
import uuid
from datetime import datetime, timedelta, timezone

from activity_event_board_parity import call, database_id
from activity_particulars_parity import stored_id
from parity import shape


COUNT = 200


def timestamp(index):
    return (datetime(2030, 1, 1, tzinfo=timezone.utc) + timedelta(seconds=index)).isoformat(
        timespec="milliseconds").replace("+00:00", "Z")


def fixture(target, count=COUNT):
    board = call(target, "/boards", "POST",
                 {"board": {"name": "Bulk board 000", "all_access": False}}, 201)
    new_ids = []
    with sqlite3.connect(target["db"]) as db:
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA foreign_keys=ON")
        original_id = database_id(target, board["id"])
        original = db.execute("SELECT * FROM boards WHERE id=?", (original_id,)).fetchone()
        access = db.execute("SELECT * FROM accesses WHERE board_id=?", (original_id,)).fetchone()
        assert original and access, target["name"]
        baseline = db.execute("SELECT COUNT(*) FROM boards b JOIN accesses x ON x.board_id=b.id "
                              "WHERE b.account_id=? AND x.user_id=?",
                              (original["account_id"], access["user_id"])).fetchone()[0]
        db.execute("UPDATE accesses SET accessed_at=? WHERE board_id=? AND user_id=?",
                   (timestamp(0), original_id, access["user_id"]))
        board_columns = list(original.keys())
        board_insert = f"INSERT INTO boards ({','.join(board_columns)}) VALUES ({','.join('?' for _ in board_columns)})"
        access_columns = list(access.keys())
        access_insert = f"INSERT INTO accesses ({','.join(access_columns)}) VALUES ({','.join('?' for _ in access_columns)})"
        for index in range(1, count):
            board_id = stored_id(target, uuid.uuid4().int)
            new_ids.append(board_id)
            clone = dict(original)
            clone["id"] = board_id
            clone["name"] = f"Bulk board {index:03d}"
            db.execute(board_insert, [clone[column] for column in board_columns])
            cloned_access = dict(access)
            if "id" in cloned_access:
                cloned_access["id"] = stored_id(target, uuid.uuid4().int)
            cloned_access["board_id"] = board_id
            cloned_access["accessed_at"] = timestamp(index)
            db.execute(access_insert, [cloned_access[column] for column in access_columns])
    return board, new_ids, baseline


def page(target, number):
    url = f"{target['url']}/{target['account']}/boards.json?page={number}"
    request = urllib.request.Request(
        url, headers={"Authorization": f"Bearer {target['token']}",
                      "Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.load(response), response.headers


def verify(target, baseline, count=COUNT):
    assert count >= COUNT
    pages = []
    ordered = []
    first_shape = None
    expected_sizes = (15, 30, 50, 100, 5)
    for number, expected_size in enumerate(expected_sizes, start=1):
        records, headers = page(target, number)
        assert len(records) >= expected_size, (target["name"], number, len(records))
        if number < 5:
            assert len(records) == expected_size, (target["name"], number, len(records))
        assert int(headers["X-Total-Count"]) == baseline + count - 1
        if number < 5:
            assert 'rel="next"' in headers.get("Link", ""), (target["name"], number)
        if first_shape is None:
            first_shape = shape(records[0])
        names = [record["name"] for record in records[:expected_size]]
        assert all(record["all_access"] is False for record in records[:expected_size])
        pages.append(expected_size)
        ordered.extend(names)
    expected = [f"Bulk board {index:03d}" for index in reversed(range(count - COUNT, count))]
    assert ordered == expected, (target["name"], ordered[:5], ordered[-5:])
    return pages, first_shape


def cleanup(target, board, new_ids):
    with sqlite3.connect(target["db"]) as db:
        db.execute("PRAGMA foreign_keys=ON")
        for start in range(0, len(new_ids), 500):
            batch = new_ids[start:start + 500]
            placeholders = ",".join("?" for _ in batch)
            db.execute(f"DELETE FROM accesses WHERE board_id IN ({placeholders})", batch)
            db.execute(f"DELETE FROM boards WHERE id IN ({placeholders})", batch)
    if board:
        call(target, f"/boards/{board['id']}", "DELETE", status=204)


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        fields = ("URL", "ACCOUNT", "TOKEN", "DB")
        if any(not os.getenv(f"{name}_{field}") for field in fields):
            raise SystemExit(f"Set {name}_URL, {name}_ACCOUNT, {name}_TOKEN, and {name}_DB")
        target = {"name": name.lower(),
                  **{field.lower(): os.environ[f"{name}_{field}"] for field in fields}}
        board = None
        new_ids = []
        try:
            board, new_ids, baseline = fixture(target)
            results[name.lower()] = verify(target, baseline)
        finally:
            cleanup(target, board, new_ids)
    assert results["fizzy"] == results["rizzy"], results
    print("200 recently accessed boards matched Fizzy across five geared JSON pages")


if __name__ == "__main__":
    main()
