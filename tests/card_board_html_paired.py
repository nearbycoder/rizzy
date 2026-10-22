#!/usr/bin/env python3
"""Run the card board picker comparison on fresh Fizzy and Rizzy apps."""

import http.cookiejar
import json
import os
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from card_board_html_parity import check  # noqa: E402
from draft_workflow_parity import NoRedirect, request  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def browser(session, *arguments):
    return subprocess.run(["agent-browser", "--session", session, *arguments],
                          capture_output=True, text=True, check=True).stdout.strip()


def check_rust_browser(target):
    account = target["account"]
    boards = []
    session = f"rizzy-board-picker-{os.getpid()}"
    try:
        for label in ("Picker origin", "Picker destination"):
            status, _, raw = request(target, f"/{account}/boards", "POST",
                                     {"board": {"name": label}}, bearer=True)
            assert status == 201, (label, status)
            boards.append(json.loads(raw))
        status, _, raw = request(target, f"/{account}/boards/{boards[0]['id']}/cards", "POST",
                                 {"card": {"title": "Browser board move"}}, bearer=True)
        assert status == 201, status
        number = json.loads(raw)["number"]
        card_url = f"{target['url']}/{account}/cards/{number}"
        browser(session, "open", target["url"] + "/session/new")
        for cookie in target["jar"]:
            browser(session, "cookies", "set", cookie.name, cookie.value,
                    "--url", target["url"], "--path", cookie.path, "--httpOnly")
        browser(session, "open", card_url)
        browser(session, "click", "#board-picker-link")
        browser(session, "wait", "500")
        browser(session, "fill", "#board-search", "Picker destination")
        state = json.loads(browser(session, "eval", """(() => {
          const dialog = document.querySelector('#board-picker-dialog');
          const visible = [...dialog.querySelectorAll('.popup__item')].filter(item => !item.hidden);
          return { open: dialog.open, count: visible.length,
            choice: visible[0]?.textContent.trim(),
            focusSearch: document.activeElement?.id === 'board-search' };
        })()"""))
        assert state["open"] and state["count"] == 1 and state["focusSearch"], state
        assert "Picker destination" in state["choice"], state
        browser(session, "press", "Enter")
        browser(session, "wait", "700")
        assert browser(session, "get", "url") == card_url
        status, _, raw = request(target, f"/{account}/cards/{number}", bearer=True)
        assert status == 200 and json.loads(raw)["board"]["id"] == boards[1]["id"]
    finally:
        subprocess.run(["agent-browser", "--session", session, "close"],
                       capture_output=True, text=True)
        for board in boards:
            request(target, f"/{account}/boards/{board['id']}", "DELETE", bearer=True)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-card-board-paired-") as temporary:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env, credentials = prepare_source(directory, 2)
                else:
                    env = rust_env(directory)
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                    env, credentials = prepare_rust(directory, env)
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                database = directory / ("db.sqlite3" if name == "fizzy"
                                        else "rizzy.sqlite3")
                jar = http.cookiejar.CookieJar()
                target = {"name": name, "url": url,
                          "account": str(credentials["account"]),
                          "token": credentials["token"],
                          "email": "owner@example.com", "db": str(database),
                          "jar": jar,
                          "opener": urllib.request.build_opener(
                              urllib.request.HTTPCookieProcessor(jar),
                              NoRedirect())}
                results[name] = check(target)
                if name == "rizzy":
                    check_rust_browser(target)
            assert results["fizzy"] == results["rizzy"], results
            print("Matched fresh Fizzy and Rizzy board picker contracts; Rust browser move passed")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
