#!/usr/bin/env python3
"""Compare the nested card and comment activity JSON on equivalent records."""
import os
import urllib.parse

from activity_event_board_parity import call
from parity import shape


def verify(target):
    board = call(target, "/boards", "POST", {"board": {"name": "Activity envelope"}}, 201)
    try:
        card = call(target, f"/boards/{board['id']}/cards", "POST",
                    {"card": {"title": "Envelope <&>"}}, 201)
        comment = call(target, f"/cards/{card['number']}/comments", "POST",
                       {"comment": {"body": "Note <&>"}}, 201)
        query = urllib.parse.urlencode({"board_ids[]": board["id"]})
        events = call(target, f"/activities?{query}")
        assert len(events) == 2, (target["name"], len(events))
        by_action = {event["action"]: event for event in events}
        assert set(by_action) == {"card_published", "comment_created"}, target["name"]
        published = by_action["card_published"]
        commented = by_action["comment_created"]
        assert published["eventable_type"] == "Card"
        assert commented["eventable_type"] == "Comment"
        assert published["eventable"]["title"] == "Envelope <&>"
        assert published["eventable"]["id"] == card["id"]
        assert commented["eventable"]["id"] == comment["id"]
        assert commented["eventable"]["body"]["plain_text"] == "Note <&>"
        assert commented["eventable"]["card"]["id"] == card["id"]
        assert published["board"]["id"] == commented["board"]["id"] == board["id"]
        assert published["creator"]["id"] == commented["creator"]["id"]
        assert urllib.parse.urlsplit(published["url"]).path.endswith(f"/cards/{card['number']}")
        assert urllib.parse.urlsplit(commented["url"]).fragment == f"comment_{comment['id']}"
        current_card = call(target, f"/cards/{card['number']}")
        assert published["eventable"] == {key: current_card.get(key) for key in published["eventable"]}, (target["name"], {
            key: (value, current_card.get(key))
            for key, value in published["eventable"].items()
            if value != current_card.get(key)
        })
        listed = call(target, f"/cards?{query}")
        listed_card = next(item for item in listed if item["id"] == card["id"])
        assert listed_card["description_html"] == current_card["description_html"]
        result = {
            "shapes": {action: shape(event) for action, event in by_action.items()},
            "card_description_html": published["eventable"]["description_html"],
            "comment_body_html": commented["eventable"]["body"]["html"],
        }
        updated = call(target, f"/cards/{card['number']}.json", "PATCH",
                       {"card": {"description": ""}})
        changed = call(target, f"/activities?{query}")
        published_after = next(event for event in changed if event["action"] == "card_published")
        assert published_after["eventable"]["description_html"] == updated["description_html"]
        listed_after = call(target, f"/cards?{query}")
        assert next(item for item in listed_after if item["id"] == card["id"])["description_html"] == updated["description_html"]
        result["explicit_empty_description_html"] = updated["description_html"]
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
    print("activity card and comment envelopes matched Fizzy")


if __name__ == "__main__":
    main()
