#!/usr/bin/env python3
"""Compare a populated browser notification tray and read controls with Fizzy."""

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench"))

from activity_event_board_parity import create_member
from draft_workflow_parity import login, request
from events_html_parity import target
from sustained_mixed import ROOT, SOURCE, prepare_rust, prepare_source, start_server, stop_server


def api(item, path, method="GET", payload=None):
    status, _, body = request(item, f"/{item['account']}{path}", method, payload, bearer=True)
    assert status in (200, 201, 204), (item["name"], method, path, status, body[:160])
    return json.loads(body) if body else None


def frame(item, suffix=""):
    status, _, body = request(item, f"/{item['account']}/notifications/tray{suffix}", accept="text/html")
    assert status == 200 and b'<turbo-frame id="notifications">' in body, (item["name"], status)
    return body


def check(owner, env):
    board = api(owner, "/boards", "POST", {"board": {"name": "Tray parity", "all_access": False}})
    member_id, member_token = create_member(owner, board["id"])
    if owner["name"] == "fizzy":
        with sqlite3.connect(owner["db"]) as db:
            account_id, created_at, updated_at = db.execute(
                "SELECT account_id,created_at,updated_at FROM users WHERE id=?", (member_id,)).fetchone()
            db.execute("INSERT INTO user_settings(id,account_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?)",
                       (uuid.uuid4().bytes, account_id, member_id, created_at, updated_at))
    member = target(owner["name"], owner["url"], owner["account"], member_token,
                    f"activity-{member_token}@example.test", owner["db"])
    api(member, f"/boards/{board['id']}/involvement", "PUT", {"involvement": "watching"})
    api(owner, f"/boards/{board['id']}/cards", "POST", {"card": {"title": "A populated tray"}})
    if owner["name"] == "fizzy" and not api(member, "/notifications/tray"):
        job = subprocess.run(["bin/rails", "runner", "Event.order(created_at: :desc).first.notify_recipients"],
                             cwd=SOURCE, env=env, capture_output=True, text=True)
        assert job.returncode == 0, job.stderr[-2000:]
    notices = api(member, "/notifications/tray")
    assert len(notices) == 1 and notices[0]["title"] == "A populated tray", (owner["name"], notices)
    notice = notices[0]
    login(member)
    html = frame(member)
    expected = (
        b'class="tray__item tray__item--notification"',
        b'class="card card--notification unread"',
        b'class="card__notification-meta',
        b'class="avatar txt-x-small"',
        b'card__notification-unread-indicator',
        b'Mark as read',
        b'A populated tray',
        b'Added by Owner',
    )
    assert all(marker in html for marker in expected), (owner["name"], [m for m in expected if m not in html])
    assert html.count(b'class="tray__item tray__item--notification"') == 1, owner["name"]
    read_path = f"/notifications/{notice['id']}/reading"
    api(member, read_path, "POST")
    assert b'tray__item--notification' not in frame(member), owner["name"]
    read_html = frame(member, "?include_read=true")
    assert b'A populated tray' in read_html and b'Mark as unread' in read_html, owner["name"]
    api(member, read_path, "DELETE")
    assert b'A populated tray' in frame(member), owner["name"]
    status, _, body = request(member, f"/{member['account']}/notifications/bulk_reading?from_tray=true",
                              "POST", b"", accept="text/html")
    assert status == 200 and not body, (owner["name"], status, body[:160])
    assert b'tray__item--notification' not in frame(member), owner["name"]
    return {"event": notice["title"], "initial": 1, "read": 0, "include_read": 1, "bulk_read": 0}


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-notification-tray-") as temp:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temp) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                if name == "rizzy":
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                owner = target(name, url, str(credentials["account"]), credentials["token"],
                               "owner@example.com", str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")))
                results[name] = check(owner, env)
            assert results["fizzy"] == results["rizzy"], results
            print("PASS populated notification tray HTML and read transitions", results["rizzy"])
        finally:
            for process in processes:
                stop_server(process)


if __name__ == "__main__":
    main()
