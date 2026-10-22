#!/usr/bin/env python3
"""Compare the public email/code sign-in browser flow on paired MySQL databases."""

import http.cookiejar
import json
import re
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, request
from sustained_mixed_mysql import mysql_query


def path(location):
    return urllib.parse.urlparse(location).path


def csrf_from(page):
    match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
    return match.group(1).decode() if match else None


def check(target):
    target["opener"] = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
    status, headers, _ = request(target, "/session/magic_link", accept="text/html")
    assert status == 302 and path(headers["Location"]) == "/session/new"
    status, _, page = request(target, "/session/new", accept="text/html")
    assert status == 200 and b"Enter your email address to sign in." in page
    status, _, page_again = request(target, "/session/new", accept="text/html")
    assert status == 200 and b"Enter your email address to sign in." not in page_again

    form = lambda fields: urllib.parse.urlencode(fields).encode()
    csrf = csrf_from(page_again)
    status, headers, _ = request(target, "/session/magic_link", "POST",
                                 form({"code": "WRONG0"}), accept="text/html", csrf=csrf)
    assert status == 302 and path(headers["Location"]) == "/session/new"
    status, _, page = request(target, "/session/new", accept="text/html")
    assert status == 200 and b"Enter your email address to sign in." in page

    status, headers, _ = request(target, "/session", "POST",
                                 form({"email_address": target["email"]}),
                                 accept="text/html", csrf=csrf_from(page))
    assert status == 302 and path(headers["Location"]) == "/session/magic_link"
    status, _, page = request(target, "/session/magic_link", accept="text/html")
    assert status == 200 and b"one-time-code" in page
    csrf = csrf_from(page)
    status, headers, _ = request(target, "/session/magic_link", "POST",
                                 form({"code": "WRONG0"}), accept="text/html", csrf=csrf)
    assert status == 302 and path(headers["Location"]) == "/session/magic_link"
    status, _, page = request(target, "/session/magic_link", accept="text/html")
    assert status == 200 and re.search(rb'<div class="panel [^"]*\bshake\b', page)
    status, _, page_again = request(target, "/session/magic_link", accept="text/html")
    assert status == 200 and not re.search(rb'<div class="panel [^"]*\bshake\b', page_again)
    status, _, raw = request(target, "/session/magic_link", "POST", {"code": "WRONG0"})
    assert status == 401 and json.loads(raw) == {"message": "Try another code."}

    email = target["email"].replace("'", "''")
    code = mysql_query(target["mysql_client"], target["mysql_socket"],
                       target["mysql_database"],
                       "SELECT m.code FROM magic_links m JOIN identities i ON i.id=m.identity_id "
                       f"WHERE i.email_address='{email}' ORDER BY m.created_at DESC LIMIT 1")
    status, headers, _ = request(target, "/session/magic_link", "POST",
                                 form({"code": code}), accept="text/html", csrf=csrf)
    assert status == 302, (target["name"], status)
    destination = path(headers["Location"])
    status, _, raw = request(target, "/my/identity.json")
    assert status == 200 and json.loads(raw), (target["name"], status)
    status, headers, _ = request(target, destination, accept="text/html")
    assert status == 302 and path(headers["Location"]) == "/session/menu"
    status, headers, _ = request(target, "/session/menu", accept="text/html")
    assert status == 302 and path(headers["Location"]) == f"/{target['account']}/"
    return {"missing_pending_get": 302, "missing_pending_post": 302,
            "alert_once": True, "invalid_form": 302, "shake_once": True,
            "invalid_json": 401, "signed_in": 302, "destination": destination,
            "landing_menu": 302, "account_home": 302}


def check_rate_limit(target):
    target["opener"] = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
    status, _, page = request(target, "/session/new", accept="text/html")
    assert status == 200
    csrf = csrf_from(page)
    form = lambda fields: urllib.parse.urlencode(fields).encode()
    status, headers, _ = request(target, "/session", "POST",
                                 form({"email_address": target["email"]}),
                                 accept="text/html", csrf=csrf)
    assert status == 302 and path(headers["Location"]) == "/session/magic_link"
    status, _, page = request(target, "/session/magic_link", accept="text/html")
    assert status == 200
    csrf = csrf_from(page)
    for attempt in range(11):
        status, headers, _ = request(target, "/session/magic_link", "POST",
                                     form({"code": "WRONG0"}), accept="text/html", csrf=csrf)
        assert status == 302 and path(headers["Location"]) == "/session/magic_link", (
            target["name"], attempt, status, headers.get("Location"))
    status, _, page = request(target, "/session/magic_link", accept="text/html")
    assert status == 200 and b"Try again in 15 minutes." in page
    status, _, page = request(target, "/session/magic_link", accept="text/html")
    assert status == 200 and b"Try again in 15 minutes." not in page
    status, _, raw = request(target, "/session/magic_link", "POST", {"code": "WRONG0"})
    assert status == 429 and json.loads(raw) == {"message": "Try again in 15 minutes."}
    return {"invalid_form_redirects": 10, "throttled_form_redirect": 302,
            "alert_once": True, "throttled_json": 429}
