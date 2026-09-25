#!/usr/bin/env python3
"""Run the browser signup form through code verification and account creation."""
import http.cookiejar
import json
import os
import re
import sqlite3
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
            slug = accounts[0]["slug"]
            with sqlite3.connect(Path(temp) / "signup.sqlite3") as db:
                assert db.execute("SELECT cards_count FROM accounts").fetchone()[0] == 11
                assert db.execute("SELECT COUNT(*) FROM boards").fetchone()[0] == 1
                assert db.execute("SELECT COUNT(*) FROM cards").fetchone()[0] == 11
                assert db.execute("SELECT COUNT(*) FROM events WHERE action='card_published'").fetchone()[0] == 11
                assert db.execute("SELECT COUNT(*) FROM watches WHERE watching=1").fetchone()[0] == 11
                assert db.execute("SELECT name,auto_postpone_period_in_days,entropy_override FROM boards").fetchone() == ("Playground", 365, 1)
                assert db.execute("SELECT number,title FROM cards ORDER BY number LIMIT 1").fetchone() == (1, "Finally, watch this Fizzy orientation video")
                assert db.execute("SELECT involvement FROM accesses").fetchone()[0] == "watching"
                assert db.execute("PRAGMA foreign_key_check").fetchall() == []
            reference = json.loads((ROOT / "static/signup_playground.json").read_text())
            assert len(reference) == 11
            for number, expected in enumerate(reference, 1):
                status, _, raw = request(item, f"{slug}/cards/{number}.json")
                assert status == 200
                card = json.loads(raw)
                assert card["title"] == expected["title"]
                assert card["description"] == expected["plain"], (number, card["description"])
                assert card["has_attachments"]
                if number == 1:
                    assert "<video" in card["description_html"]
                    assert "fizzyorientation-4k.mp4" in card["description_html"]
                if number == 3:
                    assert "<img" in card["description_html"]
                    assert "invite-link.gif" in card["description_html"]
            assert request(item, "/signup/new", accept="text/html")[1]["Location"] == "/signup/completion/new"
            status, landing_headers, _ = request(item, headers["Location"], accept="text/html")
            assert status == 302 and landing_headers["Location"].startswith(f"{slug}/boards/")
        finally:
            stop(server)
    print("Browser signup passed: email, code, completion validation, account creation and landing")


if __name__ == "__main__":
    main()
