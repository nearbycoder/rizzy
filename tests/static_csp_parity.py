#!/usr/bin/env python3
"""Compare pinned Fizzy's static error pages and production 404 policy."""

import os
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402
from events_html import browser_cookie  # noqa: E402

STATIC_POLICY = ("default-src 'none'; img-src 'self'; style-src 'self'; "
                 "base-uri 'none'; form-action 'none'; frame-ancestors 'none'")
PAGES = ("400.html", "404.html", "406-unsupported-browser.html", "422.html", "500.html")


def get(url, path, accept="text/html", cookie=None):
    headers = {"Accept": accept}
    if cookie:
        headers["Cookie"] = cookie
    request = urllib.request.Request(url + path, headers=headers)
    try:
        response = urllib.request.urlopen(request, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


def check(name, url, credentials, directory):
    results = {}
    for page in PAGES:
        status, headers, body = get(url, "/" + page)
        assert status == 200 and headers["Content-Security-Policy"] == STATIC_POLICY, (
            name, page, status, headers)
        assert body == (ROOT / "upstream" / "public" / page).read_bytes(), (name, page)
        assert b"<script" not in body and b"javascript:" not in body, (name, page)
        results[page] = (status, headers["Content-Security-Policy"], body)
    status, headers, body = get(url, "/nonexistent/static-csp-probe")
    assert status == 404 and headers["Content-Security-Policy"] == STATIC_POLICY, (
        name, status, headers)
    assert body == (ROOT / "upstream" / "public" / "404.html").read_bytes(), name
    results["fallback"] = (status, headers["Content-Security-Policy"], body)
    for path in ("/nonexistent/static-csp-probe", "/client_configurations/invalid.json"):
        status, headers, body = get(url, path, "application/json")
        assert status == 404 and headers.get_content_type() == "application/json", (
            name, path, status, headers)
        assert headers.get("Content-Security-Policy") is None, (name, path, headers)
        results["json:" + path] = (status, body)
    target = {"name": name, "url": url, "account": str(credentials["account"]),
              "email": "owner@example.com",
              "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3"))}
    cookie = browser_cookie(target)
    for path, accept, session in (
        ("/up", "text/html", None),
        ("/session/new", "text/html", None),
        ("/signup/new", "text/html", None),
        ("/client_configurations/android_v1.json", "application/json", None),
        (f"/{credentials['account']}/account/settings", "text/html", cookie),
    ):
        status, headers, body = get(url, path, accept, session)
        assert status == 200, (name, path, status, body[:120])
        csp = headers["Content-Security-Policy"]
        match = re.search(r"'nonce-([^']+)'", csp)
        assert match, (name, path, csp)
        assert len(match.group(1)) == 24
        normalized = csp.replace(match.group(1), "[NONCE]")
        key = "/[ACCOUNT]/account/settings" if session else path
        results[key] = normalized
        if name == "rizzy" and b"<script" in body:
            for tag in re.findall(rb"<script\b[^>]*>", body, re.I):
                assert b'nonce="' + match.group(1).encode() + b'"' in tag, (path, tag)
    return results


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-static-csp-") as temporary:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                if name == "rizzy":
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                results[name] = check(name, url, credentials, directory)
                print(f"{name}: static errors and dynamic page policies verified",
                      flush=True)
        finally:
            for process in processes:
                stop_server(process)
        for path in results["fizzy"]:
            assert results["fizzy"][path] == results["rizzy"][path], (
                path, results["fizzy"][path], results["rizzy"][path])


if __name__ == "__main__":
    main()
