#!/usr/bin/env python3
"""Probe successful GET routes with equivalent persisted fixtures on paired servers."""

import json
import time
import urllib.error
import urllib.request


def request(target, path, method="GET", payload=None):
    scoped = f"/{target['account']}{path}"
    headers = {"Authorization": f"Bearer {target['token']}",
               "Accept": "application/json", "Content-Type": "application/json"}
    req = urllib.request.Request(
        target["url"] + scoped,
        data=json.dumps(payload).encode() if payload is not None else None,
        method=method, headers=headers)
    try:
        response = urllib.request.urlopen(req, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read()
        content_type = response.headers.get_content_type()
        try:
            value = json.loads(raw) if raw and content_type == "application/json" else None
        except json.JSONDecodeError:
            value = None
        return response.status, content_type, value


def shape(value):
    if isinstance(value, dict):
        return {key: shape(item) for key, item in value.items()}
    if isinstance(value, list):
        return [shape(value[0])] if value else []
    return type(value).__name__


def run(target):
    name = "Valid route parity " + str(time.time_ns())

    def create(path, payload):
        status, content_type, value = request(target, path, "POST", payload)
        assert status == 201 and content_type == "application/json", (target["name"], path, status, value)
        return value

    status, _, users = request(target, "/users")
    assert status == 200 and users, (target["name"], "users", status)
    user_id = next(user["id"] for user in users if user["role"] == "owner")
    board = create("/boards", {"board": {"name": name}})
    board_id = board["id"]
    column = create(f"/boards/{board_id}/columns", {"column": {"name": "Doing"}})
    column_id = column["id"]
    webhook = create(f"/boards/{board_id}/webhooks", {"webhook": {
        "name": "Parity", "url": "https://example.invalid/rizzy-parity",
        "subscribed_actions": []}})
    webhook_id = webhook["id"]
    card = create(f"/boards/{board_id}/cards", {"card": {"title": "Valid route card", "description": "Body text"}})
    number = card["number"]
    step = create(f"/cards/{number}/steps", {"step": {"content": "First step"}})
    comment = create(f"/cards/{number}/comments", {"comment": {"body": "First comment"}})
    reaction = create(f"/cards/{number}/reactions", {"reaction": {"content": "👍"}})
    paths = {
        "user": f"/users/{user_id}",
        "user_events": f"/users/{user_id}/events",
        "board": f"/boards/{board_id}",
        "board_accesses": f"/boards/{board_id}/accesses",
        "board_columns": f"/boards/{board_id}/columns",
        "column": f"/boards/{board_id}/columns/{column_id}",
        "column_cards": f"/boards/{board_id}/columns/{column_id}/cards",
        "board_stream": f"/boards/{board_id}/columns/stream",
        "board_webhooks": f"/boards/{board_id}/webhooks",
        "webhook": f"/boards/{board_id}/webhooks/{webhook_id}",
        "webhook_deliveries": f"/boards/{board_id}/webhooks/{webhook_id}/deliveries",
        "card": f"/cards/{number}",
        "card_steps": f"/cards/{number}/steps",
        "step": f"/cards/{number}/steps/{step['id']}",
        "card_comments": f"/cards/{number}/comments",
        "comment": f"/cards/{number}/comments/{comment['id']}",
        "card_reactions": f"/cards/{number}/reactions",
        "comment_reactions": f"/cards/{number}/comments/{comment['id']}/reactions",
    }
    result = {}
    for label, path in paths.items():
        status, content_type, value = request(target, path)
        if label != "user_events":
            assert status == 200, (target["name"], label, status, value)
        result[label] = (status, content_type, shape(value))
    assert reaction["id"]
    return result
