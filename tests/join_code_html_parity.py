#!/usr/bin/env python3
"""Check the account invite-link browser flow against pinned Fizzy."""
import json
import re
import urllib.parse

from draft_workflow_parity import login, request
from users_events_html_parity import target


def check(item):
    login(item)
    path = f"/{item['account']}/account/join_code"
    status, _, raw = request(item, path + ".json", bearer=True)
    assert status == 200, (item["name"], status)
    original = json.loads(raw)
    status, _, page = request(item, path, accept="text/html")
    assert status == 200 and b"Add people" in page
    assert original["code"].encode() in page and b"Change limit" in page
    csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
    csrf = csrf_match.group(1).decode() if csrf_match else None
    status, _, edit = request(item, path + "/edit", accept="text/html")
    assert status == 200 and b"Change usage limit" in edit and b'account_join_code[usage_limit]' in edit
    try:
        limit = original["usage_limit"] + 1
        form = urllib.parse.urlencode({"_method": "patch", "account_join_code[usage_limit]": limit}).encode()
        status, headers, _ = request(item, path, "POST", form, accept="text/html", csrf=csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == path, (item["name"], status)
        status, _, raw = request(item, path + ".json", bearer=True)
        assert json.loads(raw)["usage_limit"] == limit

        form = urllib.parse.urlencode({"_method": "patch", "account_join_code[usage_limit]": "99999999999999999999999999999999999"}).encode()
        status, _, edit = request(item, path, "POST", form, accept="text/html", csrf=csrf)
        assert status == 422 and b"cannot be larger than the population of the planet" in edit, (item["name"], status, edit[:300])

        status, headers, _ = request(item, path, "DELETE", accept="text/html", csrf=csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == path, (item["name"], status)
        status, _, raw = request(item, path + ".json", bearer=True)
        assert json.loads(raw)["code"] != original["code"]
        return {"show": True, "edit": True, "update": True, "invalid": True, "reset": True}
    finally:
        request(item, path + ".json", "PUT", {"account_join_code": {"usage_limit": original["usage_limit"]}}, bearer=True)


def main():
    source, rust = check(target("fizzy")), check(target("rizzy"))
    assert source == rust
    print("Invite link HTML matched: show, edit, limit update, invalid limit, and reset")


if __name__ == "__main__":
    main()
