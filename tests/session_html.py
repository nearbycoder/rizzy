#!/usr/bin/env python3
"""Exercise the browser form sign-in flow with development magic links."""
import http.cookiejar
import os
import re
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, request


def main():
    target = {"url": os.environ.get("RIZZY_URL", "http://127.0.0.1:3030")}
    target["opener"] = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect()
    )
    email = os.environ.get("RIZZY_EMAIL", "owner@example.com")
    status, _, html = request(target, "/session/new", accept="text/html")
    assert status == 200 and b'form' in html and b'email_address' in html
    status, headers, _ = request(target, "/session", "POST",
                                 urllib.parse.urlencode({"email_address": email}).encode(), accept="text/html")
    assert status == 302 and headers["Location"] == "/session/magic_link", status
    status, _, html = request(target, "/session/magic_link", accept="text/html")
    assert status == 200 and email.encode() in html and b'name="code"' in html, status
    code = re.search(rb'Development code: <strong>([^<]+)</strong>', html)
    assert code, "Start Rizzy with RIZZY_DEV_MAGIC_LINKS=1 for this local check"
    status, headers, _ = request(target, "/session/magic_link", "POST",
                                 urllib.parse.urlencode({"code": code.group(1).decode()}).encode(), accept="text/html")
    assert status == 302 and headers["Location"] == "/landing", status
    status, headers, _ = request(target, "/landing", accept="text/html")
    assert status == 302 and headers["Location"] == "/session/menu", status
    status, headers, _ = request(target, "/session/menu", accept="text/html")
    assert status == 302 and re.fullmatch(r"/\d+/", headers["Location"]), status
    status, _, html = request(target, headers["Location"], accept="text/html")
    assert status == 200 and b'Latest Activity' in html, status
    print("Browser form sign-in matched: email, code entry, and account home")


if __name__ == "__main__":
    main()
