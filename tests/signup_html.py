#!/usr/bin/env python3
"""Run the browser signup form through code verification and account creation."""
import http.cookiejar
import json
import os
import re
import subprocess
import tempfile
import urllib.parse
import urllib.request
from pathlib import Path

from auth import ROOT, start, stop
from draft_workflow_parity import NoRedirect, request


def main():
    subprocess.run(["cargo", "build", "--locked", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{Path(temp) / 'signup.sqlite3'}",
               "RIZZY_MULTI_TENANT": "1", "RIZZY_DEV_MAGIC_LINKS": "1"}
        port, server = start(env)
        item = {"url": f"http://127.0.0.1:{port}",
                "opener": urllib.request.build_opener(
                    urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
        email = "browser-owner@example.test"
        try:
            assert request(item, "/signup", accept="text/html")[0] == 302
            status, _, signin = request(item, "/session/new", accept="text/html")
            assert status == 200 and b'href="/signup/new"' in signin
            status, _, page = request(item, "/signup/new", accept="text/html")
            assert status == 200 and b'signup[email_address]' in page
            assert request(item, "/signup", "POST", urllib.parse.urlencode({"signup[email_address]": "bad"}).encode(), accept="text/html")[0] == 422
            status, headers, _ = request(item, "/signup", "POST",
                                         urllib.parse.urlencode({"signup[email_address]": email}).encode(),
                                         accept="text/html")
            assert status == 302 and headers["Location"] == "/session/magic_link"
            status, _, page = request(item, "/session/magic_link", accept="text/html")
            assert status == 200 and email.encode() in page
            code = re.search(rb'Development code: <strong>([^<]+)</strong>', page)
            assert code
            status, headers, _ = request(item, "/session/magic_link", "POST",
                                         urllib.parse.urlencode({"code": code.group(1).decode()}).encode(),
                                         accept="text/html")
            assert status == 302 and headers["Location"] == "/signup/completion/new"
            status, _, page = request(item, "/signup/completion/new", accept="text/html")
            assert status == 200 and b'signup[full_name]' in page
            assert b"Or import a Fizzy account" in page
            status, _, page = request(item, "/signup/completion", "POST",
                                      urllib.parse.urlencode({"signup[full_name]": ""}).encode(),
                                      accept="text/html")
            assert status == 422 and b"Full name can&#39;t be blank" in page
            status, headers, _ = request(item, "/signup/completion", "POST",
                                         urllib.parse.urlencode({"signup[full_name]": "Browser Owner"}).encode(),
                                         accept="text/html")
            assert status == 302 and re.fullmatch(r"/\d+/landing", headers["Location"])
            status, _, raw = request(item, "/my/identity.json")
            assert status == 200
            accounts = json.loads(raw)["accounts"]
            assert len(accounts) == 1 and accounts[0]["user"]["name"] == "Browser Owner"
            assert request(item, "/signup/new", accept="text/html")[1]["Location"] == "/signup/completion/new"
            status, _, _ = request(item, headers["Location"], accept="text/html")
            assert status == 302
        finally:
            stop(server)
    print("Browser signup passed: email, code, completion validation, account creation and landing")


if __name__ == "__main__":
    main()
