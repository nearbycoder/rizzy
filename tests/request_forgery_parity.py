#!/usr/bin/env python3
"""Compare Fizzy's header-only request forgery decisions with Rizzy."""

import http.cookiejar
import json
import os
import sqlite3
import time
import urllib.error
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request as browser_request


def request(target, method, path, payload=None, extra_headers=None):
    headers = {"Authorization": "Bearer " + target["token"],
               "Accept": "application/json", "Content-Type": "application/json"}
    if extra_headers:
        headers.update(extra_headers)
    req = urllib.request.Request(
        f"{target['url']}/{target['account']}{path}",
        data=json.dumps(payload).encode() if payload is not None else None,
        headers=headers, method=method,
    )
    try:
        response = urllib.request.urlopen(req, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read()
        return response.status, response.headers, json.loads(raw) if raw else None


def run(target):
    outcomes = {}
    cases = (
        ("missing", {}, 201),
        ("same_origin", {"Sec-Fetch-Site": "same-origin"}, 201),
        ("same_site", {"Sec-Fetch-Site": "same-site"}, 201),
        ("correct_origin", {"Sec-Fetch-Site": "same-origin", "Origin": target["url"]}, 201),
        ("cross_site", {"Sec-Fetch-Site": "cross-site"}, 422),
        ("cross_site_matching_origin", {"Sec-Fetch-Site": "cross-site", "Origin": target["url"]}, 422),
        ("none", {"Sec-Fetch-Site": "none"}, 422),
        ("invalid", {"Sec-Fetch-Site": "garbage"}, 422),
        ("bad_origin", {"Sec-Fetch-Site": "same-origin", "Origin": "https://other.example"}, 422),
        ("null_origin", {"Sec-Fetch-Site": "same-origin", "Origin": "null"}, 422),
        ("https_missing_json", {"X-Forwarded-Proto": "https"}, 201),
        ("https_same_origin", {"X-Forwarded-Proto": "https", "Sec-Fetch-Site": "same-origin"}, 201),
    )
    created = []
    try:
        for label, headers, expected in cases:
            status, response_headers, body = request(
                target, "POST", "/boards",
                {"board": {"name": f"Request forgery parity {label} {time.time_ns()}"}},
                headers,
            )
            assert status == expected, (target["name"], label, status, body)
            if status == 201:
                assert "Sec-Fetch-Site" in response_headers.get("Vary", ""), (label, response_headers)
                created.append(body["id"])
                outcomes[label] = status
            else:
                assert body == {"status": 422, "error": "Unprocessable Content"}, (label, body)
                outcomes[label] = (status, body)
    finally:
        for board_id in created:
            status, _, _ = request(target, "DELETE", f"/boards/{board_id}")
            assert status == 204, (target["name"], board_id, status)
    return outcomes


def main():
    targets = [
        {"name": name.lower(), "url": os.environ[f"{name}_URL"].rstrip("/"),
         "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"]}
        for name in ("FIZZY", "RIZZY")
    ]
    source, rust = (run(target) for target in targets)
    assert source == rust, (source, rust)
    fizzy_db = os.environ.get("FIZZY_DB_PATH", "upstream/storage/production.sqlite3")
    with sqlite3.connect(fizzy_db) as db:
        source_email = db.execute(
            "SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id "
            "WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
            (int(targets[0]["account"]),),
        ).fetchone()[0]
    html_results = []
    for target in targets:
        target["email"] = source_email if target["name"] == "fizzy" else os.environ.get("RIZZY_EMAIL", "owner@example.com")
        target["db"] = fizzy_db
        target["opener"] = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
        if target["name"] == "fizzy":
            target["extra_headers"] = {"X-Forwarded-For": f"198.51.100.{time.time_ns() % 200 + 30}"}
        login(target)
        target["extra_headers"] = {"Sec-Fetch-Site": "cross-site"}
        status, headers, body = browser_request(
            target, f"/{target['account']}/boards", "POST",
            urllib.parse.urlencode({"board[name]": "Blocked browser CSRF"}).encode(),
            accept="text/html",
        )
        html_results.append((status, headers.get_content_type(), body))
        status, _, _ = browser_request(
            target, f"/{target['account']}/notifications/unsubscribe?access_token=invalid",
            "POST", b"", accept="text/html",
        )
        assert status == 302, (target["name"], "unsubscribe exemption", status)
        target["extra_headers"] = {"Sec-Fetch-Site": "same-origin", "Origin": target["url"]}
        status, headers, _ = browser_request(
            target, f"/{target['account']}/boards", "POST",
            urllib.parse.urlencode({"board[name]": f"Allowed browser CSRF {time.time_ns()}"}).encode(),
            accept="text/html",
        )
        assert status == 302, (target["name"], "same-origin form", status)
        board_id = urllib.parse.urlparse(headers["Location"]).path.rsplit("/", 1)[-1]
        status, _, _ = request(target, "DELETE", f"/boards/{board_id}")
        assert status == 204, (target["name"], "same-origin form cleanup", status)
    assert html_results[0] == html_results[1] and html_results[0][0] == 422, (
        [(status, content_type, len(body)) for status, content_type, body in html_results])
    print(f"Request forgery decisions matched on {len(source)} JSON writes, HTML acceptance and rejection, and unsubscribe exemption")


if __name__ == "__main__":
    main()
