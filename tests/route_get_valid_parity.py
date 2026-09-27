#!/usr/bin/env python3
"""Probe GET contracts with equivalent persisted fixtures on paired servers."""

import hashlib
import json
import time
import urllib.error
import urllib.request

from draft_workflow_parity import request as browser_request


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


def fixture(target):
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
    named_card = create(f"/boards/{board_id}/cards", {"card": {"title": "Named column card"}})
    status, _, _ = request(target, f"/cards/{named_card['number']}/triage", "POST", {"column_id": column_id})
    assert status == 204, (target["name"], "triage", status)
    step = create(f"/cards/{number}/steps", {"step": {"content": "First step"}})
    comment = create(f"/cards/{number}/comments", {"comment": {"body": "First comment"}})
    create(f"/cards/{number}/reactions", {"reaction": {"content": "👍"}})
    return {"name": name, "user_id": user_id, "board_id": board_id,
            "column_id": column_id, "webhook_id": webhook_id,
            "number": number, "card_id": card["id"], "step_id": step["id"],
            "comment_id": comment["id"]}


def run(target):
    item = fixture(target)
    user_id, board_id, column_id = (item[key] for key in ("user_id", "board_id", "column_id"))
    webhook_id, number, step_id, comment_id = (
        item[key] for key in ("webhook_id", "number", "step_id", "comment_id"))
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
        "step": f"/cards/{number}/steps/{step_id}",
        "card_comments": f"/cards/{number}/comments",
        "comment": f"/cards/{number}/comments/{comment_id}",
        "card_reactions": f"/cards/{number}/reactions",
        "comment_reactions": f"/cards/{number}/comments/{comment_id}/reactions",
    }
    result = {}
    for label, path in paths.items():
        status, content_type, value = request(target, path)
        if label != "user_events":
            assert status == 200, (target["name"], label, status, value)
        result[label] = (status, content_type, shape(value))
    return result


def run_html(target):
    item = fixture(target)
    board_id, column_id, webhook_id, number, step_id, comment_id = (
        item[key] for key in ("board_id", "column_id", "webhook_id", "number", "step_id", "comment_id"))
    paths = {
        "board_accesses": (f"/boards/{board_id}/accesses", None),
        "board": (f"/boards/{board_id}", item["name"]),
        "board_edit": (f"/boards/{board_id}/edit", item["name"]),
        "board_stream": (f"/boards/{board_id}/columns/stream", "Valid route card"),
        "board_columns": (f"/boards/{board_id}/columns", "Doing"),
        "column": (f"/boards/{board_id}/columns/{column_id}", "Doing"),
        "column_cards": (f"/boards/{board_id}/columns/{column_id}/cards", "Named column card"),
        "webhooks": (f"/boards/{board_id}/webhooks", "Parity"),
        "webhook_new": (f"/boards/{board_id}/webhooks/new", None),
        "webhook": (f"/boards/{board_id}/webhooks/{webhook_id}", "Parity"),
        "webhook_edit": (f"/boards/{board_id}/webhooks/{webhook_id}/edit", "Parity"),
        "card": (f"/cards/{number}", "Valid route card"),
        "card_edit": (f"/cards/{number}/edit", "Valid route card"),
        "card_board_edit": (f"/cards/{number}/board/edit", item["name"]),
        "card_column_edit": (f"/cards/{number}/column/edit", "Doing"),
        "card_assignments_new": (f"/cards/{number}/assignments/new", "Assign this to"),
        "card_taggings_new": (f"/cards/{number}/taggings/new", None),
        "card_reactions": (f"/cards/{number}/reactions", f"reacting_card_{item['card_id']}"),
        "card_reactions_new": (f"/cards/{number}/reactions/new", "New reaction"),
        "steps": (f"/cards/{number}/steps", "First step"),
        "step": (f"/cards/{number}/steps/{step_id}", "First step"),
        "step_edit": (f"/cards/{number}/steps/{step_id}/edit", "First step"),
        "comments": (f"/cards/{number}/comments", "First comment"),
        "comment": (f"/cards/{number}/comments/{comment_id}", "First comment"),
        "comment_edit": (f"/cards/{number}/comments/{comment_id}/edit", "First comment"),
        "comment_reactions": (f"/cards/{number}/comments/{comment_id}/reactions", f"reacting_comment_{comment_id}"),
        "comment_reactions_new": (f"/cards/{number}/comments/{comment_id}/reactions/new", "New reaction"),
        "prompt_users": ("/prompts/users", "lexxy-prompt-item"),
        "prompt_board_users": (f"/prompts/boards/{board_id}/users", "lexxy-prompt-item"),
        "prompt_cards": ("/prompts/cards", "Valid route card"),
    }
    result = {}
    for label, (path, marker) in paths.items():
        status, headers, body = browser_request(
            target, f"/{target['account']}{path}", accept="text/html")
        content_type = headers.get_content_type()
        detail = ((marker.encode() in body) if marker else bool(body)) if status == 200 else (
            len(body), hashlib.sha256(body).hexdigest())
        result[label] = (status, content_type, detail)
        if status == 200 and marker:
            assert marker.encode() in body, (target["name"], label, marker, body[:300])
    steps_path = f"/{target['account']}/cards/{number}/steps"
    status, headers, _ = browser_request(target, steps_path, accept="application/json", bearer=True)
    assert status == 200 and headers.get("ETag"), (target["name"], "steps validator", status)
    status, headers, _ = browser_request(
        {**target, "extra_headers": {"If-None-Match": headers["ETag"]}},
        steps_path, accept="text/html")
    result["steps_conditional_html"] = (status, headers.get_content_type())
    return result
