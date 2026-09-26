#!/usr/bin/env python3
"""Compare saved-filter activity timelines with the pinned Fizzy server."""
import json
import os
import re
import sqlite3
import subprocess
import tempfile
import time
import urllib.parse
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from draft_workflow_parity import login, request
from events_html_parity import target


def conditional(item, path, etag):
    original = item.get("extra_headers", {}).copy()
    item["extra_headers"] = {**original, "If-None-Match": etag}
    try:
        return request(item, path, accept="text/html")
    finally:
        if original:
            item["extra_headers"] = original
        else:
            item.pop("extra_headers", None)


def run(item, mutation_url=None):
    account = item["account"]
    boards = []
    filter_id = None
    login(item)
    try:
        titles = [f"Timeline saved filter {side} {time.time_ns()}" for side in "AB"]
        for side in "AB":
            status, _, body = request(item, f"/{account}/boards", "POST", {
                "board": {"name": f"Timeline filter {side} {time.time_ns()}"}
            }, bearer=True)
            assert status == 201, (item["name"], status, body[:200])
            boards.append(json.loads(body))
        cards = []
        for board, title in zip(boards, titles):
            status, _, body = request(item, f"/{account}/boards/{board['id']}/cards", "POST", {
                "card": {"title": title}
            }, bearer=True)
            assert status == 201, (item["name"], status, body[:200])
            cards.append(json.loads(body))
        day = cards[0]["created_at"][:10]
        status, _, html = request(item, f"/{account}/events", accept="text/html")
        assert status == 200, (item["name"], status)
        match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', html)
        csrf = match.group(1).decode() if match else None
        fields = urllib.parse.urlencode({
            "board_ids[]": boards[0]["id"],
            "creator_ids[]": cards[0]["creator"]["id"],
        }).encode()
        status, _, body = request(item, f"/{account}/filters", "POST", fields,
                                  accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200, (item["name"], status, body[:300])
        match = re.search(rb'/filters/([a-zA-Z0-9_-]+)', body)
        assert match, (item["name"], body[:400])
        filter_id = match.group(1).decode()
        suffix = urllib.parse.urlencode({"day": day, "filter_id": filter_id,
                                         "board_ids[]": boards[1]["id"]})
        etags = {}
        for route in ("events", "events/days", "events/day_timeline/columns/added",
                      "events/day_timeline/columns/updated", "events/day_timeline/columns/closed"):
            path = f"/{account}/{route}?{suffix}"
            status, headers, html = request(item, path, accept="text/html")
            assert status == 200, (item["name"], route, status, html[:200])
            etags[route] = headers["ETag"]
            assert headers["Cache-Control"] == "max-age=0, private, must-revalidate"
            cached_status, cached_headers, cached_body = conditional(item, path, etags[route])
            assert cached_status == 304 and cached_headers["ETag"] == etags[route] and not cached_body
            if route in ("events", "events/days", "events/day_timeline/columns/added"):
                assert titles[0].encode() in html and titles[1].encode() not in html, (item["name"], route)
            if route == "events":
                assert b"Activity in" in html, (item["name"], "filtered heading")
        assert len({etags["events"], etags["events/days"],
                    etags["events/day_timeline/columns/added"]}) == 3
        assert etags["events/day_timeline/columns/added"] == etags["events/day_timeline/columns/updated"]
        assert etags["events/day_timeline/columns/added"] == etags["events/day_timeline/columns/closed"]
        equivalent = urllib.parse.urlencode({
            "day": day, "board_ids[]": boards[0]["id"],
            "creator_ids[]": cards[0]["creator"]["id"],
        })
        status, equivalent_headers, _ = request(item, f"/{account}/events?{equivalent}", accept="text/html")
        assert status == 200 and equivalent_headers["ETag"] == etags["events"], (
            item["name"], "equivalent filter ETag", equivalent_headers.get("ETag"), etags["events"])
        different = equivalent + "&" + urllib.parse.urlencode({"terms[]": "nonmatching filter term"})
        status, different_headers, different_body = request(
            item, f"/{account}/events?{different}", accept="text/html")
        assert status == 200 and titles[0].encode() in different_body
        assert different_headers["ETag"] != etags["events"], (item["name"], "distinct filter ETag")
        user_path = f"/{account}/users/{cards[0]['creator']['id']}/events?day={day}"
        status, user_headers, user_body = request(item, user_path, accept="text/html")
        assert status == 200 and titles[0].encode() in user_body
        user_etag = user_headers["ETag"]
        status, _, body = conditional(item, user_path, user_etag)
        assert status == 304 and not body
        new_title = f"Saved timeline invalidation {time.time_ns()}"
        mutation_target = {**item, "url": mutation_url} if mutation_url else item
        status, _, body = request(mutation_target, f"/{account}/boards/{boards[0]['id']}/cards", "POST", {
            "card": {"title": new_title}
        }, bearer=True)
        assert status == 201, (item["name"], status, body[:200])
        status, changed_headers, changed_body = conditional(item, f"/{account}/events?{suffix}", etags["events"])
        assert status == 200 and changed_headers["ETag"] != etags["events"]
        assert new_title.encode() in changed_body
        status, changed_headers, changed_body = conditional(item, user_path, user_etag)
        assert status == 200 and changed_headers["ETag"] != user_etag
        assert new_title.encode() in changed_body
        renamed = f"Timeline renamed {time.time_ns()}"
        status, _, body = request(item, f"/{account}/boards/{boards[0]['id']}", "PUT", {
            "board": {"name": renamed}
        }, bearer=True)
        assert status == 200, (item["name"], "rename", status, body[:200])
        time.sleep(1.1)
        status, _, refreshed = request(item, f"/{account}/events?{suffix}", accept="text/html")
        assert status == 200 and renamed.encode() in refreshed, (item["name"], "rendered board rename")
        status, _, _ = request(item, f"/{account}/events?filter_id=missing", accept="text/html")
        assert status == 404, (item["name"], "missing filter", status)
        return {"saved_scope": True, "query_override": True, "day_and_column": True,
                "unknown_filter": True, "conditional": True, "invalidation": True,
                "refreshed_detail": True}
    finally:
        if filter_id:
            request(item, f"/{account}/filters/{filter_id}", "DELETE", b"",
                    accept="text/vnd.turbo-stream.html")
        for board in boards:
            request(item, f"/{account}/boards/{board['id']}", "DELETE", bearer=True)


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    source_creds = json.loads(Path("/tmp/rizzy-fizzy-fresh-creds.json").read_text())
    source_db = ROOT / "upstream/storage/production.sqlite3"
    with sqlite3.connect(source_db) as db:
        source_email = db.execute(
            "SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id "
            "WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
            (source_creds["account"],),
        ).fetchone()[0]
    source = target("fizzy", os.environ.get("FIZZY_URL", "http://127.0.0.1:3006"),
                    str(source_creds["account"]), source_creds["token"],
                    source_email, str(source_db))
    with tempfile.TemporaryDirectory() as temp:
        rust_db = Path(temp) / "rizzy.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{rust_db}",
               "RIZZY_SEED_EMAIL": "saved-timeline@example.test"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded.stdout).group(1)
        port, server = start(env)
        try:
            other_port, other_server = start(env)
            try:
                rust = target("rizzy", f"http://127.0.0.1:{port}", account, token,
                              env["RIZZY_SEED_EMAIL"], str(rust_db))
                assert run(source) == run(rust, f"http://127.0.0.1:{other_port}")
            finally:
                stop(other_server)
        finally:
            stop(server)
    print("Activity parity: saved filter scope, conditional HTML, event invalidation, and missing filter")


if __name__ == "__main__":
    main()
