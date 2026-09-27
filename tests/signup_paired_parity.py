#!/usr/bin/env python3
"""Compare multi-tenant browser signup on disposable Fizzy and Rizzy MySQL databases."""

import http.cookiejar
import json
import re
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, request
from sustained_mixed_mysql import mysql_query


def path(location):
    return urllib.parse.urlparse(location).path


def count(target, table, where="1=1"):
    return int(mysql_query(target["mysql_client"], target["mysql_socket"],
                           target["mysql_database"],
                           f"SELECT COUNT(*) FROM {table} WHERE {where}").splitlines()[-1])


def check(target):
    target["opener"] = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
    email = "paired-signup@example.test"
    status, _, page = request(target, "/signup/new", accept="text/html")
    assert status == 200 and b"signup[email_address]" in page, (target["name"], status)
    token = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
    csrf = token.group(1).decode() if token else None
    before = (count(target, "identities"), count(target, "magic_links"), count(target, "accounts"))
    form = lambda value: urllib.parse.urlencode(value).encode()
    status, _, _ = request(target, "/signup", "POST",
                           form({"signup[email_address]": "invalid"}),
                           accept="text/html", csrf=csrf)
    assert status == 422, (target["name"], "invalid email", status)
    assert (count(target, "identities"), count(target, "magic_links"),
            count(target, "accounts")) == before, target["name"]
    status, headers, _ = request(target, "/signup", "POST",
                                 form({"signup[email_address]": email}),
                                 accept="text/html", csrf=csrf)
    assert status == 302 and path(headers["Location"]) == "/session/magic_link", (
        target["name"], "valid email", status, headers.get("Location"))
    assert (count(target, "identities"), count(target, "magic_links")) == (
        before[0] + 1, before[1] + 1), target["name"]
    status, _, page = request(target, "/session/magic_link", accept="text/html")
    assert status == 200 and email.encode() in page, (target["name"], "code page", status)
    escaped_email = email.replace("'", "''")
    code = mysql_query(target["mysql_client"], target["mysql_socket"],
                       target["mysql_database"],
                       "SELECT m.code FROM magic_links m JOIN identities i ON i.id=m.identity_id "
                       f"WHERE i.email_address='{escaped_email}' ORDER BY m.created_at DESC LIMIT 1")
    status, headers, _ = request(target, "/session/magic_link", "POST",
                                 form({"code": code}), accept="text/html", csrf=csrf)
    assert status == 302 and path(headers["Location"]) == "/signup/completion/new", (
        target["name"], "code verification", status, headers.get("Location"))
    status, _, page = request(target, "/signup/completion/new", accept="text/html")
    assert status == 200 and b"signup[full_name]" in page, (target["name"], "completion", status)
    token = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
    csrf = token.group(1).decode() if token else None
    status, _, page = request(target, "/signup/completion", "POST",
                              form({"signup[full_name]": ""}),
                              accept="text/html", csrf=csrf)
    assert status == 422 and b"Full name can" in page, (
        target["name"], "blank name", status, page[:300])
    blank_error_class = b"txt-negative" in page
    assert count(target, "accounts") == before[2], target["name"]
    status, headers, _ = request(target, "/signup/completion", "POST",
                                 form({"signup[full_name]": "Paired Owner"}),
                                 accept="text/html", csrf=csrf)
    landing = path(headers.get("Location", ""))
    assert status == 302 and re.fullmatch(r"/\d+/landing", landing), (
        target["name"], "account creation", status, landing)
    assert count(target, "accounts") == before[2] + 1, target["name"]
    status, headers, _ = request(target, "/signup/new", accept="text/html")
    assert status == 302 and path(headers["Location"]) == "/signup/completion/new", (
        target["name"], "authenticated redirect", status)
    records = (count(target, "identities"), count(target, "magic_links"), count(target, "accounts"))
    status, headers, _ = request(target, "/signup", "POST",
                                 form({"signup[email_address]": email}),
                                 accept="text/html", csrf=csrf)
    assert status == 302 and path(headers["Location"]) == "/signup/completion/new", (
        target["name"], "authenticated signup post", status)
    assert (count(target, "identities"), count(target, "magic_links"),
            count(target, "accounts")) == records, target["name"]
    status, _, raw = request(target, "/signup/completion.json", "POST",
                             {"signup": {"full_name": ""}})
    assert status == 422 and "Full name can't be blank" in json.loads(raw)["errors"], (
        target["name"], "blank JSON name", status, raw[:300])
    assert count(target, "accounts") == records[2], target["name"]
    status, _, raw = request(target, "/my/identity.json")
    assert status == 200, (target["name"], "identity", status)
    accounts = json.loads(raw)["accounts"]
    assert any(account["user"]["name"] == "Paired Owner" for account in accounts), target["name"]
    return {"invalid_email": 422, "magic_link": 302, "code_verified": 302,
            "blank_name": 422, "blank_error_class": blank_error_class,
            "account_created": 302, "authenticated_redirect": 302,
            "authenticated_post": 302, "blank_json_name": 422,
            "account_count_delta": 1}
