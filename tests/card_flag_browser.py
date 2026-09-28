#!/usr/bin/env python3
"""Check Rizzy's Golden Ticket and completion form flow in Chromium."""

import http.cookiejar
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

from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import prepare_rust, rust_env, start_server, stop_server  # noqa: E402


def browser(session, *args):
    return subprocess.run(["agent-browser", "--session", session, *args],
                          capture_output=True, text=True, check=True).stdout.strip()


def state(session):
    return {"gold": None, "pin": None, "watch": None, "done": False, "undo": False, "closed": None} | json.loads(browser(session, "eval", """(() => ({
      ready:document.readyState,
      gold:document.getElementById('gold')?.dataset.method,
      pin:document.getElementById('pin')?.dataset.method,
      watch:document.getElementById('watch')?.dataset.method,
      done:!!document.getElementById('closure'),
      undo:!!document.getElementById('undo-closure'),
      closed:document.getElementById('card')?.classList.contains('is-closed'),
      editing:!!document.querySelector('form[id^="edit_form_card_"]'),
      save:!!document.querySelector('[id^="card_closure_toggle_card_"] button[form^="edit_form_card_"]'),
      move:!!document.getElementById('choose-column'),
      pinTray:!!document.getElementById('pin-tray')?.open,
      notificationTray:!!document.getElementById('notification-tray')?.open,
      tagDialog:!!document.querySelector('.tag-dialog')?.open,
      focusTagTrigger:document.activeElement === document.querySelector('.card__tag-picker-button'),
      title:document.querySelector('.card__title-link')?.textContent.trim()
    }))()"""))


def until(session, predicate, label):
    for _ in range(40):
        value = state(session)
        if value.get("ready") == "complete" and predicate(value):
            return
        time.sleep(0.2)
    raise AssertionError((label, value, browser(session, "get", "url"), browser(session, "errors")))


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-card-flag-browser-") as temporary:
        directory = Path(temporary)
        env = rust_env(directory)
        env["RIZZY_DEV_MAGIC_LINKS"] = "1"
        env, credentials = prepare_rust(directory, env)
        process, url = start_server("rizzy", directory, env, sorted(os.sched_getaffinity(0))[:2])
        session = f"rizzy-card-flags-{os.getpid()}"
        jar = http.cookiejar.CookieJar()
        target = {"name": "rizzy", "url": url, "account": str(credentials["account"]),
                  "token": credentials["token"], "email": "owner@example.com",
                  "db": str(directory / "rizzy.sqlite3"),
                  "opener": urllib.request.build_opener(
                      urllib.request.HTTPCookieProcessor(jar), NoRedirect())}
        board_id = None
        try:
            login(target)
            status, _, raw = request(target, f"/{target['account']}/boards", "POST",
                                     {"board": {"name": "Flag browser"}}, bearer=True)
            assert status == 201
            board_id = json.loads(raw)["id"]
            status, _, raw = request(target, f"/{target['account']}/boards/{board_id}/cards", "POST",
                                     {"card": {"title": "Flag browser card"}}, bearer=True)
            assert status == 201
            path = f"/{target['account']}/cards/{json.loads(raw)['number']}"
            browser(session, "open", url + "/session/new")
            for cookie in jar:
                browser(session, "cookies", "set", cookie.name, cookie.value,
                        "--url", url, "--path", cookie.path, "--httpOnly")
            browser(session, "open", url + path)
            until(session, lambda value: value["gold"] == "POST" and value["done"] and not value["closed"] and value["watch"] == "DELETE" and value["pin"] == "POST", "initial loaded controls")
            initial_controls = state(session)
            browser(session, "click", ".card__title-link")
            until(session, lambda value: value["editing"] and value["save"] and not value["done"] and value["watch"] == initial_controls["watch"] and value["pin"] == initial_controls["pin"], "edit controls")
            browser(session, "press", "Escape")
            until(session, lambda value: not value["editing"] and value["done"] and value["move"] and value["watch"] == initial_controls["watch"] and value["pin"] == initial_controls["pin"], "cancel restores controls")
            browser(session, "click", ".card__title-link")
            until(session, lambda value: value["editing"] and value["save"], "reopen editor")
            browser(session, "fill", 'textarea[name="card[title]"]', "Flag browser saved")
            browser(session, "click", 'form[id^="edit_form_card_"] button[type="submit"]')
            until(session, lambda value: not value["editing"] and value["title"] == "Flag browser saved" and value["done"] and value["move"], "save restores controls")
            browser(session, "press", "e")
            until(session, lambda value: value["editing"] and value["save"], "edit hotkey")
            browser(session, "click", 'textarea[name="card[title]"]')
            browser(session, "press", "Shift+g")
            assert state(session)["gold"] == "POST", "Typing in an editor triggered the Golden Ticket hotkey"
            browser(session, "press", "Escape")
            until(session, lambda value: not value["editing"] and value["done"], "hotkey edit cancel")
            browser(session, "press", "Shift+g")
            until(session, lambda value: value["gold"] == "DELETE", "Golden Ticket hotkey on")
            browser(session, "press", "Shift+g")
            until(session, lambda value: value["gold"] == "POST", "Golden Ticket hotkey off")
            browser(session, "press", "Shift+n")
            until(session, lambda value: value["watch"] == "POST", "watch hotkey off")
            assert not state(session)["notificationTray"], "Shift+n also opened the notification tray"
            browser(session, "press", "Shift+n")
            until(session, lambda value: value["watch"] == "DELETE", "watch hotkey on")
            browser(session, "press", "Shift+p")
            until(session, lambda value: value["pin"] == "DELETE", "pin hotkey on")
            assert not state(session)["pinTray"], "Shift+p also opened the pin tray"
            browser(session, "press", "Shift+p")
            until(session, lambda value: value["pin"] == "POST", "pin hotkey off")
            browser(session, "press", "p")
            until(session, lambda value: value["pinTray"], "unmodified pin-tray hotkey")
            browser(session, "press", "p")
            until(session, lambda value: not value["pinTray"], "close pin tray with hotkey")
            browser(session, "press", "n")
            until(session, lambda value: value["notificationTray"], "unmodified notification-tray hotkey")
            browser(session, "press", "n")
            until(session, lambda value: not value["notificationTray"], "close notification tray with hotkey")
            browser(session, "press", "t")
            until(session, lambda value: value["tagDialog"], "tag picker hotkey")
            browser(session, "press", "Escape")
            until(session, lambda value: not value["tagDialog"] and value["focusTagTrigger"], "close tag picker and restore focus")
            browser(session, "press", "d")
            until(session, lambda value: value["closed"] and value["undo"], "complete hotkey")
            browser(session, "click", "#undo-closure")
            until(session, lambda value: not value["closed"] and value["done"], "reopen after complete hotkey")
            browser(session, "click", "#gold")
            until(session, lambda value: value["gold"] == "DELETE", "promote")
            browser(session, "click", "#gold")
            until(session, lambda value: value["gold"] == "POST", "demote")
            browser(session, "click", "#closure")
            until(session, lambda value: value["closed"] and value["undo"], "complete")
            browser(session, "click", "#undo-closure")
            until(session, lambda value: not value["closed"] and value["done"], "reopen")
            browser(session, "set", "viewport", "390", "844")
            browser(session, "click", "#gold")
            until(session, lambda value: value["gold"] == "DELETE", "mobile promote")
            browser(session, "click", "#gold")
            until(session, lambda value: value["gold"] == "POST", "mobile demote")
            browser(session, "wait", "--load", "networkidle")
            browser(session, "click", "#closure")
            until(session, lambda value: value["closed"] and value["undo"], "mobile complete")
            browser(session, "click", "#undo-closure")
            until(session, lambda value: not value["closed"] and value["done"], "mobile reopen")
            for kind in ("pin", "watch"):
                browser(session, "click", f"#{kind}")
                until(session, lambda value, kind=kind: value[kind] == "DELETE", f"mobile {kind} on")
                browser(session, "click", f"#{kind}")
                until(session, lambda value, kind=kind: value[kind] == "POST", f"mobile {kind} off")
            assert browser(session, "get", "url") == url + path
            assert not browser(session, "errors"), "Browser page errors"
            print("PASS card edit restoration, keyboard shortcuts, and desktop/mobile control toggles")
        finally:
            try:
                browser(session, "close")
            except subprocess.CalledProcessError:
                pass
            if board_id:
                request(target, f"/{target['account']}/boards/{board_id}", "DELETE", bearer=True)
            stop_server(process)


if __name__ == "__main__":
    main()
