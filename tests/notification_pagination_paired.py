#!/usr/bin/env python3
"""Compare read-notification pagination and exercise automatic browser loading."""

import datetime as dt
import http.cookiejar
import os
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from card_workflow import expect  # noqa: E402
from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, rust_env, start_server, stop_server  # noqa: E402


def browser(session, *arguments):
    return subprocess.run(
        ["agent-browser", "--session", session, *arguments],
        capture_output=True, text=True, check=True,
    ).stdout.strip()


def count(session):
    return int(browser(session, "eval", "document.querySelectorAll('#notifications_list_read .tray__item').length"))


def make_fixture(name, directory):
    if name == "fizzy":
        env, credentials = prepare_source(directory, 2)
    else:
        env = rust_env(directory)
        env["RIZZY_DEV_MAGIC_LINKS"] = "1"
        env, credentials = prepare_rust(directory, env)
    process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
    jar = http.cookiejar.CookieJar()
    target = {
        "name": name, "url": url, "account": str(credentials["account"]),
        "token": credentials["token"], "email": "owner@example.com",
        "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
        "opener": urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar), NoRedirect()),
    }
    try:
        login(target)
        board, _ = expect(target, "/boards", "POST", {"board": {"name": "Read notice pages"}}, 201)
        for index in range(46):
            card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                             {"card": {"title": f"Notice card {index:02d}"}}, 201)
            card_id = (int(card["id"], 36).to_bytes(16, "big")
                       if name == "fizzy" else card["id"])
            user_id = (int(board["creator"]["id"], 36).to_bytes(16, "big")
                       if name == "fizzy" else board["creator"]["id"])
            with sqlite3.connect(target["db"]) as db:
                event = db.execute(
                    "SELECT id FROM events WHERE eventable_id=? AND action='card_published'",
                    (card_id,),
                ).fetchone()
                assert event, (name, "publication event", index)
                account_id = db.execute("SELECT account_id FROM cards WHERE id=?", (card_id,)).fetchone()[0]
                read_at = (dt.datetime(2026, 1, 1, tzinfo=dt.timezone.utc)
                           + dt.timedelta(seconds=index)).isoformat(timespec="milliseconds")
                db.execute(
                    "INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,unread_count,read_at,created_at,updated_at) "
                    "VALUES(?,?,?,?,?,?,'Event',0,?,?,?)",
                    (uuid.uuid4().bytes if name == "fizzy" else str(uuid.uuid4()),
                     account_id, user_id, user_id, card_id, event[0], read_at, read_at, read_at),
                )
        path = f"/{target['account']}/notifications"
        status, _, html = request(target, path, accept="text/html")
        assert status == 200 and html.count(b'id="notification_') == 15, (name, status)
        assert b'data-controller="fetch-on-visible"' in html
        assert f'data-fetch-on-visible-url-value="{path}?page=2"'.encode() in html
        status, _, stream = request(target, path + "?page=2", accept="text/vnd.turbo-stream.html")
        assert status == 200 and stream.count(b'id="notification_') == 30
        assert b'action="append" target="notifications_list_read"' in stream
        assert f'data-fetch-on-visible-url-value="{path}?page=3"'.encode() in stream
        return process, target, jar
    except BaseException:
        stop_server(process)
        raise


def check_browser(name, target, jar):
    session = "rizzy-notification-pagination-" + name
    path = f"/{target['account']}/notifications"
    try:
        browser(session, "open", target["url"] + "/session/new")
        for cookie in jar:
            browser(session, "cookies", "set", cookie.name, cookie.value,
                    "--url", target["url"], "--path", cookie.path,
                    "--httpOnly")
        browser(session, "open", target["url"] + path)
        assert count(session) == 15, (name, "first page")
        for expected in (45, 46):
            browser(session, "eval", "window.scrollTo(0, document.body.scrollHeight)")
            for _ in range(30):
                if count(session) >= expected:
                    break
                time.sleep(0.25)
            assert count(session) == expected, (name, "auto loaded", expected, count(session))
        assert browser(session, "get", "url") == target["url"] + path
        assert browser(session, "eval", "document.querySelector('#next_page') === null") == "true"
        assert browser(session, "eval", "new Set(Array.from(document.querySelectorAll('#notifications_list_read .tray__item'), item => item.id)).size") == "46"
        assert browser(session, "eval", "document.querySelector('#notifications_list_read').firstElementChild.textContent.includes('Notice card 45')") == "true"
        assert browser(session, "eval", "document.querySelector('#notifications_list_read').lastElementChild.textContent.includes('Notice card 00')") == "true"
        assert not browser(session, "errors")
    finally:
        browser(session, "close")


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-notification-pagination-") as temporary:
        for name in ("fizzy", "rizzy"):
            directory = Path(temporary) / name
            directory.mkdir()
            process, target, jar = make_fixture(name, directory)
            try:
                check_browser(name, target, jar)
            finally:
                stop_server(process)
    print("Matched source pagination marker and stream; both browsers loaded all 46 read notifications")


if __name__ == "__main__":
    main()
