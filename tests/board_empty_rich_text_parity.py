#!/usr/bin/env python3
"""Compare absent and explicit empty public board descriptions with Fizzy."""
import os
import urllib.parse

from activity_event_board_parity import call


def fields(value):
    return value["public_description"], value["public_description_html"]


def nested_fields(target, board, card):
    query = urllib.parse.urlencode({"board_ids[]": board["id"]})
    listed = next(item for item in call(target, "/boards") if item["id"] == board["id"])
    cards = call(target, f"/cards?{query}")
    card_board = next(item for item in cards if item["id"] == card["id"])["board"]
    events = call(target, f"/activities?{query}")
    event = next(item for item in events if item["action"] == "card_published"
                 and item["eventable"]["id"] == card["id"])
    assert fields(listed) == fields(card_board) == fields(event["board"]), (
        target["name"], fields(listed), fields(card_board), fields(event["board"]))
    assert fields(event["eventable"]["board"]) == fields(listed), (
        target["name"], fields(event["eventable"]["board"]), fields(listed))
    return fields(listed)


def verify(target):
    boards = []
    try:
        omitted = call(target, "/boards", "POST", {"board": {"name": "Omitted public text"}}, 201)
        boards.append(omitted)
        published = call(target, f"/boards/{omitted['id']}/publication", "POST", status=201)
        result = {"omitted": fields(published)}
        card = call(target, f"/boards/{omitted['id']}/cards", "POST",
                    {"card": {"title": "Public text fixture"}}, 201)
        assert nested_fields(target, omitted, card) == result["omitted"]

        updated = call(target, f"/boards/{omitted['id']}", "PATCH",
                       {"board": {"public_description": ""}})
        result["explicit_update"] = fields(updated)
        card = call(target, f"/boards/{omitted['id']}/cards", "POST",
                    {"card": {"title": "After explicit empty"}}, 201)
        assert nested_fields(target, omitted, card) == result["explicit_update"]

        updated = call(target, f"/boards/{omitted['id']}", "PATCH",
                       {"board": {"public_description": "<p>Visible</p>"}})
        result["nonempty"] = fields(updated)
        updated = call(target, f"/boards/{omitted['id']}", "PATCH",
                       {"board": {"public_description": ""}})
        result["cleared"] = fields(updated)
        card = call(target, f"/boards/{omitted['id']}/cards", "POST",
                    {"card": {"title": "After clear"}}, 201)
        assert nested_fields(target, omitted, card) == result["cleared"]

        explicit = call(target, "/boards", "POST",
                        {"board": {"name": "Explicit empty public text", "public_description": ""}}, 201)
        boards.append(explicit)
        result["explicit_create"] = fields(
            call(target, f"/boards/{explicit['id']}/publication", "POST", status=201))
        return result
    finally:
        for board in reversed(boards):
            call(target, f"/boards/{board['id']}", "DELETE", status=204)


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        fields_required = ("URL", "ACCOUNT", "TOKEN")
        if any(not os.getenv(f"{name}_{field}") for field in fields_required):
            raise SystemExit(f"Set {name}_URL, {name}_ACCOUNT, and {name}_TOKEN")
        target = {"name": name.lower(),
                  **{field.lower(): os.environ[f"{name}_{field}"] for field in fields_required}}
        results[name.lower()] = verify(target)
    assert results["fizzy"] == results["rizzy"], results
    assert results["rizzy"]["omitted"] == ("", "")
    assert results["rizzy"]["explicit_update"][1].startswith('<div class="action-text-content">')
    print("published board rich-text presence matched Fizzy across direct and nested JSON")


if __name__ == "__main__":
    main()
