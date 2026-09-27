#!/usr/bin/env python3
"""Compare source and Rust webhook bodies without making outbound requests."""

import json
import re
import subprocess
import sys
import tempfile
import urllib.parse
from pathlib import Path

sys.path[:0] = [str(Path(__file__).resolve().parent),
                str(Path(__file__).resolve().parents[1] / "bench")]
from card_image_parity import call  # noqa: E402
from compare import shape  # noqa: E402
from sustained_mixed import (ROOT, RUST, SOURCE, prepare_rust, prepare_source,
                             rust_env, source_env, start_server, stop_server)  # noqa: E402


PUBLIC_URL = "https://rizzy.example"
TITLE = "Tom & Jerry's <Great> \"Adventure\""
COMMENT = "First comment & more"
HOOKS = {
    "generic": "https://example.invalid/webhook",
    "slack": "https://hooks.slack.com/services/T123/B456/secret",
    "campfire": "https://example.invalid/rooms/123/456-room-name/messages",
    "basecamp": "https://example.invalid/123/integrations/webhook/buckets/456/chats/789/lines",
}


def stable(value):
    if isinstance(value, dict):
        return {key: "<generated>" if key in {
            "id", "created_at", "updated_at", "last_active_at"
        } else stable(item) for key, item in value.items()}
    if isinstance(value, list):
        return [stable(item) for item in value]
    if isinstance(value, str):
        value = re.sub(r"/(boards|users|comments)/[^/|<>]+", r"/\1/<generated>", value)
        return re.sub(r"#comment_[^/|<>]+", "#comment_<generated>", value)
    return value


def stable_wire(body):
    body = re.sub(r"[0-9a-z]{25}", "<generated>", body)
    return re.sub(r"20\d\d-\d\d-\d\dT\d\d:\d\d:\d\d\.\d\d\dZ",
                  "<timestamp>", body)


def api(target, path, method="GET", payload=None, expected=200):
    status, _, body = call(target, f"/{target['account']}{path}", method, payload)
    assert status == expected, (target["name"], method, path, status, body[:350])
    return json.loads(body) if body else None


def source_runner(target, ruby):
    return subprocess.run(["bin/rails", "runner", ruby], cwd=SOURCE, env=target["env"],
                          check=True, text=True, capture_output=True).stdout


def fixture(target):
    board = api(target, "/boards", "POST", {"board": {"name": "Webhook wire"}}, 201)
    board_id = board["id"]
    hooks = {}
    for name, url in HOOKS.items():
        hook = api(target, f"/boards/{board_id}/webhooks", "POST", {
            "webhook": {"name": name, "url": url,
                        "subscribed_actions": ["card_published", "comment_created"]}}, 201)
        hooks[name] = hook["id"]
    card = api(target, f"/boards/{board_id}/cards", "POST", {
        "card": {"title": TITLE}}, 201)
    assert card["title"] == TITLE
    comment = api(target, f"/cards/{card['number']}/comments", "POST", {
        "comment": {"body": COMMENT}}, 201)
    assert comment["body"]["plain_text"] == COMMENT
    if target["name"] == "fizzy":
        for action in ("card_published", "comment_created"):
            ruby = ('event = Event.where(board_id: Webhook.find(' + json.dumps(hooks["generic"])
                    + ').board_id, action: ' + json.dumps(action)
                    + ').order(created_at: :desc).first; '
                    'Event::WebhookDispatchJob.perform_now(event)')
            source_runner(target, ruby)
    deliveries = {}
    for name, hook_id in hooks.items():
        rows = api(target, f"/boards/{board_id}/webhooks/{hook_id}/deliveries")
        assert len(rows) == 2 and all(row["state"] == "pending" for row in rows), (
            target["name"], name, rows)
        for row in rows:
            deliveries[f"{name}_{row['event']['action']}"] = row["id"]
    return deliveries


def wire(target, ids):
    result = {}
    for name, delivery_id in ids.items():
        if target["name"] == "fizzy":
            ruby = ('require "json"; d = Webhook::Delivery.find(' + json.dumps(delivery_id)
                    + '); puts JSON.generate({body: d.send(:payload), '
                    'content_type: d.send(:content_type)})')
            if name.startswith("generic_"):
                result[f"{name}_warm"] = json.loads(source_runner(target, ruby))
                ruby = ruby.replace('d = Webhook::Delivery.find(',
                                    'Rails.cache.clear; d = Webhook::Delivery.find(')
            result[name] = json.loads(source_runner(target, ruby))
        else:
            output = subprocess.run([str(RUST), "inspect-webhook", delivery_id], cwd=ROOT,
                                    env=target["env"], check=True, text=True,
                                    capture_output=True).stdout
            result[name] = json.loads(output)
    return result


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-webhook-wire-pair-") as temporary:
        targets = {}
        processes = []
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env = source_env(directory, 2)
                    env["BASE_URL"] = PUBLIC_URL
                    env, credentials = prepare_source(directory, 2, env)
                else:
                    env = rust_env(directory)
                    env["RIZZY_PUBLIC_URL"] = PUBLIC_URL
                    env, credentials = prepare_rust(directory, env)
                process, url = start_server(name, directory, env, [0, 1])
                processes.append(process)
                targets[name] = {"name": name, "url": url, "env": env,
                                 "account": str(credentials["account"]),
                                 "token": credentials["token"]}
            source = wire(targets["fizzy"], fixture(targets["fizzy"]))
            rust = wire(targets["rizzy"], fixture(targets["rizzy"]))
            for name in ("slack", "campfire", "basecamp"):
                name += "_card_published"
                assert source[name] == rust[name], (name, source[name], rust[name])
            card_name = "generic_card_published"
            source_generic = json.loads(source[card_name]["body"])
            rust_generic = json.loads(rust[card_name]["body"])
            warm_source_generic = json.loads(source[f"{card_name}_warm"]["body"])
            assert warm_source_generic["eventable"]["url"].startswith((
                targets["fizzy"]["url"] + "/", PUBLIC_URL + "/")), (
                    warm_source_generic["eventable"]["url"])
            assert source_generic["eventable"]["url"].startswith(PUBLIC_URL + "/")
            assert source[card_name]["content_type"] == rust[card_name]["content_type"]
            assert shape(source_generic) == shape(rust_generic), (
                shape(source_generic), shape(rust_generic))
            assert stable(source_generic) == stable(rust_generic), (
                stable(source_generic), stable(rust_generic))
            for key in ("generic_card_published", "generic_comment_created"):
                assert stable_wire(source[key]["body"]) == stable_wire(rust[key]["body"]), (
                    key, stable_wire(source[key]["body"]),
                    stable_wire(rust[key]["body"]))
            for name in ("slack", "campfire", "basecamp", "generic"):
                key = f"{name}_comment_created"
                assert source[key]["content_type"] == rust[key]["content_type"], key
                if name in ("slack", "generic"):
                    source_body = json.loads(source[key]["body"])
                    rust_body = json.loads(rust[key]["body"])
                elif name == "basecamp":
                    source_body = urllib.parse.parse_qs(source[key]["body"])["content"][0]
                    rust_body = urllib.parse.parse_qs(rust[key]["body"])["content"][0]
                else:
                    source_body, rust_body = source[key]["body"], rust[key]["body"]
                assert stable(source_body) == stable(rust_body), (
                    key, stable(source_body), stable(rust_body))
            print("PASS card webhook wire parity and stable comment payloads across four formats")
        finally:
            for process in processes:
                stop_server(process)


if __name__ == "__main__":
    main()
