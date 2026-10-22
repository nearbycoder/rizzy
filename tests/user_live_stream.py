#!/usr/bin/env python3
"""Check live notification and pin updates across two Rizzy processes."""

import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from activity_event_board_parity import create_member  # noqa: E402
from card_flag_browser import browser  # noqa: E402
from draft_workflow_parity import login  # noqa: E402
from events_html_parity import target  # noqa: E402
from notification_tray_html_parity import api  # noqa: E402
from sustained_mixed import prepare_rust, rust_env, start_server, stop_server  # noqa: E402


def next_revision(response):
    for _ in range(10):
        line = response.readline()
        if not line:
            raise AssertionError("Live stream closed")
        if line.startswith(b"data: "):
            return json.loads(line[6:])
    raise AssertionError("No live revision event")


def browser_state(session):
    return json.loads(browser(session, "eval", """(() => ({
      unread:document.querySelector('[data-open-tray="notifications"]')?.classList.contains('unread'),
      pinsOpen:!!document.getElementById('pin-tray')?.open,
      pinsText:document.getElementById('pin-tray')?.textContent || ''
    }))()"""))


def until_browser(session, predicate, label):
    for _ in range(40):
        current = browser_state(session)
        if predicate(current):
            return current
        time.sleep(.2)
    raise AssertionError((label, current, browser(session, "errors")))


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-user-live-") as temporary:
        directory = Path(temporary)
        env = rust_env(directory)
        env["RIZZY_DEV_MAGIC_LINKS"] = "1"
        env, credentials = prepare_rust(directory, env)
        cpus = sorted(os.sched_getaffinity(0))[:2]
        processes = []
        session = f"rizzy-user-live-{os.getpid()}"
        try:
            for index in range(2):
                worker_directory = directory / f"process-{index}"
                worker_directory.mkdir()
                process, url = start_server("rizzy", worker_directory, env, cpus)
                processes.append(process)
                if index == 0:
                    first_url = url
                else:
                    second_url = url
            account = str(credentials["account"])
            database = str(directory / "rizzy.sqlite3")
            owner = target("rizzy", second_url, account, credentials["token"],
                           "owner@example.com", database)
            board = api(owner, "/boards", "POST",
                        {"board": {"name": "Live tray", "all_access": False}})
            member_id, member_token = create_member(owner, board["id"])
            member_email = f"activity-{member_token}@example.test"
            member = target("rizzy", first_url, account, member_token, member_email, database)
            member_other = {**member, "url": second_url}
            first_card = api(owner, f"/boards/{board['id']}/cards", "POST",
                             {"card": {"title": "First live card"}})
            login(member)
            live_request = urllib.request.Request(first_url + f"/{account}/my/live",
                                                  headers={"Accept": "text/event-stream"})
            with member["opener"].open(live_request, timeout=12) as stream:
                assert stream.headers.get_content_type() == "text/event-stream"
                initial = next_revision(stream)
                api(member_other, f"/cards/{first_card['number']}/pin", "POST")
                pinned = next_revision(stream)
                assert pinned["pins"] != initial["pins"]
                api(member_other, f"/boards/{board['id']}/involvement", "PUT",
                    {"involvement": "watching"})
                api(owner, f"/boards/{board['id']}/cards", "POST",
                    {"card": {"title": "Second live card"}})
                notified = next_revision(stream)
                assert notified["notifications"] != pinned["notifications"]
                api(member_other, f"/cards/{first_card['number']}/pin", "DELETE")
                unpinned = next_revision(stream)
                assert unpinned["pins"] != notified["pins"]

            browser(session, "open", first_url + "/session/new")
            for cookie in member["cookies"]:
                browser(session, "cookies", "set", cookie.name, cookie.value,
                        "--url", first_url, "--path", cookie.path, "--httpOnly")
            browser(session, "open", first_url + f"/{account}")
            until_browser(session, lambda state: state["unread"], "initial unread badge")
            api(member_other, "/notifications/bulk_reading", "POST")
            until_browser(session, lambda state: not state["unread"], "remote clear updates badge")
            api(owner, f"/boards/{board['id']}/cards", "POST",
                {"card": {"title": "Third live card"}})
            until_browser(session, lambda state: state["unread"], "remote event updates badge")
            browser(session, "click", '[data-open-tray="pins"]')
            until_browser(session, lambda state: state["pinsOpen"], "open pin tray")
            api(member_other, f"/cards/{first_card['number']}/pin", "POST")
            until_browser(session, lambda state: "First live card" in state["pinsText"],
                          "remote pin updates open tray")
            assert not browser(session, "errors"), "Browser page errors"
            print("PASS cross-process live SSE and open-page notification/pin refresh")
        finally:
            try:
                browser(session, "close")
            except subprocess.CalledProcessError:
                pass
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
