#!/usr/bin/env python3
"""List paired HTML form contracts on representative populated pages."""

import argparse
import http.cookiejar
import os
import re
import sys
import tempfile
import urllib.request
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from route_get_valid_parity import fixture  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)

ID = re.compile(r"(?<![0-9A-Za-z])[0-9a-z]{25,26}(?![0-9A-Za-z])")


def normalized(value, account):
    value = value or ""
    prefix = f"/{account}/"
    if value.startswith(prefix):
        value = "/:account/" + value[len(prefix):]
    return ID.sub(":id", value)


class Forms(HTMLParser):
    def __init__(self, account):
        super().__init__()
        self.account = account
        self.forms = []
        self.active = None

    def finish_form(self):
        if self.active is not None:
            action, method, fields = self.active
            self.forms.append((action, method, tuple(fields)))
            self.active = None

    def handle_starttag(self, tag, attributes):
        attrs = dict(attributes)
        if tag == "form":
            self.finish_form()
            self.active = [normalized(attrs.get("action"), self.account),
                           attrs.get("method", "get").lower(), []]
            return
        if self.active is not None and tag in {"input", "select", "textarea", "button"}:
            name = attrs.get("name")
            if name and name != "authenticity_token":
                self.active[2].append((tag, name, attrs.get("type", "")))

    def handle_endtag(self, tag):
        if tag == "form":
            self.finish_form()

    def close(self):
        super().close()
        self.finish_form()


def collect(target):
    item = fixture(target)
    board, webhook, number, step, comment = (
        item[key] for key in ("board_id", "webhook_id", "number", "step_id", "comment_id"))
    paths = {
        "board_edit": f"/boards/{board}/edit",
        "webhook_new": f"/boards/{board}/webhooks/new",
        "webhook_edit": f"/boards/{board}/webhooks/{webhook}/edit",
        "card": f"/cards/{number}",
        "card_edit": f"/cards/{number}/edit",
        "step_edit": f"/cards/{number}/steps/{step}/edit",
        "comment_edit": f"/cards/{number}/comments/{comment}/edit",
    }
    result = {}
    for label, path in paths.items():
        status, _, body = request(target, f"/{target['account']}{path}", accept="text/html")
        assert status == 200, (target["name"], label, status)
        parser = Forms(target["account"])
        parser.feed(body.decode())
        parser.close()
        result[label] = parser.forms
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--strict", action="store_true",
                        help="fail when any normalized form contract differs")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="rizzy-html-forms-") as temporary:
        processes = []
        targets = {}
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
                targets[name] = {"name": name, "url": url,
                                 "account": str(credentials["account"]),
                                 "token": credentials["token"],
                                 "email": "owner@example.com",
                                 "db": str(directory / ("db.sqlite3" if name == "fizzy"
                                                        else "rizzy.sqlite3")),
                                 "opener": urllib.request.build_opener(
                                     urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                                     NoRedirect())}
                login(targets[name])
            source = collect(targets["fizzy"])
            rust = collect(targets["rizzy"])
            for label in source:
                print(f"\n{label}: Fizzy {len(source[label])}, Rizzy {len(rust[label])}")
                for name, forms in (("Fizzy", source[label]), ("Rizzy", rust[label])):
                    for action, method, fields in forms:
                        print(f"  {name}: {method.upper()} {action} {fields}")
            if args.strict:
                differences = [label for label in source if source[label] != rust[label]]
                assert not differences, f"Form contracts differ: {', '.join(differences)}"
                print("Matched all seven populated-page form contracts")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
