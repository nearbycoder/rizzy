#!/usr/bin/env python3
"""Compare one, many, and zero-account browser chooser states."""

import http.cookiejar
import json
import re
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request


def path(location):
    return urllib.parse.urlparse(location).path


def browser(target, email):
    return {**target, "email": email, "opener": urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}


def check(target):
    owner = browser(target, target["email"])
    login(owner)
    status, headers, _ = request(owner, "/session/menu", accept="text/html")
    assert status == 302 and path(headers["Location"]) == f"/{target['account']}/"

    status, _, _ = request(owner, "/signup/completion.json", "POST",
                           {"signup": {"full_name": "Second Owner"}})
    assert status == 201, (target["name"], "second account", status)
    status, _, page = request(owner, "/session/menu", accept="text/html")
    assert status == 200 and b"Your " in page and b" accounts" in page
    links = [int(slug) for slug in re.findall(rb'href="/(\d+)/landing"', page)]
    assert len(links) == 2 and int(target["account"]) in links, (
        target["name"], "account links", links)
    assert b'popup__list' in page and b'popup__item' in page
    assert b'href="/signup/new"' in page
    status, headers, _ = request(owner, f"/{links[-1]}/landing", accept="text/html")
    assert status == 302 and path(headers["Location"]).startswith(f"/{links[-1]}/")

    newcomer = browser(target, "accountless-menu@example.test")
    login(newcomer)
    status, _, raw = request(newcomer, "/my/identity.json")
    assert status == 200 and json.loads(raw), (target["name"], "new identity")
    status, _, page = request(newcomer, "/session/menu", accept="text/html")
    assert status == 200 and b"Hmm..." in page and b"have any" in page
    assert b'href="/signup/new"' in page and b'popup__list' not in page
    return {"one_account_redirect": 302, "multiple_accounts": 2,
            "account_link": 302, "empty_menu": 200, "signup_link": True}
