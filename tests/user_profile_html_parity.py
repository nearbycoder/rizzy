#!/usr/bin/env python3
"""Check browser user profile and name editing against pinned Fizzy."""
import json
import re
import urllib.parse

from draft_workflow_parity import login, request
from users_events_html_parity import target


def check(item):
    login(item)
    account = item["account"]
    path = f"/{account}/users/{item['user_id']}"
    status, _, raw = request(item, path + ".json", bearer=True)
    assert status == 200, (item["name"], status)
    original = json.loads(raw)["name"]
    status, _, profile = request(item, path, accept="text/html")
    assert status == 200 and original.encode() in profile and b'id="user_events"' in profile, (item["name"], status)
    assert b"Edit profile" in profile and b"avatar" in profile, item["name"]
    status, _, edit = request(item, path + "/edit", accept="text/html")
    assert status == 200 and b"Edit your profile" in edit and b'user[name]' in edit, (item["name"], status)
    csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', edit)
    csrf = csrf_match.group(1).decode() if csrf_match else None
    changed = original + " edited"
    try:
        form = urllib.parse.urlencode({"_method": "patch", "user[name]": changed}).encode()
        status, headers, _ = request(item, path, "POST", form, accept="text/html", csrf=csrf)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == path, (item["name"], status)
        status, _, raw = request(item, path + ".json", bearer=True)
        assert status == 200 and json.loads(raw)["name"] == changed, (item["name"], status)
        status, _, profile = request(item, path, accept="text/html")
        assert status == 200 and changed.encode() in profile, item["name"]
        empty = urllib.parse.urlencode({"_method": "patch", "user[name]": ""}).encode()
        status, _, body = request(item, path, "POST", empty, accept="text/html", csrf=csrf)
        assert status == 422 and b"Save changes" in body, (item["name"], status)
        status, _, _ = request(item, f"/{account}/users/no-such-user/edit", accept="text/html")
        assert status == 404, (item["name"], status)
        return {"profile": True, "edit": True, "name_redirect": True,
                "json_name": True, "invalid_name": True, "missing_user": True}
    finally:
        request(item, path + ".json", "PATCH", {"user": {"name": original}}, bearer=True)


def main():
    source, rust = check(target("fizzy")), check(target("rizzy"))
    assert source == rust
    print("User profile HTML matched: show, edit, name change, invalid name, and missing user")


if __name__ == "__main__":
    main()
