#!/usr/bin/env python3
"""Compare the browser account settings forms with pinned Fizzy."""
import json
import re
import urllib.parse

from draft_workflow_parity import login, request
from users_events_html_parity import target


def check(item):
    login(item)
    path = f"/{item['account']}/account/settings"
    status, _, raw = request(item, path + ".json", bearer=True)
    assert status == 200, (item["name"], status)
    original = json.loads(raw)
    status, _, page = request(item, path, accept="text/html")
    assert status == 200 and b"Account Settings" in page and b"People on this account" in page
    assert b'account[name]' in page and b'role="radiogroup"' in page
    assert b"Export account data" in page and b"Invite people" in page
    assert item["email"].encode() in page
    csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
    csrf = csrf_match.group(1).decode() if csrf_match else None
    options = []
    for tag in re.findall(rb'<input\b[^>]*>', page):
        if b'name="entropy[auto_postpone_period_in_days]"' in tag and b'type="radio"' in tag:
            options.append(int(re.search(rb'value="(\d+)"', tag).group(1)))
    assert options == [3, 7, 30, 90, 365, 11], (item["name"], options)
    try:
        updated = original["name"] + " parity"
        form = urllib.parse.urlencode({"_method": "put", "account[name]": updated}).encode()
        status, headers, _ = request(item, path, "POST", form, accept="text/html", csrf=csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == path, (item["name"], status)
        status, _, raw = request(item, path + ".json", bearer=True)
        assert status == 200 and json.loads(raw)["name"] == updated

        blank = urllib.parse.urlencode({"_method": "put", "account[name]": ""}).encode()
        status, _, _ = request(item, path, "POST", blank, accept="text/html", csrf=csrf)
        assert status == 422, (item["name"], status)

        entropy = f"/{item['account']}/account/entropy"
        selected = 7 if original["auto_postpone_period_in_days"] != 7 else 11
        form = urllib.parse.urlencode({"_method": "put", "entropy[auto_postpone_period_in_days]": selected}).encode()
        status, headers, _ = request(item, entropy, "POST", form, accept="text/html", csrf=csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == path, (item["name"], status)
        status, _, raw = request(item, path + ".json", bearer=True)
        assert json.loads(raw)["auto_postpone_period_in_days"] == selected

        invalid = urllib.parse.urlencode({"_method": "put", "entropy[auto_postpone_period_in_days]": "1"}).encode()
        status, _, _ = request(item, entropy, "POST", invalid, accept="text/html", csrf=csrf)
        assert status == 422, (item["name"], status)
        status, _, raw = request(item, path + ".json", bearer=True)
        assert json.loads(raw)["auto_postpone_period_in_days"] == selected
        return {"show": True, "people": True, "options": options, "name": True, "blank_name": True,
                "entropy": True, "invalid_entropy": True}
    finally:
        request(item, path + ".json", "PUT", {"account": {"name": original["name"]}}, bearer=True)
        request(item, f"/{item['account']}/account/entropy.json", "PUT", {
            "entropy": {"auto_postpone_period_in_days": original["auto_postpone_period_in_days"]}}, bearer=True)


def main():
    source, rust = check(target("fizzy")), check(target("rizzy"))
    assert source == rust
    print("Account Settings HTML matched: page, people, name update, auto-close update and validation")


if __name__ == "__main__":
    main()
