#!/usr/bin/env python3
"""Compare populated webhook delivery rows on fresh Fizzy and Rizzy apps."""

import http.cookiejar
import json
import os
import re
import sqlite3
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)
from webhook_delinquency_timestamp_parity import source_runner  # noqa: E402


def api(target, path, method="GET", payload=None, expected=200):
    status, _, body = request(target, f"/{target['account']}{path}", method,
                              payload, bearer=True)
    assert status == expected, (target["name"], method, path, status, body[:200])
    return json.loads(body) if body else None


def update_delivery(target, delivery_id, state, code):
    response = json.dumps({"code": code}) if code is not None else None
    if target["name"] == "fizzy":
        source_runner(target, "Webhook::Delivery.find(" + json.dumps(delivery_id) +
                      ").update_columns(state: " + json.dumps(state) +
                      ", response: " + (f'{{ code: {code} }}' if code is not None else "nil") + ")")
    else:
        with sqlite3.connect(target["db"]) as db:
            db.execute("UPDATE webhook_deliveries SET state=?,response=? WHERE id=?",
                       (state, response, delivery_id))


def row(target, path, delivery_id):
    status, _, body = request(target, f"/{target['account']}{path}", accept="text/html")
    assert status == 200, (target["name"], "show", status)
    match = re.search(rb'<li id="webhook_delivery_' + delivery_id.encode() +
                      rb'" class="([^"]*)">(.*?)</li>', body, re.S)
    assert match, (target["name"], "missing delivery row", body[-1000:])
    assert re.search(rb'<time datetime="\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ">'
                     rb'less than a minute</time>', match[2]), (target["name"], match[2])
    return {"class": match[1].decode(),
            "state": re.search(rb'<strong>\s*(\w+)\s*</strong>', match[2]).group(1).decode(),
            "subtle": b'class="txt-subtle"' in match[2]}


def check(target):
    login(target)
    board = api(target, "/boards", "POST", {"board": {"name": "Delivery HTML"}}, 201)
    board_id = board["id"]
    try:
        webhook = api(target, f"/boards/{board_id}/webhooks", "POST", {
            "webhook": {"name": "Delivery HTML", "url": "https://example.com/hook",
                        "subscribed_actions": ["card_published"]}}, 201)
        webhook_id = webhook["id"]
        api(target, f"/boards/{board_id}/cards", "POST",
            {"card": {"title": "Triggers delivery"}}, 201)
        if target["name"] == "fizzy":
            source_runner(target, 'Event::WebhookDispatchJob.perform_now('
                          'Event.where(board_id: Webhook.find(' + json.dumps(webhook_id) +
                          ').board_id).order(created_at: :desc).first)')
        deliveries = api(target, f"/boards/{board_id}/webhooks/{webhook_id}/deliveries")
        assert len(deliveries) == 1, (target["name"], deliveries)
        delivery_id = deliveries[0]["id"]
        path = f"/boards/{board_id}/webhooks/{webhook_id}"
        results = []
        for state, code in (("pending", None), ("completed", 200),
                            ("completed", 500), ("errored", None)):
            update_delivery(target, delivery_id, state, code)
            actual = row(target, path, delivery_id)
            assert actual["state"] == state, (target["name"], actual)
            assert actual["class"] == ("delivery--succeeded" if code == 200 else ""), (
                target["name"], actual)
            assert actual["subtle"], (target["name"], actual)
            results.append(actual)
        return results
    finally:
        api(target, f"/boards/{board_id}", "DELETE", expected=204)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-webhook-delivery-html-") as temporary:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env, credentials = prepare_source(directory, 2)
                else:
                    env = rust_env(directory)
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                    env, credentials = prepare_rust(directory, env)
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url, "env": env,
                          "account": str(credentials["account"]),
                          "token": credentials["token"], "email": "owner@example.com",
                          "db": directory / ("db.sqlite3" if name == "fizzy"
                                             else "rizzy.sqlite3"),
                          "opener": urllib.request.build_opener(
                              urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                              NoRedirect())}
                results[name] = check(target)
            assert results["fizzy"] == results["rizzy"], results
            print("PASS webhook delivery HTML pending, successful, failed, and errored states")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
