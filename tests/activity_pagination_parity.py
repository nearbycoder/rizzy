#!/usr/bin/env python3
"""Compare all JSON activity pages and next links on a 197-event board."""
import json
import os
import re
import sqlite3
import urllib.parse
import urllib.request
import uuid
from datetime import datetime, timedelta, timezone

from activity_event_board_parity import call, database_id


def new_event_id(target):
    return uuid.uuid4().bytes if target["name"] == "fizzy" else uuid.uuid4().hex


def fixture(target):
    board = call(target, "/boards", "POST", {"board": {"name": "Activity pagination"}}, 201)
    card = call(target, f"/boards/{board['id']}/cards", "POST",
                {"card": {"title": "Pagination card"}}, 201)
    board_id = database_id(target, board["id"])
    with sqlite3.connect(target["db"]) as db:
        db.row_factory = sqlite3.Row
        event = db.execute("SELECT * FROM events WHERE board_id=? AND action='card_published' "
                           "ORDER BY created_at DESC LIMIT 1", (board_id,)).fetchone()
        assert event, target["name"]
        origin = datetime(2024, 1, 1, tzinfo=timezone.utc)

        def stamp(index):
            return (origin + timedelta(seconds=index)).isoformat(timespec="milliseconds").replace("+00:00", "Z")

        db.execute("UPDATE events SET created_at=?,updated_at=? WHERE id=?",
                   (stamp(196), stamp(196), event["id"]))
        columns = list(event.keys())
        query = f"INSERT INTO events ({','.join(columns)}) VALUES ({','.join('?' for _ in columns)})"
        for index in range(196):
            cloned = dict(event)
            cloned["id"] = new_event_id(target)
            cloned["created_at"] = stamp(index)
            cloned["updated_at"] = stamp(index)
            db.execute(query, [cloned[column] for column in columns])
    return board, card


def page(target, board_id, number):
    query = urllib.parse.urlencode({"board_ids[]": board_id, "page": number})
    url = f"{target['url']}/{target['account']}/activities.json?{query}"
    request = urllib.request.Request(url, headers={"Authorization": f"Bearer {target['token']}",
                                                   "Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.load(response), response.headers


def verify(target, board, card):
    pages, times, ids = [], [], []
    for number in range(1, 7):
        items, headers = page(target, board["id"], number)
        pages.append(len(items))
        assert all(item["board"]["id"] == board["id"] for item in items)
        assert all(item["eventable"]["id"] == card["id"] for item in items)
        ids.extend(item["id"] for item in items)
        times.extend(datetime.fromisoformat(item["created_at"].replace("Z", "+00:00"))
                     for item in items)
        assert int(headers["X-Total-Count"]) == 197
        link = headers.get("Link")
        if number != 5:
            assert link and 'rel="next"' in link, (target["name"], number, link)
            match = re.search(r"<([^>]+)>;\s*rel=\"next\"", link)
            assert match, (target["name"], link)
            parsed = urllib.parse.urlparse(match.group(1))
            assert parsed.path == f"/{target['account']}/activities.json"
            params = urllib.parse.parse_qs(parsed.query)
            assert params.get("page") == [str(number + 1)]
            assert params.get("board_ids[]") == [board["id"]]
        else:
            assert not link or 'rel="next"' not in link, (target["name"], number, link)
    assert len(ids) == len(set(ids)) == 197
    assert times == sorted(times, reverse=True)
    return pages, [stamp.isoformat() for stamp in times]


def main():
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        fields = ("URL", "ACCOUNT", "TOKEN", "DB")
        if any(not os.getenv(f"{name}_{field}") for field in fields):
            raise SystemExit(f"Set {name}_URL, {name}_ACCOUNT, {name}_TOKEN, and {name}_DB")
        targets[name.lower()] = {"name": name.lower(),
                                 **{field.lower(): os.environ[f"{name}_{field}"] for field in fields}}
    results = {}
    for name, target in targets.items():
        board = None
        try:
            board, card = fixture(target)
            results[name] = verify(target, board, card)
        finally:
            if board:
                call(target, f"/boards/{board['id']}", "DELETE", status=204)
    assert results["fizzy"] == results["rizzy"], {name: value[0] for name, value in results.items()}
    print("all activity pages, order, and next links matched Fizzy:", results["rizzy"][0])


if __name__ == "__main__":
    main()
