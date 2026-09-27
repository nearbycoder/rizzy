#!/usr/bin/env python3
"""Compare source and Rust webhook bodies without making outbound requests."""

import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path[:0] = [str(Path(__file__).resolve().parent),
                str(Path(__file__).resolve().parents[1] / "bench")]
from card_image_parity import call  # noqa: E402
from compare import shape  # noqa: E402
from sustained_mixed import (ROOT, RUST, SOURCE, prepare_rust, prepare_source,
                             rust_env, source_env, start_server, stop_server)  # noqa: E402


PUBLIC_URL = "https://rizzy.example"
TITLE = "Tom & Jerry's <Great> \"Adventure\""
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
        return re.sub(r"/(boards|users)/[^/]+", r"/\1/<generated>", value)
    return value


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
                        "subscribed_actions": ["card_published"]}}, 201)
        hooks[name] = hook["id"]
    card = api(target, f"/boards/{board_id}/cards", "POST", {
        "card": {"title": TITLE}}, 201)
    assert card["title"] == TITLE
    if target["name"] == "fizzy":
        ruby = ('event = Event.where(board_id: Webhook.find(' + json.dumps(hooks["generic"])
                + ').board_id, action: "card_published").order(created_at: :desc).first; '
                'Event::WebhookDispatchJob.perform_now(event)')
        source_runner(target, ruby)
    deliveries = {}
    for name, hook_id in hooks.items():
        rows = api(target, f"/boards/{board_id}/webhooks/{hook_id}/deliveries")
        assert len(rows) == 1 and rows[0]["state"] == "pending", (target["name"], name, rows)
        deliveries[name] = rows[0]["id"]
    return deliveries


def wire(target, ids):
    result = {}
    for name, delivery_id in ids.items():
        if target["name"] == "fizzy":
            ruby = ('require "json"; d = Webhook::Delivery.find(' + json.dumps(delivery_id)
                    + '); puts JSON.generate({body: d.send(:payload), '
                    'content_type: d.send(:content_type)})')
            if name == "generic":
                result["generic_warm"] = json.loads(source_runner(target, ruby))
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
                assert source[name] == rust[name], (name, source[name], rust[name])
            source_generic = json.loads(source["generic"]["body"])
            rust_generic = json.loads(rust["generic"]["body"])
            warm_source_generic = json.loads(source["generic_warm"]["body"])
            assert warm_source_generic["eventable"]["url"].startswith(
                targets["fizzy"]["url"] + "/"), warm_source_generic["eventable"]["url"]
            assert source_generic["eventable"]["url"].startswith(PUBLIC_URL + "/")
            assert source["generic"]["content_type"] == rust["generic"]["content_type"]
            assert shape(source_generic) == shape(rust_generic), (
                shape(source_generic), shape(rust_generic))
            assert stable(source_generic) == stable(rust_generic), (
                stable(source_generic), stable(rust_generic))
            print("PASS exact Slack, Campfire, and Basecamp payload parity; full stable generic JSON parity")
        finally:
            for process in processes:
                stop_server(process)


if __name__ == "__main__":
    main()
