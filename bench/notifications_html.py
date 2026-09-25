#!/usr/bin/env python3
"""Compare signed-in notification HTML after generating equal unread card events."""
import argparse
import http.cookiejar
import json
import os
import sqlite3
import sys
import time
import urllib.error
import urllib.request

from compare import request
from events_html import html, run


def session_request(target, path, method="GET", payload=None):
    body = json.dumps(payload).encode() if payload is not None else None
    request_object = urllib.request.Request(
        target["url"] + path, body, method=method,
        headers={"Accept": "application/json", "Content-Type": "application/json"},
    )
    with target["opener"].open(request_object, timeout=15) as response:
        data = response.read()
        return json.loads(data) if data else None, response.headers


def join_member(target):
    invite = request(target, "/account/join_code")
    assert invite["active"] and invite["usage_count"] < invite["usage_limit"], invite
    email = f"notification-benchmark-{time.time_ns()}@example.test"
    target["member_email"] = email
    jar = http.cookiejar.CookieJar()
    target["opener"] = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    join_path = f"/{target['account']}/join/{invite['code']}"
    _, headers = session_request(target, join_path, "POST", {"email_address": email})
    code = headers.get("X-Magic-Link-Code")
    if not code:
        with sqlite3.connect(target["db"]) as db:
            code = db.execute(
                "SELECT m.code FROM magic_links m JOIN identities i ON i.id=m.identity_id "
                "WHERE i.email_address=? ORDER BY m.created_at DESC LIMIT 1", (email,),
            ).fetchone()[0]
    session_request(target, "/session/magic_link", "POST", {"code": code})
    users = request(target, "/users")
    member = next(user for user in users if user.get("email_address") == email)
    target["member_id"] = member["id"]
    cookie_request = urllib.request.Request(target["url"] + f"/{target['account']}/notifications")
    jar.add_cookie_header(cookie_request)
    target["cookie"] = cookie_request.get_header("Cookie")
    assert target["cookie"], "Member session cookie missing"


def make_fixture(target, cards):
    join_member(target)
    board = request(target, "/boards", "POST", {"board": {
        "name": f"Notification benchmark {time.time_ns()}", "all_access": True}})
    target["board_id"] = board["id"]
    session_request(target, f"/{target['account']}/boards/{board['id']}/involvement", "PUT",
                    {"involvement": "watching"})
    for index in range(cards):
        request(target, f"/boards/{board['id']}/cards", "POST", {
            "card": {"title": f"Notification benchmark card {index}"}})
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        notices, _ = session_request(target, f"/{target['account']}/notifications")
        if len(notices) == cards:
            break
        time.sleep(.2)
    else:
        raise AssertionError((target["name"], "notifications", len(notices), cards))
    assert all(not notice["read"] for notice in notices)
    sample = html(target, "/notifications")
    visible = sample.count(b'class="tray__item tray__item--notification"')
    assert visible == cards, (target["name"], visible, cards)
    assert b"Notification benchmark card 0" in sample and b"New for you" in sample
    for _ in range(5):
        html(target, "/notifications")
    return {"unread": len(notices), "visible": visible, "html_bytes": len(sample)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--concurrency", default="1,8,32,64")
    args = parser.parse_args()
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        keys = [f"{name}_{field}" for field in ("URL", "ACCOUNT", "TOKEN", "DB")]
        if any(not os.getenv(key) for key in keys):
            parser.error("Set " + ", ".join(keys))
        targets[name.lower()] = {"name": name.lower(), **{
            field.lower(): os.environ[f"{name}_{field}"]
            for field in ("URL", "ACCOUNT", "TOKEN", "DB")}}
    results = {"fixture_cards": args.cards, "sample_parity": {}, "runs": {}}
    try:
        for name, target in targets.items():
            results["sample_parity"][name] = make_fixture(target, args.cards)
        for concurrency in map(int, args.concurrency.split(",")):
            for name, target in targets.items():
                result = run(target, "/notifications", concurrency, args.seconds)
                assert not result["errors"], (name, result)
                results["runs"].setdefault(name, []).append(result)
                print(f"{name} c={concurrency}: {result}", file=sys.stderr, flush=True)
        for fizzy, rust in zip(results["runs"]["fizzy"], results["runs"]["rizzy"]):
            results.setdefault("comparison", []).append({
                "concurrency": fizzy["concurrency"],
                "rps_ratio_rizzy_over_fizzy": round(rust["rps"] / fizzy["rps"], 3),
                "p95_ratio_fizzy_over_rizzy": round(fizzy["p95_ms"] / rust["p95_ms"], 3)})
        print(json.dumps(results, indent=2))
    finally:
        for name, target in targets.items():
            if board := target.get("board_id"):
                request(target, f"/boards/{board}", method="DELETE")
            if member := target.get("member_id"):
                request(target, f"/users/{member}", method="DELETE")


if __name__ == "__main__":
    main()
