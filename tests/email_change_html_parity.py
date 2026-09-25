#!/usr/bin/env python3
"""Compare the browser email-change form and validation with pinned Fizzy."""
import re
import urllib.parse

from draft_workflow_parity import login, request
from users_events_html_parity import target


def check(item):
    login(item)
    account = item["account"]
    user_id = item["user_id"]
    edit_path = f"/{account}/users/{user_id}/edit"
    new_path = f"/{account}/users/{user_id}/email_addresses/new"
    create_path = f"/{account}/users/{user_id}/email_addresses"
    status, _, edit = request(item, edit_path, accept="text/html")
    assert status == 200 and b"Change email" in edit and new_path.encode() in edit, item["name"]
    status, _, form = request(item, new_path, accept="text/html")
    assert status == 200 and b"Change your email" in form and b'name="email_address"' in form, (item["name"], status)
    assert create_path.encode() in form and b"New email address" in form, item["name"]
    csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', form)
    csrf = csrf_match.group(1).decode() if csrf_match else None
    for email in ("invalid", item["email"]):
        body = urllib.parse.urlencode({"email_address": email}).encode()
        status, headers, _ = request(item, create_path, "POST", body, accept="text/html", csrf=csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == new_path, (item["name"], email, status)
    status, _, _ = request(item, f"/{account}/users/no-such-user/email_addresses/new", accept="text/html")
    assert status == 404, (item["name"], status)
    if item["name"] == "rizzy":
        body = urllib.parse.urlencode({"email_address": "pending-browser-change@example.test"}).encode()
        status, _, sent = request(item, create_path, "POST", body, accept="text/html", csrf=csrf)
        assert status == 200 and b"Check your email" in sent and b"pending-browser-change@example.test" in sent
    return {"new": True, "edit_link": True, "invalid_redirect": True,
            "same_email_redirect": True, "missing_user": True}


def main():
    assert check(target("fizzy")) == check(target("rizzy"))
    print("Email-change browser form matched: new page, edit link, and validation redirects")


if __name__ == "__main__":
    main()
