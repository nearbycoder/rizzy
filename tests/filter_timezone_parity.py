#!/usr/bin/env python3
"""Compare local-date card and closure filters with production Fizzy."""

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import urllib.parse
import urllib.request
from datetime import datetime, time, timedelta, timezone
from pathlib import Path
from zoneinfo import ZoneInfo

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from events_html import browser_cookie  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402

ZONES = ("America/Los_Angeles", "Asia/Tokyo")
WINDOWS = ("today", "this week", "this month", "this year")


def request(target, path, method="GET", payload=None, cookie=None, accept="application/json"):
    body = json.dumps(payload).encode() if payload is not None else None
    headers = {"Authorization": "Bearer " + target["token"], "Accept": accept}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if cookie:
        headers["Cookie"] = cookie
    req = urllib.request.Request(target["url"] + f"/{target['account']}" + path,
                                 data=body, method=method, headers=headers)
    with urllib.request.urlopen(req, timeout=15) as response:
        raw = response.read()
        return (json.loads(raw) if raw and accept == "application/json" else raw,
                response.status)


def boundaries(zone, now):
    local = now.astimezone(zone)
    day = local.date()
    starts = (day, day - timedelta(days=day.weekday()), day.replace(day=1),
              day.replace(month=1, day=1))
    return {name: datetime.combine(start, time.min, zone)
            for name, start in zip(WINDOWS, starts)}


def fill(target, now):
    target["cookie"] = browser_cookie(target)
    boards = {}
    for zone_name in ZONES:
        zone = ZoneInfo(zone_name)
        board, status = request(target, "/boards", "POST",
                                {"board": {"name": "Timezone filter " + zone_name}})
        assert status == 201
        cards = {}
        for window, start in boundaries(zone, now).items():
            for side, stamp in (("before", start - timedelta(minutes=30)),
                                ("after", start + timedelta(minutes=30))):
                title = f"{window} {side}"
                card, status = request(target, f"/boards/{board['id']}/cards", "POST",
                                       {"card": {"title": title, "created_at":
                                                 stamp.astimezone(timezone.utc).isoformat()}})
                assert status == 201
                cards[title] = card
        for side, stamp in (("before", boundaries(zone, now)["today"] - timedelta(minutes=30)),
                            ("after", boundaries(zone, now)["today"] + timedelta(minutes=30))):
            title = "closure " + side
            card, status = request(target, f"/boards/{board['id']}/cards", "POST",
                                   {"card": {"title": title}})
            assert status == 201
            _, status = request(target, f"/cards/{card['number']}/closure", "POST")
            assert status == 204
            utc_stamp = stamp.astimezone(timezone.utc)
            formatted = (utc_stamp.strftime("%Y-%m-%d %H:%M:%S.%f") if target["name"] == "fizzy"
                         else utc_stamp.isoformat(timespec="milliseconds").replace("+00:00", "Z"))
            with sqlite3.connect(target["db"]) as db:
                if target["name"] == "fizzy":
                    updated = db.execute(
                        "UPDATE closures SET created_at=? WHERE card_id="
                        "(SELECT id FROM cards WHERE number=?)", (formatted, card["number"]))
                else:
                    updated = db.execute("UPDATE closures SET created_at=? WHERE card_id=?",
                                         (formatted, card["id"]))
                assert updated.rowcount == 1, (target["name"], title)
            cards[title] = card
        boards[zone_name] = (board["id"], cards)
    return boards


def check(target, boards, now):
    results = {}
    for zone_name, (board_id, cards) in boards.items():
        zone = ZoneInfo(zone_name)
        cookie = target["cookie"] + "; timezone=" + urllib.parse.quote(zone_name)
        for window, start in boundaries(zone, now).items():
            query = urllib.parse.urlencode({"board_ids[]": board_id, "creation": window})
            values, status = request(target, "/cards?" + query, cookie=cookie)
            assert status == 200
            titles = {card["title"] for card in values}
            end = {"today": start + timedelta(days=1),
                   "this week": start + timedelta(days=7),
                   "this month": (start.replace(day=28) + timedelta(days=4)).replace(day=1),
                   "this year": start.replace(year=start.year + 1)}[window]
            expected = {title for title, card in cards.items() if not title.startswith("closure ")
                        and start <= datetime.fromisoformat(card["created_at"].replace("Z", "+00:00"))
                        .astimezone(zone) < end}
            assert titles == expected, (target["name"], zone_name, window, titles, expected)
            results[(zone_name, window)] = sorted(titles)
        query = urllib.parse.urlencode({"board_ids[]": board_id, "closure": "today"})
        values, status = request(target, "/cards?" + query, cookie=cookie)
        assert status == 200
        titles = {card["title"] for card in values}
        assert titles == {"closure after"}, (target["name"], zone_name, titles)
        results[(zone_name, "closure today")] = sorted(titles)
        # Both the browser board and the lazy card preview path use the same filter.
        board_query = urllib.parse.urlencode({"creation": "today", "sorted_by": "oldest"})
        html, status = request(target, f"/boards/{board_id}?{board_query}",
                               cookie=cookie, accept="text/html")
        assert status == 200
        for title in results[(zone_name, "today")]:
            assert title.encode() in html, (target["name"], zone_name, title)
        for title in set(cards) - set(results[(zone_name, "today")]):
            assert title.encode() not in html, (target["name"], zone_name, title)
        previews_query = urllib.parse.urlencode({"board_ids[]": board_id, "creation": "today",
                                                 "target": "timezone-filter-results"})
        previews, status = request(target, "/cards/previews.turbo_stream?" + previews_query,
                                   cookie=cookie, accept="text/vnd.turbo-stream.html")
        assert status == 200, (target["name"], zone_name, status)
        for title in results[(zone_name, "today")]:
            assert title.encode() in previews, (target["name"], zone_name, title)
        for title in set(cards) - set(results[(zone_name, "today")]):
            assert title.encode() not in previews, (target["name"], zone_name, title)
        if zone_name == "America/Los_Angeles":
            query = urllib.parse.urlencode({"board_ids[]": board_id, "creation": "today"})
            alias = "Pacific Time (US & Canada)"
            values, status = request(target, "/cards?" + query,
                                     cookie=target["cookie"] + "; timezone=" + urllib.parse.quote(alias))
            assert status == 200
            assert {card["title"] for card in values} == set(results[(zone_name, "today")])
            utc_start = datetime.combine(now.date(), time.min, timezone.utc)
            utc_end = utc_start + timedelta(days=1)
            expected_utc = {title for title, card in cards.items()
                            if not title.startswith("closure ") and utc_start <=
                            datetime.fromisoformat(card["created_at"].replace("Z", "+00:00"))
                            < utc_end}
            values, status = request(target, "/cards?" + query,
                                     cookie=target["cookie"] + "; timezone=Not%2FAZone")
            assert status == 200
            assert {card["title"] for card in values} == expected_utc
    return results


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    now = datetime.now(timezone.utc)
    with tempfile.TemporaryDirectory(prefix="rizzy-filter-timezone-") as temporary:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                if name == "rizzy":
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url, "account": str(credentials["account"]),
                          "token": credentials["token"], "db": str(directory / (
                              "db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                          "email": "mixed-benchmark@example.com" if name == "fizzy"
                          else "owner@example.com"}
                boards = fill(target, now)
                results[name] = check(target, boards, now)
                print(f"{name}: {len(results[name])} local-date filters matched", flush=True)
        finally:
            for process in processes:
                stop_server(process)
    assert results["fizzy"] == results["rizzy"], results


if __name__ == "__main__":
    main()
