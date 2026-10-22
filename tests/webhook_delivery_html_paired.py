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
from datetime import datetime, timedelta, timezone
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


def update_request(target, delivery_id):
    request_data = {"headers": {"User-Agent": "fizzy/1.0.0 Webhook",
                                "X-Webhook-Signature": "private signature"},
                    "body": "private request body"}
    if target["name"] == "fizzy":
        source_runner(target, "Webhook::Delivery.find(" + json.dumps(delivery_id) +
                      ').update_columns(request: { headers: { "User-Agent" => '
                      '"fizzy/1.0.0 Webhook", "X-Webhook-Signature" => '
                      '"private signature" }, body: "private request body" }, '
                      'updated_at: Time.current)')
    else:
        with sqlite3.connect(target["db"]) as db:
            db.execute("UPDATE webhook_deliveries SET request=? WHERE id=?",
                       (json.dumps(request_data), delivery_id))


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


def older_delivery_age(target, path, delivery_id, days, expected):
    if target["name"] == "fizzy":
        source_runner(target, "Webhook::Delivery.find(" + json.dumps(delivery_id) +
                      f").update_columns(created_at: {days}.days.ago)")
    else:
        timestamp = (datetime.now(timezone.utc) - timedelta(days=days)).isoformat(
            timespec="milliseconds").replace("+00:00", "Z")
        with sqlite3.connect(target["db"]) as db:
            db.execute("UPDATE webhook_deliveries SET created_at=? WHERE id=?",
                       (timestamp, delivery_id))
    status, _, body = request(target, f"/{target['account']}{path}", accept="text/html")
    assert status == 200, (target["name"], "old delivery", status)
    match = re.search(rb'<li id="webhook_delivery_' + delivery_id.encode() +
                      rb'" class="[^"]*">(.*?)</li>', body, re.S)
    assert match, (target["name"], "missing old delivery")
    label = re.search(rb'<time datetime="[^"]+">([^<]+)</time>', match[1]).group(1).decode()
    assert label == expected, (target["name"], days, label, expected, match[1][-200:])


def check(target):
    login(target)
    board = api(target, "/boards", "POST", {"board": {"name": "Delivery HTML"}}, 201)
    board_id = board["id"]
    try:
        webhook = api(target, f"/boards/{board_id}/webhooks", "POST", {
            "webhook": {"name": "Delivery HTML", "url": "https://example.com/hook",
                        "subscribed_actions": ["card_published", "comment_created"]}}, 201)
        webhook_id = webhook["id"]
        card = api(target, f"/boards/{board_id}/cards", "POST",
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
        update_request(target, delivery_id)
        delivery = api(target, path + "/deliveries")[0]
        assert delivery["request"] == {"headers": {"User-Agent": "fizzy/1.0.0 Webhook"}}, (
            target["name"], delivery["request"])
        comment = api(target, f"/cards/{card['number']}/comments", "POST",
                      {"comment": {"body": "Webhook comment"}}, 201)
        if target["name"] == "fizzy":
            source_runner(target, 'Event::WebhookDispatchJob.perform_now('
                          'Event.where(board_id: Webhook.find(' + json.dumps(webhook_id) +
                          ').board_id, action: "comment_created").order(created_at: :desc).first)')
        deliveries = api(target, path + "/deliveries")
        assert len(deliveries) == 2, (target["name"], deliveries)
        comment_delivery = next(item for item in deliveries if item["event"]["action"] ==
                                "comment_created")
        event = comment_delivery["event"]
        assert comment_delivery["state"] == "pending", (target["name"], comment_delivery)
        assert comment_delivery["request"] is None and comment_delivery["response"] is None
        assert event["creator"]["name"] == "Owner", (target["name"], event)
        assert event["eventable"]["type"] == "Comment" and event["eventable"]["id"] == comment["id"]
        assert event["eventable"]["url"].endswith(
            f"/{target['account']}/cards/{card['number']}#comment_{comment['id']}"), (
                target["name"], event)
        for days, label in ((50, "about 2 months"), (400, "about 1 year"),
                            (600, "over 1 year"), (700, "almost 2 years")):
            older_delivery_age(target, path, delivery_id, days, label)
        for index in range(22):
            api(target, f"/boards/{board_id}/cards", "POST",
                {"card": {"title": f"Delivery page {index:02d}"}}, 201)
        if target["name"] == "fizzy":
            quoted_id = json.dumps(webhook_id)
            source_runner(target, 'webhook = Webhook.find(' + quoted_id + '); '
                          'Event.where(board_id: webhook.board_id, action: "card_published")'
                          '.where.not(id: Webhook::Delivery.where(webhook_id: webhook.id)'
                          '.select(:event_id)).find_each { |event| '
                          'Event::WebhookDispatchJob.perform_now(event) }')
        pages = []
        for page_number, expected_count in ((1, 15), (2, 9)):
            status, headers, raw = request(
                target, f"/{target['account']}{path}/deliveries?page={page_number}",
                bearer=True)
            assert status == 200 and int(headers["X-Total-Count"]) == 24, (
                target["name"], page_number, status, headers)
            link = headers.get("Link")
            assert (link is not None and 'rel="next"' in link) == (page_number == 1), (
                target["name"], page_number, link)
            page_rows = json.loads(raw)
            assert len(page_rows) == expected_count, (target["name"], page_number, len(page_rows))
            pages.append(page_rows)
        all_rows = pages[0] + pages[1]
        assert len({item["id"] for item in all_rows}) == 24, (target["name"], "duplicate delivery")
        assert [item["created_at"] for item in all_rows] == sorted(
            (item["created_at"] for item in all_rows), reverse=True), (
                target["name"], "delivery order")
        status, _, body = request(target, f"/{target['account']}{path}", accept="text/html")
        assert status == 200 and body.count(b'<li id="webhook_delivery_') == 20, (
            target["name"], "HTML delivery limit", status)
        return {"states": results, "comment_event": event["action"],
                "page_sizes": [len(page) for page in pages], "html_limit": 20}
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
            print("PASS webhook delivery states, JSON, older times, and pagination")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
