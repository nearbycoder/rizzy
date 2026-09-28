#!/usr/bin/env python3
"""Compare notification preferences on disposable Fizzy and Rizzy accounts."""

import http.cookiejar
import json
import os
import sys
import tempfile
import urllib.parse
import urllib.request
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


class Tags(HTMLParser):
    def __init__(self, raw):
        super().__init__()
        self.tags = []
        self.feed(raw.decode())

    def handle_starttag(self, tag, attrs):
        self.tags.append((tag, dict(attrs)))

    def find(self, tag, **wanted):
        return [attrs for name, attrs in self.tags if name == tag and
                all(attrs.get(key) == value for key, value in wanted.items())]

    def classes(self, tag, *classes):
        return [attrs for name, attrs in self.tags if name == tag and
                set(classes) <= set(attrs.get("class", "").split())]


def check(target):
    login(target)
    account = target["account"]
    board_ids = []
    for name in ("Zulu board", "Alpha board"):
        status, _, body = request(target, f"/{account}/boards", "POST",
                                  {"board": {"name": name}}, bearer=True)
        assert status == 201, (target["name"], status, body[:200])
        board_ids.append(json.loads(body)["id"])
    path = f"/{account}/notifications/settings"
    status, _, body = request(target, path, accept="text/html")
    assert status == 200, (target["name"], status)
    tags = Tags(body)
    assert len(tags.classes("h1", "header__title")) == 1
    assert len(tags.classes("div", "settings__panel")) == 2
    assert len(tags.classes("div", "settings__panel--users")) == 1
    assert len(tags.classes("div", "settings__scrollable-list")) == 1
    assert len(tags.find("section", **{"data-controller": "notifications"})) == 1
    subscribe = tags.find("div", **{"data-notifications-target": "subscribeButton"})
    assert len(subscribe) == 1 and "hidden" in subscribe[0]
    assert len(tags.find("form", **{"data-controller": "form"})) == 1
    assert len(tags.find("select", name="user_settings[bundle_email_frequency]")) == 1
    assert b"Alpha board" in body and b"Zulu board" in body
    assert body.index(b"Alpha board") < body.index(b"Zulu board")
    assert b"Email Notifications" in body and b"Help me fix this" in body
    board_frames = [attrs for attrs in tags.find("turbo-frame") if
                    attrs.get("id", "").startswith("involvement_board_")]
    button_frames = [attrs for attrs in tags.find("turbo-frame") if
                     attrs.get("id", "").startswith("involvement_button_board_")]
    assert len(board_frames) == len(button_frames) == 2, (target["name"], tags.find("turbo-frame"))
    assert len(tags.classes("form", "button_to")) >= 2, (target["name"], tags.classes("form", "button_to"))
    assert len(tags.find("input", name="_method", value="put")) >= 2
    selected = [attrs["value"] for attrs in tags.find("option") if "selected" in attrs]
    assert "every_few_hours" in selected, (target["name"], selected)
    csrf = next((attrs["content"] for attrs in tags.find("meta", name="csrf-token")), None)

    involvement_path = f"/{account}/boards/{board_ids[0]}/involvement"
    status, _, _ = request(target, involvement_path, "PUT",
                           {"involvement": "access_only"}, bearer=True)
    assert status == 204, (target["name"], status)
    status, headers, body = request(target, involvement_path + "?involvement=watching",
                                    "PUT", b"", accept="text/html", csrf=csrf)
    assert status == 200, (target["name"], status, body[:150])
    assert headers.get_content_type() == "text/html"
    assert Tags(body).find("turbo-frame", id=f"involvement_button_board_{board_ids[0]}")
    assert b"Stop watching" in body
    watch_form = urllib.parse.urlencode({"_method": "put", "involvement": "access_only",
                                        "show_watchers": "false", "icon_only": "true"}).encode()
    status, _, body = request(target, involvement_path, "POST", watch_form,
                              accept="text/html", csrf=csrf)
    assert status == 200, (target["name"], status, body[:150])
    assert Tags(body).find("turbo-frame", id=f"involvement_button_board_{board_ids[0]}")
    assert b"Watch this" in body
    status, _, _ = request(target, involvement_path, "POST", b"", accept="text/html", csrf=csrf)
    assert status == 404, (target["name"], status)
    anonymous = {**target, "opener": urllib.request.build_opener(NoRedirect())}
    status, _, _ = request(anonymous, involvement_path, "POST", b"", accept="text/html")
    assert status == 404, (target["name"], status)

    form = urllib.parse.urlencode({"_method": "patch", "user_settings[bundle_email_frequency]": "daily"}).encode()
    status, headers, _ = request(target, path, "POST", form, accept="text/html", csrf=csrf)
    assert status == 302, (target["name"], status)
    assert urllib.parse.urlparse(headers["Location"]).path == path
    status, _, body = request(target, path + ".json", bearer=True)
    assert status == 200 and json.loads(body)["bundle_email_frequency"] == "daily"
    status, _, body = request(target, path, accept="text/html")
    assert status == 200
    assert "daily" in [attrs["value"] for attrs in Tags(body).find("option") if "selected" in attrs]
    return {"panels": 2, "board_frames": len(board_frames), "saved_frequency": "daily"}


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-notification-settings-") as temporary:
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
                results[name] = check({
                    "name": name, "url": url, "account": str(credentials["account"]),
                    "token": credentials["token"], "email": "owner@example.com",
                    "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                    "opener": urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                        NoRedirect()),
                })
            assert results["fizzy"] == results["rizzy"], results
            print("PASS paired notification settings, board watch forms, and email update")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
