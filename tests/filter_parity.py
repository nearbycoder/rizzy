#!/usr/bin/env python3
"""Compare Fizzy and Rizzy card-filter result sets through their JSON APIs."""
import json
import os
import time
import urllib.parse
from datetime import datetime, timedelta, timezone

from parity import request


def checked(target, path, method="GET", payload=None, expected=200):
    status, value = request(target, path, method, payload)
    assert status == expected, (path, status, value)
    return value


def run(target):
    unique = str(time.time_ns())
    board = checked(target, "/boards", "POST", {"board": {"name": "Filter parity " + unique}}, 201)
    board_id = board["id"]
    output = {}
    try:
        cards = {}
        for title in ("haggis neeps " + unique, "haggis " + unique, "neeps " + unique):
            payload = {"title": title}
            if title.startswith("neeps "):
                payload["created_at"] = "2024-01-15T10:30:00Z"
            cards[title] = checked(target, f"/boards/{board_id}/cards", "POST", {"card": payload}, 201)
        first, second, third = cards.values()
        soon_time = (datetime.now(timezone.utc) - timedelta(days=25)).isoformat().replace("+00:00", "Z")
        checked(target, f"/boards/{board_id}/cards", "POST", {"card": {"title": "closing soon " + unique, "last_active_at": soon_time}}, 201)
        output["backdated_created_at"] = third["created_at"]
        output["backdated_last_active_at"] = third["last_active_at"]
        owner = first["creator"]["id"]

        def results(name, **params):
            query = urllib.parse.urlencode({"board_ids[]": [board_id], **params}, doseq=True)
            values = checked(target, "/cards?" + query)
            output[name] = sorted(card["title"].replace(unique, "<fixture>") for card in values)

        results("all")
        results("postponing_soon", indexed_by="postponing_soon")
        results("stalled", indexed_by="stalled")
        results("two_terms", **{"terms[]": ["haggis", "neeps"]})
        checked(target, f"/cards/{second['number']}/comments", "POST", {"comment": {"body": "neeps in comment"}}, 201)
        results("two_terms_with_comment", **{"terms[]": ["haggis", "neeps"]})
        checked(target, f"/cards/{first['number']}/closure", "POST", expected=204)
        results("closed", indexed_by="closed")
        results("closed_today", closure="today")
        results("closed_by_owner", **{"closer_ids[]": [owner]})
        results("closed_by_owner_and_term", **{"closer_ids[]": [owner], "terms[]": ["haggis"]})
        checked(target, f"/cards/{first['number']}/closure", "DELETE", expected=204)
        results("reopened_today", closure="today")
        results("created_this_week", creation="this week")
        return output
    finally:
        checked(target, f"/boards/{board_id}", "DELETE", expected=204)


def main():
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        targets[name.lower()] = {key.lower(): os.environ[f"{name}_{key}"] for key in ("URL", "ACCOUNT", "TOKEN")}
    source = run(targets["fizzy"])
    rust = run(targets["rizzy"])
    differences = {key: {"fizzy": source[key], "rizzy": rust[key]} for key in source if source[key] != rust[key]}
    print(json.dumps({"checked": len(source), "differences": differences}, indent=2))
    if differences:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
