#!/usr/bin/env python3
"""Compare the card's inline step flow in disposable Fizzy and Rizzy browsers."""

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

from card_workflow import expect  # noqa: E402
from draft_workflow_parity import NoRedirect, login  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def browser(session, *arguments):
    return subprocess.run(["agent-browser", "--session", session, *arguments],
                          capture_output=True, text=True, check=True).stdout.strip()


def state(session, step_id):
    return json.loads(browser(session, "eval", f"""(() => {{
      const frame=document.getElementById('step_{step_id}');
      return {{editing:!!frame?.querySelector('input[name="step[content]"]'),
        text:frame?.querySelector('a.step__content')?.textContent.trim()||'',
        value:frame?.querySelector('input[name="step[content]"]')?.value||'',
        checked:frame?.querySelector('input[type=checkbox]')?.checked||false,
        focused:document.activeElement===frame?.querySelector('input[name="step[content]"]')}};
    }})()"""))


def wait_for(session, step_id, predicate, label):
    for _ in range(40):
        current = state(session, step_id)
        if predicate(current):
            return current
        time.sleep(0.2)
    raise AssertionError((label, current))


def fixture(name, directory):
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
        board, _ = expect(target, "/boards", "POST", {"board": {"name": "Step browser"}}, 201)
        card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                         {"card": {"title": "Step browser card"}}, 201)
        step, _ = expect(target, f"/cards/{card['number']}/steps", "POST",
                         {"step": {"content": "Initial step"}}, 201)
        return process, target, jar, card, step
    except BaseException:
        stop_server(process)
        raise


def check(name, target, jar, card, step):
    session = f"rizzy-step-browser-{name}-{os.getpid()}"
    path = f"/{target['account']}/cards/{card['number']}"
    step_id = step["id"]
    try:
        browser(session, "open", target["url"] + "/session/new")
        for cookie in jar:
            browser(session, "cookies", "set", cookie.name, cookie.value,
                    "--url", target["url"], "--path", cookie.path, "--httpOnly")
        browser(session, "open", target["url"] + path)
        wait_for(session, step_id, lambda value: value["text"] == "Initial step", "initial")

        browser(session, "click", f'turbo-frame#step_{step_id} a.step__content')
        wait_for(session, step_id, lambda value: value["editing"] and value["focused"], "edit")
        browser(session, "fill", f'turbo-frame#step_{step_id} input[name="step[content]"]',
                "Discarded text")
        browser(session, "press", "Escape")
        wait_for(session, step_id, lambda value: value["text"] == "Initial step", "cancel")

        browser(session, "click", f'turbo-frame#step_{step_id} a.step__content')
        wait_for(session, step_id, lambda value: value["editing"], "reopen")
        browser(session, "fill", f'turbo-frame#step_{step_id} input[name="step[content]"]',
                "Renamed step")
        browser(session, "click", f'turbo-frame#step_{step_id} form.step button[type=submit]')
        wait_for(session, step_id, lambda value: value["text"] == "Renamed step", "save")
        assert expect(target, f"/cards/{card['number']}/steps/{step_id}")[0]["content"] == "Renamed step"

        browser(session, "check", f'turbo-frame#step_{step_id} input[type=checkbox]')
        wait_for(session, step_id, lambda value: value["checked"], "complete")
        for _ in range(40):
            if expect(target, f"/cards/{card['number']}/steps/{step_id}")[0]["completed"]:
                break
            time.sleep(0.2)
        else:
            raise AssertionError((name, "completion was not persisted"))

        if name == "rizzy":
            browser(session, "click", f'turbo-frame#step_{step_id} a.step__content')
            wait_for(session, step_id, lambda value: value["editing"], "concurrent edit")
            browser(session, "fill", f'turbo-frame#step_{step_id} input[name="step[content]"]',
                    "Draft kept during refresh")
            expect(target, f"/cards/{card['number']}/steps/{step_id}", "PATCH",
                   {"step": {"content": "Remote change"}})
            browser(session, "eval", "refreshLiveCard(true)")
            preserved = state(session, step_id)
            assert preserved["value"] == "Draft kept during refresh" and preserved["focused"], preserved
            browser(session, "press", "Escape")
            wait_for(session, step_id, lambda value: value["text"] == "Remote change", "remote cancel")

        browser(session, "fill", '.steps li[id^="new_step_card_"] input[name="step[content]"]',
                "Second step")
        browser(session, "press", "Enter")
        second_id = None
        for _ in range(40):
            steps, _ = expect(target, f"/cards/{card['number']}/steps")
            if len(steps) == 2:
                second_id = next(item["id"] for item in steps if item["id"] != step_id)
                if state(session, second_id)["text"] == "Second step":
                    break
            time.sleep(0.2)
        assert second_id and state(session, second_id)["text"] == "Second step"
        browser(session, "click", f'turbo-frame#step_{second_id} a.step__content')
        wait_for(session, second_id, lambda value: value["editing"], "delete editor")
        browser(session, "click", f'turbo-frame#step_{second_id} form.button_to button')
        wait_for(session, second_id, lambda value: not value["editing"] and not value["text"], "delete")
        assert len(expect(target, f"/cards/{card['number']}/steps")[0]) == 1
        assert browser(session, "get", "url") == target["url"] + path
        return {"cancel": True, "save": True, "complete": True,
                "create": True, "delete": True, "same_url": True}
    finally:
        browser(session, "close")


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-step-browser-paired-") as temporary:
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = Path(temporary) / name
            directory.mkdir()
            process, target, jar, card, step = fixture(name, directory)
            try:
                results[name] = check(name, target, jar, card, step)
            finally:
                stop_server(process)
        assert results["fizzy"] == results["rizzy"], results
    print("PASS paired browser step edit, cancel, save, complete, create, and delete")


if __name__ == "__main__":
    main()
