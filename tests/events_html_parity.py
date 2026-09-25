#!/usr/bin/env python3
"""Check browser activity timeline behavior against pinned Fizzy."""
import http.cookiejar
import datetime
from html.parser import HTMLParser
import json
import os
import re
import sqlite3
import urllib.parse
import urllib.request
import zoneinfo

from draft_workflow_parity import NoRedirect, login, request


class EventPositions(HTMLParser):
    def __init__(self):
        super().__init__()
        self.depth = 0
        self.blocks = []
        self.events = {}

    def handle_starttag(self, tag, attrs):
        fields = dict(attrs)
        if tag == "div" and "events__time-block" in fields.get("class", ""):
            position = re.search(r"grid-area:\s*(\d+)/(\d+)", fields.get("style", ""))
            if position:
                self.blocks.append((self.depth, int(position.group(1))))
        if tag == "a" and "event " in fields.get("class", "") and self.blocks:
            self.events[fields.get("id")] = self.blocks[-1][1]
        if tag not in ("br", "hr", "img", "input", "meta", "link"):
            self.depth += 1

    def handle_endtag(self, tag):
        if tag not in ("br", "hr", "img", "input", "meta", "link"):
            self.depth -= 1
        if tag == "div" and self.blocks and self.blocks[-1][0] == self.depth:
            self.blocks.pop()


def positions(body):
    parsed = EventPositions()
    parsed.feed(body.decode())
    return parsed.events


def target(name, url, account, token, email, db):
    jar = http.cookiejar.CookieJar()
    item = {"name": name, "url": url, "account": account,
            "token": token, "email": email, "db": db,
            "cookies": jar,
            "opener": urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar), NoRedirect())}
    if name == "fizzy":
        item["extra_headers"] = {"X-Forwarded-For": "198.51.100.227"}
    return item


def set_timezone(target, name):
    cookie = http.cookiejar.Cookie(version=0, name="timezone", value=name,
        port=None, port_specified=False, domain="127.0.0.1", domain_specified=False,
        domain_initial_dot=False, path="/", path_specified=True, secure=False,
        expires=None, discard=True, comment=None, comment_url=None, rest={}, rfc2109=False)
    target["cookies"].set_cookie(cookie)


def check(target, board_id, board_name, date):
    login(target)
    account = target["account"]
    status, _, home = request(target, f"/{account}/", accept="text/html")
    assert status == 200 and b"events__day" in home, (target["name"], status)
    path = f"/{account}/events?day={date}&board_ids%5B%5D={urllib.parse.quote(board_id)}"
    status, _, body = request(target, path, accept="text/html")
    assert status == 200, (target["name"], status, body[:200])
    assert b"events__columns" in body and b"events__time-block" in body, target["name"]
    assert board_name.encode() in body, target["name"]
    assert body.count(b'class="events__column"') == 3, target["name"]
    assert b"grid-area:" in body, target["name"]
    original_positions = positions(body)
    set_timezone(target, "America/New_York")
    status, _, eastern = request(target, path, accept="text/html")
    target["cookies"].clear("127.0.0.1", "/", "timezone")
    assert status == 200, (target["name"], status)
    eastern_positions = positions(eastern)
    shared = original_positions.keys() & eastern_positions.keys()
    assert shared, (target["name"], "no events shared between timezones")
    noon = datetime.datetime.fromisoformat(date + "T12:00:00+00:00")
    offset = -int(noon.astimezone(zoneinfo.ZoneInfo("America/New_York")).utcoffset().total_seconds() / 3600)
    assert all(eastern_positions[key] == original_positions[key] + offset for key in shared), target["name"]
    for part in re.findall(rb'class="event [^"]*"[^>]*>(.*?)</a>', body, re.DOTALL):
        assert board_name.encode() in part, target["name"]
    for suffix in ("days", "day_timeline/columns/added", "day_timeline/columns/updated", "day_timeline/columns/closed"):
        status, _, rendered = request(target, f"/{account}/events/{suffix}?day={date}&board_ids%5B%5D={urllib.parse.quote(board_id)}", accept="text/html")
        assert status == 200, (target["name"], suffix, status)
        if suffix == "days":
            assert b"events__day" in rendered, target["name"]
    for path in (f"/{account}/events?day=garbage", f"/{account}/events/day_timeline/columns/nope?day={date}"):
        status, _, _ = request(target, path, accept="text/html")
        assert status == 404, (target["name"], path, status)
    following = (datetime.date.fromisoformat(date) + datetime.timedelta(days=1)).isoformat()
    status, _, future = request(target, f"/{account}/events/days?day={following}&board_ids%5B%5D={urllib.parse.quote(board_id)}", accept="text/html")
    assert status == 200 and b"day-timeline-pagination-link" in future and f"day={date}".encode() in future, target["name"]
    return {"home": True, "filtered_day": True, "three_columns": True, "timezone": True,
            "day_fragment": True, "column_routes": True, "invalid_day": True,
            "invalid_column": True, "next_day": True}


def main():
    creds = json.load(open("/tmp/rizzy-fizzy-fresh-creds.json"))
    fizzy_db = "upstream/storage/production.sqlite3"
    with sqlite3.connect(fizzy_db) as db:
        source_email = db.execute("SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1", (creds["account"],)).fetchone()[0]
        source_board = db.execute("SELECT b.id,b.name,date(max(e.created_at)) FROM boards b JOIN events e ON e.board_id=b.id WHERE b.account_id=(SELECT id FROM accounts WHERE external_account_id=?) GROUP BY b.id ORDER BY max(e.created_at) DESC,count(*) DESC LIMIT 1", (creds["account"],)).fetchone()
        number = int.from_bytes(source_board[0], "big")
        alphabet = "0123456789abcdefghijklmnopqrstuvwxyz"
        digits = []
        while number:
            number, digit = divmod(number, 36)
            digits.append(alphabet[digit])
        source_board = ("".join(reversed(digits)).rjust(25, "0"), source_board[1], source_board[2])
    rust_db = open("/tmp/rizzy-filter-active-dir").read().strip() + "/rizzy.sqlite3"
    with sqlite3.connect(rust_db) as db:
        rust_account = str(db.execute("SELECT external_account_id FROM accounts LIMIT 1").fetchone()[0])
        rust_board = db.execute("SELECT b.id,b.name,date(max(e.created_at)) FROM boards b JOIN events e ON e.board_id=b.id GROUP BY b.id ORDER BY max(e.created_at) DESC,count(*) DESC LIMIT 1").fetchone()
    seed = open(os.path.dirname(rust_db) + "/seed.txt").read()
    rust_token = re.search(r"Bearer token:\s*(\S+)", seed).group(1)
    source = target("fizzy", os.environ.get("FIZZY_URL", "http://127.0.0.1:3006"), str(creds["account"]), creds["token"], source_email, fizzy_db)
    rust = target("rizzy", os.environ.get("RIZZY_URL", "http://127.0.0.1:3030"), rust_account, rust_token, "filter-verify@example.test", rust_db)
    a = check(source, *source_board)
    b = check(rust, *rust_board)
    assert a == b
    print("Activity timeline parity: account home, filtered day, day fragment, columns, invalid paths")


if __name__ == "__main__":
    main()
