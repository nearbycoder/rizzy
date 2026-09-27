#!/usr/bin/env python3
"""Compare multi-tenant browser signup on disposable Fizzy and Rizzy MySQL databases."""

import http.cookiejar
import concurrent.futures
import json
import re
import threading
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request
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


def check_rate_limit(target, invalid=False, json_format=False):
    target["opener"] = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
    status, _, page = request(target, "/signup/new", accept="text/html")
    assert status == 200, (target["name"], "rate-limit signup page", status)
    token = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
    csrf = token.group(1).decode() if token else None
    email = "invalid" if invalid else "rate-limited@example.test"
    body = {"signup": {"email_address": email}} if json_format else urllib.parse.urlencode(
        {"signup[email_address]": email}).encode()
    before = (count(target, "identities"), count(target, "magic_links"))
    outcomes = []
    for number in range(11):
        status, headers, _ = request(target, "/signup", "POST", body,
                                     accept="application/json" if json_format else "text/html",
                                     csrf=csrf)
        outcomes.append((status, path(headers.get("Location", ""))))
        if number < 10:
            expected = (422, "") if invalid else (
                (201, "") if json_format else (302, "/session/magic_link"))
            assert outcomes[-1] == expected, (
                target["name"], number, outcomes[-1])
    assert outcomes[-1] == (302, "/signup/new"), (
        target["name"], "rate limit", outcomes[-1])
    after = (count(target, "identities"), count(target, "magic_links"))
    assert (after[0] - before[0], after[1] - before[1]) == (
        (0, 0) if invalid else (1, 10)), (target["name"], "rate-limit records", before, after)
    status, _, page = request(target, "/signup/new", accept="text/html")
    assert status == 200, (target["name"], "rate-limit landing", status)
    alert_visible = b"Try again later." in page
    assert alert_visible, (target["name"], "rate-limit flash missing")
    status, _, page = request(target, "/signup/new", accept="text/html")
    assert status == 200, (target["name"], "rate-limit second landing", status)
    alert_cleared = b"Try again later." not in page
    assert alert_cleared, (target["name"], "rate-limit flash persisted")
    return {"outcomes": outcomes, "identity_delta": after[0] - before[0],
            "link_delta": after[1] - before[1],
            "alert_visible": alert_visible, "alert_cleared": alert_cleared}


def check_rate_limit_cluster(target, urls):
    email = "cluster-rate-limited@example.test"
    before = (count(target, "identities"), count(target, "magic_links"))

    def submit(url):
        isolated = {**target, "url": url,
                    "opener": urllib.request.build_opener(NoRedirect())}
        status, headers, raw = request(isolated, "/signup", "POST",
                                       {"signup": {"email_address": email}})
        return status, path(headers.get("Location", "")), raw[:100]

    first = submit(urls[0])
    assert first[:2] == (201, ""), (target["name"], "first cluster signup", first)
    barrier = threading.Barrier(23)

    def concurrent_submit(index):
        barrier.wait(timeout=10)
        return submit(urls[index % len(urls)])

    with concurrent.futures.ThreadPoolExecutor(max_workers=23) as pool:
        results = list(pool.map(concurrent_submit, range(23)))
    statuses = sorted((status, redirect) for status, redirect, _ in [first, *results])
    assert set(statuses) <= {(201, ""), (302, "/signup/new")}, (
        target["name"], "cluster response", results)
    after = (count(target, "identities"), count(target, "magic_links"))
    assert (after[0] - before[0], after[1] - before[1]) == (1, 10), (
        target["name"], "cluster records", before, after, statuses)
    return {"created": statuses.count((201, "")),
            "throttled": statuses.count((302, "/signup/new")),
            "identity_delta": after[0] - before[0], "link_delta": after[1] - before[1]}


def check_completion_cluster(target, urls):
    rounds = 3
    concurrency = 4
    sessions = []
    for _ in range(concurrency):
        isolated = {**target, "opener": urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
        login(isolated)
        sessions.append(isolated)
    before = count(target, "accounts")
    for round_number in range(rounds):
        barrier = threading.Barrier(concurrency)

        def create(index):
            isolated = {**sessions[index], "url": urls[index % len(urls)]}
            barrier.wait(timeout=10)
            status, headers, raw = request(
                isolated, "/signup/completion.json", "POST",
                {"signup": {"full_name": "Concurrent Owner"}})
            return status, path(headers.get("Location", "")), raw[:100]

        with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
            outcomes = list(pool.map(create, range(concurrency)))
        assert all(status == 201 for status, _, _ in outcomes), (
            target["name"], "concurrent completion", round_number, outcomes)
    expected = rounds * concurrency
    assert count(target, "accounts") == before + expected, (target["name"], "account count")
    slugs = [int(value) for value in mysql_query(
        target["mysql_client"], target["mysql_socket"], target["mysql_database"],
        "SELECT external_account_id FROM accounts ORDER BY external_account_id").splitlines()]
    assert len(slugs) == len(set(slugs)), (target["name"], "duplicate account number", slugs)
    statement = (
        "SELECT a.external_account_id,"
        "(SELECT COUNT(*) FROM users u WHERE u.account_id=a.id AND u.role='owner'),"
        "(SELECT COUNT(*) FROM boards b WHERE b.account_id=a.id),"
        "(SELECT COUNT(*) FROM cards c WHERE c.account_id=a.id),"
        "(SELECT COUNT(*) FROM events e WHERE e.account_id=a.id),"
        "(SELECT COUNT(*) FROM watches w JOIN cards c ON c.id=w.card_id "
        "WHERE c.account_id=a.id AND w.watching=1) "
        "FROM accounts a WHERE a.external_account_id>1 ORDER BY a.external_account_id"
    )
    records = [tuple(map(int, line.split("\t"))) for line in mysql_query(
        target["mysql_client"], target["mysql_socket"], target["mysql_database"],
        statement).splitlines()]
    assert len(records) == expected, (target["name"], "account records", records)
    return {"created": expected, "slugs": slugs, "records": records}
