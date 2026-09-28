#!/usr/bin/env python3
"""Compare the card assignment picker's keyboard flow in both browsers."""

import http.cookiejar
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from activity_event_board_parity import database_id  # noqa: E402
from assignment_limit_paired import add_people_and_assignments, api  # noqa: E402
from draft_workflow_parity import NoRedirect, login  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def browser(session, *arguments):
    return subprocess.run(["agent-browser", "--session", session, *arguments],
                          capture_output=True, text=True, check=True).stdout.strip()


def state(session):
    return json.loads(browser(session, "eval", """(() => {
      const dialog = [...document.querySelectorAll('dialog')].find(item => item.open &&
        item.querySelector('[data-assignment-limit-count-value]'));
      if (!dialog) return { open: false };
      const panel = dialog.querySelector('[data-assignment-limit-count-value]');
      const visible = [...panel.querySelectorAll('.popup__item')].filter(item => !item.hidden);
      const selected = panel.querySelector('.popup__item[aria-selected=true]');
      return { open: true, count: Number(panel.dataset.assignmentLimitCountValue),
        focusSearch: document.activeElement === panel.querySelector('input[type=search]'),
        selectedIndex: visible.indexOf(selected),
        selectedText: selected?.textContent.trim() || '',
        warning: !panel.querySelector('[data-assignment-limit-target=limitMessage]').hidden,
        unassignedHidden: [...panel.querySelectorAll('[data-assignment-limit-target=unassigned]')]
          .every(item => item.hidden) };
    })()"""))


def wait_for(session, predicate, label):
    for _ in range(40):
        current = state(session)
        if predicate(current):
            return current
        time.sleep(0.2)
    raise AssertionError((label, current))


def make_fixture(name, directory):
    if name == "fizzy":
        env, credentials = prepare_source(directory, 2)
    else:
        env = rust_env(directory)
        env["RIZZY_DEV_MAGIC_LINKS"] = "1"
        env, credentials = prepare_rust(directory, env)
    process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
    jar = http.cookiejar.CookieJar()
    target = {"name": name, "url": url, "account": str(credentials["account"]),
              "token": credentials["token"], "email": "owner@example.com",
              "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
              "opener": urllib.request.build_opener(
                  urllib.request.HTTPCookieProcessor(jar), NoRedirect())}
    try:
        login(target)
        board = api(target, "/boards", "POST", {"board": {"name": "Keyboard assignment", "all_access": False}}, 201)
        card = api(target, f"/boards/{board['id']}/cards", "POST",
                   {"card": {"title": "Keyboard assignment"}}, 201)
        api(target, f"/cards/{card['number']}/publish", "POST", {}, 201)
        owner = next(user["id"] for user in api(target, "/users") if user["role"] == "owner")
        api(target, f"/cards/{card['number']}/assignments", "POST", {"assignee_id": owner}, 204)
        add_people_and_assignments(target, board["id"], card["id"], last_name="Café 099")
        return process, target, jar, card
    except BaseException:
        stop_server(process)
        raise


def check_browser(name, target, jar, card):
    session = f"rizzy-assignment-keyboard-{name}-{os.getpid()}"
    path = f"/{target['account']}/cards/{card['number']}"
    try:
        browser(session, "open", target["url"] + "/session/new")
        for cookie in jar:
            browser(session, "cookies", "set", cookie.name, cookie.value,
                    "--url", target["url"], "--path", cookie.path, "--httpOnly")
        browser(session, "open", target["url"] + path)
        browser(session, "press", "a")
        initial = wait_for(session, lambda value: value.get("open") and value["count"] == 99,
                           (name, "open picker"))
        assert initial["focusSearch"] and initial["selectedIndex"] == 0, (name, initial)
        for key, index in (("ArrowDown", 1), ("ArrowRight", 2), ("ArrowUp", 1)):
            browser(session, "press", key)
            moved = wait_for(session, lambda value: value.get("selectedIndex") == index,
                             (name, key))
            assert moved["focusSearch"], (name, key, moved)
        browser(session, "fill", "dialog[open] input[type=search]", "cafe")
        filtered = wait_for(session, lambda value: "Café 099" in value.get("selectedText", ""),
                            (name, "filter selection"))
        assert filtered["selectedIndex"] == 0, (name, filtered)
        browser(session, "press", "Enter")
        wait_for(session, lambda value: not value["open"], (name, "submit assignment"))
        with sqlite3.connect(target["db"]) as db:
            count = db.execute("SELECT COUNT(*) FROM assignments WHERE card_id=?",
                               (database_id(target, card["id"]),)).fetchone()[0]
        assert count == 100, (name, count)
        assert browser(session, "get", "url") == target["url"] + path
        browser(session, "press", "a")
        full = wait_for(session, lambda value: value.get("open") and value["count"] == 100,
                        (name, "reopen full picker"))
        assert full["warning"] and full["unassignedHidden"], (name, full)
        browser(session, "press", "Escape")
        wait_for(session, lambda value: not value["open"], (name, "close picker"))
        return {"initial": initial["selectedIndex"], "moves": [1, 2, 1],
                "filtered": filtered["selectedIndex"], "assigned": count,
                "warning": full["warning"], "escape_closed": True}
    finally:
        browser(session, "close")


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-assignment-keyboard-") as temporary:
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = Path(temporary) / name
            directory.mkdir()
            process, target, jar, card = make_fixture(name, directory)
            try:
                results[name] = check_browser(name, target, jar, card)
            finally:
                stop_server(process)
        assert results["fizzy"] == results["rizzy"], results
    print("PASS paired assignment picker keyboard navigation, filter, submit, and Escape")


if __name__ == "__main__":
    main()
