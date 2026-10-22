#!/usr/bin/env python3
"""Compare live passkey challenge CSRF decisions with pinned Fizzy."""

import http.cookiejar
import json
import os
import re
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


def call(target, path, accept, csrf, form=False):
    headers = {"Accept": accept, "Content-Type":
               "application/x-www-form-urlencoded" if form else "application/json"}
    if csrf is not None and not form:
        headers["X-CSRF-Token"] = csrf
    body = (urllib.parse.urlencode({"purpose": "authentication",
                                    "authenticity_token": csrf or ""}).encode()
            if form else b'{"purpose":"authentication"}')
    request = urllib.request.Request(target["url"] + path, headers=headers,
                                     data=body, method="POST")
    try:
        response = target["opener"].open(request, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        body = response.read()
        destination = urllib.parse.urlsplit(response.headers.get("Location", "")).path
        if response.status == 200:
            value = json.loads(body)
            assert set(value) == {"challenge"} and isinstance(value["challenge"], str)
            assert value["challenge"]
            body = b"<challenge>"
        return response.status, response.headers.get_content_type(), destination, body


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-passkey-csrf-") as temporary:
        processes = []
        targets = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                targets[name] = {"url": url, "account": str(credentials["account"]),
                                 "opener": urllib.request.build_opener(
                                     urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                                     NoRedirect())}
            for target in targets.values():
                with target["opener"].open(target["url"] + "/session/new", timeout=10) as response:
                    assert response.status == 200
                    match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', response.read())
                    assert match, target["url"]
                    target["csrf"] = match.group(1).decode()
            count = 0
            for scoped in (False, True):
                for accept in ("application/json", "text/html"):
                    for token, form in ((None, False), ("wrong", False), ("valid", False),
                                        ("wrong", True), ("valid", True)):
                        results = {}
                        for name, target in targets.items():
                            csrf = target["csrf"] if token == "valid" else token
                            path = (f"/{target['account']}/my/passkey_challenge" if scoped
                                    else "/my/passkey_challenge")
                            results[name] = call(target, path, accept, csrf, form)
                        assert results["fizzy"] == results["rizzy"], (scoped, accept, token, form, results)
                        count += 1
        finally:
            for process in reversed(processes):
                stop_server(process)
    print(f"Matched pinned Fizzy passkey challenge CSRF responses in {count} cases")


if __name__ == "__main__":
    main()
