#!/usr/bin/env python3
"""Compare the user activity frame with Fizzy's browser route."""
import datetime
import http.cookiejar
import json
import os
import sqlite3
import time
import urllib.request

from draft_workflow_parity import NoRedirect, login, request


def base36(raw):
    number = int.from_bytes(raw, "big")
    alphabet = "0123456789abcdefghijklmnopqrstuvwxyz"
    digits = []
    while number:
        number, digit = divmod(number, 36)
        digits.append(alphabet[digit])
    return "".join(reversed(digits)).rjust(25, "0")


def target(name):
    account = os.environ[f"{name.upper()}_ACCOUNT"]
    db_path = os.environ[f"{name.upper()}_DB"]
    with sqlite3.connect(db_path) as db:
        row = db.execute("SELECT u.id,i.email_address FROM users u JOIN identities i ON i.id=u.identity_id JOIN accounts a ON a.id=u.account_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1", (int(account),)).fetchone()
    user_id = base36(row[0]) if isinstance(row[0], bytes) else row[0]
    item = {"name": name, "url": os.environ[f"{name.upper()}_URL"],
        "account": account, "token": os.environ[f"{name.upper()}_TOKEN"],
        "email": row[1], "db": db_path, "user_id": user_id,
        "opener": urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
    if name == "fizzy":
        item["extra_headers"] = {"X-Forwarded-For": f"198.51.100.{30 + time.time_ns() % 200}"}
    return item


def check(target):
    account = target["account"]
    title = f"User event parity {time.time_ns()}"
    status, _, raw = request(target, f"/{account}/boards", "POST", {"board": {"name": title}}, bearer=True)
    assert status == 201, (target["name"], status)
    board_id = json.loads(raw)["id"]
    try:
        status, _, raw = request(target, f"/{account}/boards/{board_id}/cards", "POST", {"card": {"title": title}}, bearer=True)
        assert status == 201, (target["name"], status)
        login(target)
        path = f"/{account}/users/{target['user_id']}/events"
        status, _, body = request(target, path, accept="text/html")
        assert status == 200 and b'id="user_events"' in body and b"What have you been up to?" in body, target["name"]
        assert title.encode() in body and b"events__time-block" in body, target["name"]
        for day in ("garbage", "2026-99-99"):
            status, _, _ = request(target, path + f"?day={day}", accept="text/html")
            assert status == 404, (target["name"], day, status)
        status, _, _ = request(target, path + "?day%5B%5D=garbage", accept="text/html")
        assert status == 404, (target["name"], "day[]", status)
        status, _, _ = request(target, f"/{account}/users/no-such-user/events", accept="text/html")
        assert status == 404, (target["name"], "user", status)
        tomorrow = (datetime.datetime.now(datetime.timezone.utc).date() + datetime.timedelta(days=1)).isoformat()
        status, _, future = request(target, path + f"?day={tomorrow}", accept="text/html")
        assert status == 200 and b"day-timeline-pagination-link" in future, target["name"]
        assert path.encode() in future, target["name"]
        return {"self": True, "activity": True, "invalid_day": True,
                "invalid_array_day": True, "invalid_user": True, "next_day": True}
    finally:
        request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)


def main():
    left, right = check(target("fizzy")), check(target("rizzy"))
    assert left == right
    print("User activity frame matched: self heading, event, invalid inputs, and next day")


if __name__ == "__main__":
    main()
