#!/usr/bin/env python3
"""Probe GET contracts with equivalent persisted fixtures on paired servers."""

import hashlib
import json
import re
import urllib.error
import urllib.parse
import urllib.request

from draft_workflow_parity import request as browser_request


def request(target, path, method="GET", payload=None, include_headers=False):
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
        result = (response.status, content_type, value)
        if include_headers:
            total = response.headers.get("X-Total-Count")
            result += ({"total": int(total) if total is not None else None,
                        "has_link": "Link" in response.headers,
                        "has_etag": "ETag" in response.headers},)
        return result


def conditional_get(target, path):
    url = f"{target['url']}/{target['account']}{path}"
    headers = {"Authorization": f"Bearer {target['token']}",
               "Accept": "application/json"}
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=15) as response:
        etag = response.headers.get("ETag")
        assert response.status == 200 and etag, (target["name"], path, response.status)
    try:
        response = urllib.request.urlopen(
            urllib.request.Request(url, headers={**headers, "If-None-Match": etag}), timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return (response.status, len(response.read()),
                response.headers.get("Content-Type"), response.headers.get("ETag") == etag)


def empty_tags_etag(target):
    request = urllib.request.Request(
        f"{target['url']}/{target['account']}/tags",
        headers={"Authorization": f"Bearer {target['token']}",
                 "Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=15) as response:
        assert json.load(response) == [], target["name"]
        return response.headers["ETag"]


def shape(value):
    if isinstance(value, dict):
        return {key: shape(item) for key, item in value.items()}
    if isinstance(value, list):
        return [shape(value[0])] if value else []
    return type(value).__name__


def stable_json(value):
    """Keep response values that can be compared across separately seeded apps."""
    if isinstance(value, dict):
        result = {}
        for key, item in value.items():
            if key in ("id", "signing_secret") or key.endswith(("_id", "_at")):
                continue
            if (key == "url" or key.endswith("_url")) and key != "payload_url" and isinstance(item, str):
                parsed = urllib.parse.urlsplit(item)
                path_and_query = parsed.path + (("?" + parsed.query) if parsed.query else "")
                result[key] = re.sub(
                    r"(?<![0-9A-Za-z])[0-9a-z]{25,26}(?![0-9A-Za-z])", ":id",
                    path_and_query)
            else:
                result[key] = stable_json(item)
        return result
    if isinstance(value, list):
        return [stable_json(item) for item in value]
    return value


def fixture(target):
    name = "Valid route parity"

    def create(path, payload):
        status, content_type, value = request(target, path, "POST", payload)
        assert status == 201 and content_type == "application/json", (target["name"], path, status, value)
        return value

    status, _, users = request(target, "/users")
    assert status == 200 and users, (target["name"], "users", status)
    empty_etag = empty_tags_etag(target)
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
    postponed = create(f"/boards/{board_id}/cards", {"card": {"title": "Postponed route card"}})
    status, _, _ = request(target, f"/cards/{postponed['number']}/not_now", "POST", {})
    assert status == 204, (target["name"], "postponement", status)
    closed = create(f"/boards/{board_id}/cards", {"card": {"title": "Closed route card"}})
    status, _, _ = request(target, f"/cards/{closed['number']}/closure", "POST", {})
    assert status == 204, (target["name"], "closure", status)
    step = create(f"/cards/{number}/steps", {"step": {"content": "First step"}})
    comment = create(f"/cards/{number}/comments", {"comment": {"body": "First comment"}})
    create(f"/cards/{number}/reactions", {"reaction": {"content": "👍"}})
    status, _, _ = request(target, f"/cards/{number}/taggings", "POST",
                           {"tag_title": "Route parity tag"})
    assert status == 204, (target["name"], "tagging", status)
    return {"name": name, "user_id": user_id, "board_id": board_id,
            "column_id": column_id, "webhook_id": webhook_id,
            "number": number, "card_id": card["id"], "step_id": step["id"],
            "comment_id": comment["id"], "empty_tags_etag": empty_etag}


def semantic(label, value):
    if label == "user":
        return {"role": value["role"], "active": value.get("active")}
    if label == "board":
        return stable_json(value)
    if label == "board_accesses":
        return {"all_access": value["all_access"],
                "users": [(user["role"], user["has_access"], user["involvement"])
                          for user in value["users"]]}
    if label == "board_columns":
        return [(column["name"], column.get("color")) for column in value]
    if label == "column":
        return {"name": value["name"], "color": value.get("color")}
    if label in ("column_cards", "board_stream"):
        return [card["title"] for card in value]
    if label == "board_webhooks":
        return [webhook["name"] for webhook in value]
    if label == "webhook":
        return {key: value.get(key) for key in
                ("name", "payload_url", "subscribed_actions")}
    if label == "webhook_deliveries":
        return value
    if label == "card":
        return stable_json(value)
    if label in ("activities", "search", "users", "tags", "notifications"):
        return stable_json(value)
    if label == "card_steps":
        return [(step["content"], step["completed"]) for step in value]
    if label == "step":
        return (value["content"], value["completed"])
    if label == "card_comments":
        return [comment["body"]["plain_text"] for comment in value]
    if label == "comment":
        return value["body"]["plain_text"]
    if label in ("card_reactions", "comment_reactions"):
        return [reaction["content"] for reaction in value]
    if label == "user_events":
        return value
    if label == "boards_index":
        return [board["name"] for board in value]
    if label in ("cards_index", "not_now", "closed"):
        return [card["title"] for card in value]
    if label in ("pin", "watch", "previews"):
        return None
    raise AssertionError(label)


def run(target):
    item = fixture(target)
    user_id, board_id, column_id = (item[key] for key in ("user_id", "board_id", "column_id"))
    webhook_id, number, step_id, comment_id = (
        item[key] for key in ("webhook_id", "number", "step_id", "comment_id"))
    paths = {
        "boards_index": "/boards",
        "users": "/users",
        "tags": "/tags",
        "notifications": "/notifications",
        "cards_index": "/cards",
        "not_now": f"/boards/{board_id}/columns/not_now",
        "closed": f"/boards/{board_id}/columns/closed",
        "pin": f"/cards/{number}/pin",
        "watch": f"/cards/{number}/watch",
        "previews": "/cards/previews",
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
        "activities": f"/activities?board_ids%5B%5D={board_id}",
        "search": "/search?q=Valid%20route",
        "card_steps": f"/cards/{number}/steps",
        "step": f"/cards/{number}/steps/{step_id}",
        "card_comments": f"/cards/{number}/comments",
        "comment": f"/cards/{number}/comments/{comment_id}",
        "card_reactions": f"/cards/{number}/reactions",
        "comment_reactions": f"/cards/{number}/comments/{comment_id}/reactions",
    }
    result = {}
    for label, path in paths.items():
        status, content_type, value, pagination = request(
            target, path, include_headers=True)
        if label not in ("user_events", "pin", "watch", "previews"):
            assert status == 200, (target["name"], label, status, value)
        if label == "webhook":
            assert len(value["signing_secret"]) == 24, (target["name"], label, value)
        deep = stable_json(value) if status == 200 and value is not None else None
        result[label] = [status, content_type, shape(value), semantic(label, value), deep,
                         pagination]
    values = {label: contract[3] for label, contract in result.items()}
    expected = {
        "board": ("name", item["name"]),
        "column": ("name", "Doing"),
        "webhook": ("payload_url", "https://example.invalid/rizzy-parity"),
        "card": ("title", "Valid route card"),
        "step": (0, "First step"),
    }
    for label, (field, wanted) in expected.items():
        assert values[label][field] == wanted, (target["name"], label, values[label])
    assert values["board_columns"][0][0] == "Doing", (target["name"], values["board_columns"])
    assert item["name"] in values["boards_index"], (target["name"], values["boards_index"])
    assert any(user["role"] == "owner" for user in values["users"]), (
        target["name"], values["users"])
    assert [tag["title"] for tag in values["tags"]] == ["route parity tag"], (
        target["name"], values["tags"])
    assert "Valid route card" in values["cards_index"], (target["name"], values["cards_index"])
    assert values["not_now"] == ["Postponed route card"], (target["name"], values["not_now"])
    assert values["closed"] == ["Closed route card"], (target["name"], values["closed"])
    assert values["column_cards"] == ["Named column card"], (target["name"], values["column_cards"])
    assert "Valid route card" in values["board_stream"], (target["name"], values["board_stream"])
    assert values["card"]["board"]["name"] == item["name"], (target["name"], values["card"])
    assert values["activities"], (target["name"], values["activities"])
    assert any(card["title"] == "Valid route card" for card in values["search"]), (
        target["name"], values["search"])
    assert ("First step", False) in values["card_steps"], (target["name"], values["card_steps"])
    assert values["comment"] == "First comment", (target["name"], values["comment"])
    assert "First comment" in values["card_comments"], (target["name"], values["card_comments"])
    assert "👍" in values["card_reactions"], (target["name"], values["card_reactions"])
    assert values["webhook_deliveries"] == [], (target["name"], values["webhook_deliveries"])
    assert values["comment_reactions"] == [], (target["name"], values["comment_reactions"])
    for label in ("users", "tags"):
        result[f"conditional_{label}"] = conditional_get(target, paths[label])
    result["empty_tags_etag"] = item["empty_tags_etag"]
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
        result[label] = (status, content_type, detail, bool(headers.get("ETag")))
        # These three Rust routes still return a fragment where Fizzy renders
        # the full layout. Their validators cannot share the same lifecycle yet.
        if status == 200 and headers.get("ETag") and label not in {
            "card_reactions", "comment_reactions", "comment"}:
            conditional_status, conditional_headers, conditional_body = browser_request(
                {**target, "extra_headers": {"If-None-Match": headers["ETag"]}},
                f"/{target['account']}{path}", accept="text/html")
            result[f"{label}_conditional"] = (
                conditional_status, conditional_headers.get("ETag") == headers["ETag"],
                len(conditional_body) == 0)
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
