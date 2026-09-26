#!/usr/bin/env python3
"""Compare empty and missing comment body behavior with Fizzy."""
import os
import urllib.parse
import urllib.request
import urllib.error
import json

from activity_event_board_parity import call
from parity import request


def form_request(target, path, fields):
    data = urllib.parse.urlencode(fields).encode()
    request_obj = urllib.request.Request(
        f"{target['url']}/{target['account']}{path}", data=data,
        headers={"Authorization": f"Bearer {target['token']}",
                 "Accept": "application/json",
                 "Content-Type": "application/x-www-form-urlencoded"})
    try:
        response = urllib.request.urlopen(request_obj, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read()
        return response.status, json.loads(raw) if raw else None


def verify(target):
    board = call(target, "/boards", "POST", {"board": {"name": "Empty comment body"}}, 201)
    try:
        card = call(target, f"/boards/{board['id']}/cards", "POST",
                    {"card": {"title": "Comment text states"}}, 201)
        path = f"/cards/{card['number']}/comments"
        missing_status, _ = request(target, path, "POST", {"comment": {}})
        assert missing_status == 400, (target["name"], missing_status)
        missing_form_status, _ = form_request(target, path, {})
        assert missing_form_status == 400, (target["name"], missing_form_status)
        dated_form_status, dated_form = form_request(
            target, path, {"comment[created_at]": "2025-01-01T00:00:00Z"})
        assert dated_form_status == 201 and dated_form["body"] == {"plain_text": "", "html": ""}
        comment = call(target, path, "POST", {"comment": {"body": ""}}, 201)
        assert comment["body"]["plain_text"] == ""
        assert comment["body"]["html"].startswith('<div class="action-text-content">')
        comment_path = f"{path}/{comment['id']}"
        shown = call(target, comment_path)
        assert shown["body"] == comment["body"]
        listed = call(target, path)
        assert next(item for item in listed if item["id"] == comment["id"])["body"] == comment["body"]
        query = urllib.parse.urlencode({"board_ids[]": board["id"]})
        activities = call(target, f"/activities?{query}")
        event = next(item for item in activities if item["eventable_type"] == "Comment"
                     and item["eventable"]["id"] == comment["id"])
        assert event["eventable"]["body"] == comment["body"]
        missing_update_status, _ = request(target, comment_path, "PATCH", {"comment": {}})
        assert missing_update_status == 400, (target["name"], missing_update_status)
        filled = call(target, comment_path, "PATCH", {"comment": {"body": "Filled"}})
        assert filled["body"]["plain_text"] == "Filled"
        nulled = call(target, comment_path, "PATCH", {"comment": {"body": None}})
        assert nulled["body"] == {"plain_text": "", "html": ""}
        cleared = call(target, comment_path, "PATCH", {"comment": {"body": ""}})
        assert cleared["body"] == comment["body"]
        null_comment = call(target, path, "POST", {"comment": {"body": None}}, 201)
        assert null_comment["body"] == {"plain_text": "", "html": ""}
        assert call(target, f"{path}/{null_comment['id']}")["body"] == null_comment["body"]
        listed = call(target, path)
        assert next(item for item in listed if item["id"] == null_comment["id"])["body"] == null_comment["body"]
        events = call(target, f"/activities?{query}")
        null_event = next(item for item in events if item["eventable_type"] == "Comment"
                          and item["eventable"]["id"] == null_comment["id"])
        assert null_event["eventable"]["body"] == null_comment["body"]
        return {"missing_create": missing_status, "missing_form": missing_form_status,
                "dated_form": dated_form["body"], "missing_update": missing_update_status,
                "empty": comment["body"], "filled": filled["body"],
                "nulled": nulled["body"], "cleared": cleared["body"],
                "null_create": null_comment["body"]}
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
    print("empty and missing comment body behavior matched Fizzy")


if __name__ == "__main__":
    main()
