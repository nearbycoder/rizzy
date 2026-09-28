#!/usr/bin/env python3
"""Check the account invite-link browser flow against pinned Fizzy."""
import html
import json
import re
import subprocess
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request
from users_events_html_parity import target


def qr_path(item, page, expected_url):
    assert b"Get QR code" in page, (item["name"], "missing QR control")
    image = next((tag for tag in re.findall(rb"<img\b[^>]*>", page)
                  if b'alt="QR Code"' in tag), None)
    assert image, (item["name"], "missing QR image")
    src = re.search(rb'src="([^"]+)"', image).group(1).decode()
    path = urllib.parse.urlparse(html.unescape(src)).path
    anonymous = {**item, "opener": urllib.request.build_opener(NoRedirect())}
    status, headers, svg = request(anonymous, path, accept="image/svg+xml")
    assert status == 200 and headers.get_content_type() == "image/svg+xml"
    cache = headers.get("Cache-Control", "")
    assert "public" in cache and "max-age=31556952" in cache
    png = subprocess.run(["rsvg-convert", "--width", "512"], input=svg,
                         capture_output=True, check=True).stdout
    scanned = subprocess.run(["zbarimg", "--quiet", "--raw", "-"], input=png,
                             capture_output=True, check=True).stdout.decode().strip()
    assert scanned == expected_url, (item["name"], scanned, expected_url)
    return path


def check(item):
    login(item)
    path = f"/{item['account']}/account/join_code"
    status, _, raw = request(item, path + ".json", bearer=True)
    assert status == 200, (item["name"], status)
    original = json.loads(raw)
    status, _, page = request(item, path, accept="text/html")
    assert status == 200 and b"Add people" in page
    assert original["code"].encode() in page and b"Change limit" in page
    first_qr = qr_path(item, page, original["url"])
    anonymous = {**item, "opener": urllib.request.build_opener(NoRedirect())}
    tampered = first_qr[:-1] + ("a" if first_qr[-1] != "a" else "b")
    wrong_account = first_qr.replace(f"/{item['account']}/qr_codes/",
                                     "/999999/qr_codes/", 1)
    qr_errors = {}
    for label, url in (("tampered", tampered), ("wrong_account", wrong_account)):
        for accepted in ("application/json", "image/svg+xml"):
            status, headers, _ = request(anonymous, url, accept=accepted)
            qr_errors[(label, accepted)] = (status, headers.get_content_type())
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
        latest = json.loads(raw)
        status, _, page = request(item, path, accept="text/html")
        assert status == 200
        assert qr_path(item, page, latest["url"]) != first_qr
        assert request(anonymous, first_qr, accept="image/svg+xml")[0] == 200
        return {"show": True, "edit": True, "update": True, "invalid": True,
                "reset": True, "qr": True, "qr_errors": qr_errors}
    finally:
        request(item, path + ".json", "PUT", {"account_join_code": {"usage_limit": original["usage_limit"]}}, bearer=True)


def main():
    source, rust = check(target("fizzy")), check(target("rizzy"))
    assert source == rust
    print("Invite link HTML matched: show, QR scan, edit, limit update, invalid limit, and reset")


if __name__ == "__main__":
    main()
