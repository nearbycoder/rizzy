#!/usr/bin/env python3
"""Compare email confirmation browser responses and check local session switch."""
import urllib.parse

from draft_workflow_parity import request
from users_events_html_parity import target


def invalid_link(item):
    account = item["account"]
    user = item["user_id"]
    path = f"/{account}/users/{user}/email_addresses/invalid/confirmation"
    status, _, page = request(item, path, accept="text/html")
    assert status == 200 and b"Confirm email change" in page and b"Just a sec" in page, (item["name"], status)
    status, _, expired = request(item, path, "POST", b"", accept="text/html")
    assert status == 422 and b"Link expired" in expired, (item["name"], status)
    assert f"/{account}/users/{user}/email_addresses/new".encode() in expired, item["name"]
    return {"show": True, "expired": True, "retry_link": True}


def valid_local(item):
    account = item["account"]
    user = item["user_id"]
    address = "confirmed-browser-change@example.test"
    request_path = f"/{account}/users/{user}/email_addresses"
    status, headers, _ = request(item, request_path, "POST", {"email_address": address}, bearer=True)
    assert status == 201 and headers.get("X-Email-Change-Token"), status
    token = headers["X-Email-Change-Token"]
    path = f"{request_path}/{urllib.parse.quote(token)}/confirmation"
    status, _, page = request(item, path, accept="text/html")
    assert status == 200 and b"Confirm email change" in page and token.encode() in page
    status, headers, _ = request(item, path, "POST", b"", accept="text/html")
    edit = f"/{account}/users/{user}/edit"
    assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == edit, status
    status, _, profile = request(item, edit, accept="text/html")
    assert status == 200 and address.encode() in profile, status
    status, _, expired = request(item, path, "POST", b"", accept="text/html")
    assert status == 422 and b"Link expired" in expired, status


def main():
    source, rust = target("fizzy"), target("rizzy")
    assert invalid_link(source) == invalid_link(rust)
    valid_local(rust)
    print("Email confirmation HTML matched: confirmation and expired link; Rizzy session switch passed")


if __name__ == "__main__":
    main()
