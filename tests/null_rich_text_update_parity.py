#!/usr/bin/env python3
"""Compare JSON null and explicit empty rich-text updates on cards and boards."""
import os
import urllib.parse

from activity_event_board_parity import call


def text_fields(value, prefix):
    return value[prefix], value[f"{prefix}_html"]


def verify(target):
    board = call(target, "/boards", "POST",
                 {"board": {"name": "Null rich text", "public_description": "Board text"}}, 201)
    try:
        call(target, f"/boards/{board['id']}/publication", "POST", status=201)
        card = call(target, f"/boards/{board['id']}/cards", "POST",
                    {"card": {"title": "Null card text", "description": "Card text"}}, 201)
        card_path = f"/cards/{card['number']}"
        query = urllib.parse.urlencode({"board_ids[]": board["id"]})
        result = {}
        for key, content in (("card_null", None), ("card_empty", ""), ("card_null_again", None)):
            updated = call(target, card_path + ".json", "PATCH",
                           {"card": {"description": content}})
            result[key] = text_fields(updated, "description")
            assert text_fields(call(target, card_path), "description") == result[key]
            listed = call(target, f"/cards?{query}")
            listed_card = next(item for item in listed if item["id"] == card["id"])
            assert text_fields(listed_card, "description") == result[key]
            events = call(target, f"/activities?{query}")
            published = next(item for item in events if item["action"] == "card_published"
                             and item["eventable"]["id"] == card["id"])
            assert text_fields(published["eventable"], "description") == result[key]

        board_path = f"/boards/{board['id']}"
        for key, content in (("board_null", None), ("board_empty", "")):
            updated = call(target, board_path, "PATCH",
                           {"board": {"public_description": content}})
            result[key] = text_fields(updated, "public_description")
            assert text_fields(call(target, board_path), "public_description") == result[key]
            listed_board = next(item for item in call(target, "/boards")
                                if item["id"] == board["id"])
            assert text_fields(listed_board, "public_description") == result[key]
            new_card = call(target, f"/boards/{board['id']}/cards", "POST",
                            {"card": {"title": f"After {key}"}}, 201)
            assert text_fields(new_card["board"], "public_description") == result[key]
            activities = call(target, f"/activities?{query}")
            event = next(item for item in activities if item["action"] == "card_published"
                         and item["eventable"]["id"] == new_card["id"])
            assert text_fields(event["board"], "public_description") == result[key]
        return result
    finally:
        call(target, f"/boards/{board['id']}", "DELETE", status=204)


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        fields = ("URL", "ACCOUNT", "TOKEN")
        if any(not os.getenv(f"{name}_{field}") for field in fields):
            raise SystemExit(f"Set {name}_URL, {name}_ACCOUNT, and {name}_TOKEN")
        target = {"name": name.lower(),
                  **{field.lower(): os.environ[f"{name}_{field}"] for field in fields}}
        results[name.lower()] = verify(target)
    assert results["fizzy"] == results["rizzy"], results
    assert results["rizzy"]["card_null"] == ("", "")
    assert results["rizzy"]["board_null"] == ("", "")
    print("card and board null rich-text updates matched Fizzy")


if __name__ == "__main__":
    main()
