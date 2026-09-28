#!/usr/bin/env python3
"""Compare single-use device sign-in links with the pinned Fizzy app."""
import http.cookiejar
import html
import json
import os
import re
import sqlite3
import subprocess
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request


def opener():
    return urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect()
    )


def transfer_path(target, user_id):
    status, _, body = request(target, f"/{target['account']}/users/{user_id}", accept="text/html")
    assert status == 200, (target["name"], status)
    match = re.search(rb'<input[^>]*id="session_transfer_url"[^>]*>', body)
    assert match, (target["name"], "missing transfer link")
    url = re.search(rb'value="([^"]+)"', match.group()).group(1).decode()
    return urllib.parse.urlparse(url).path


def verify_qr(target, user_id, transfer):
    status, _, body = request(target, f"/{target['account']}/users/{user_id}", accept="text/html")
    assert status == 200
    tag = next((tag for tag in re.findall(rb'<img\b[^>]*>', body) if b'alt="QR Code"' in tag), None)
    assert tag, (target["name"], "missing QR image")
    source = re.search(rb'src="([^"]+)"', tag).group(1).decode()
    path = urllib.parse.urlparse(html.unescape(source)).path
    status, headers, svg = request(target, path, accept="image/svg+xml")
    assert status == 200 and headers.get_content_type() == "image/svg+xml", (target["name"], "QR image", status)
    cache = headers.get("Cache-Control", "")
    assert "public" in cache and "max-age=31556952" in cache, (target["name"], cache)
    assert b"<svg" in svg and b"</svg>" in svg, (target["name"], "QR SVG")
    png = subprocess.run(["rsvg-convert", "--width", "512"], input=svg, capture_output=True, check=True).stdout
    scan = subprocess.run(["zbarimg", "--quiet", "--raw", "-"], input=png, capture_output=True, check=True)
    assert urllib.parse.urlparse(scan.stdout.decode().strip()).path == transfer, (target["name"], "QR destination")
    return path


def run(target):
    login(target)
    account = target["account"]
    status, _, raw = request(target, f"/{account}/users", bearer=True)
    assert status == 200, (target["name"], status)
    users = json.loads(raw)
    owner = next(user for user in users if user["role"] == "owner")
    user_id = owner["id"]
    status, _, profile = request(target, f"/{account}/users/{user_id}", accept="text/html")
    assert status == 200 and b"Devices" in profile
    csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', profile)
    csrf = csrf_match.group(1).decode() if csrf_match else None

    def generate():
        status, headers, _ = request(target, f"/{account}/transfer_token", "POST", b"", accept="text/html", csrf=csrf)
        assert status == 302, (target["name"], status)
        assert urllib.parse.urlparse(headers["Location"]).path == f"/{account}/users/{user_id}"
        return transfer_path(target, user_id)

    first = generate()
    first_qr = verify_qr(target, user_id, first)
    second = generate()
    assert first != second, target["name"]
    status, _, _ = request(target, first_qr, accept="image/svg+xml")
    assert status == 200, (target["name"], "revoked transfer QR", status)
    anonymous = {**target, "opener": opener()}
    status, _, _ = request(anonymous, first, "PUT", b"", accept="text/html")
    assert status == 400, (target["name"], "revoked link", status)
    status, _, page = request(anonymous, second, accept="text/html")
    assert status == 200 and b"form" in page, (target["name"], "link page", status)
    status, headers, _ = request(anonymous, second, "PUT", b"", accept="text/html")
    assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == "/session/menu", (target["name"], "redeem", status)
    assert "session_token" in headers.get("Set-Cookie", "")
    status, _, _ = request({**target, "opener": opener()}, second, "PUT", b"", accept="text/html")
    assert status == 400, (target["name"], "replay", status)
    status, _, _ = request(target, first_qr, accept="image/svg+xml")
    assert status == 200, (target["name"], "redeemed transfer QR", status)
    third = generate()
    status, headers, _ = request({**target, "opener": opener()}, third, "PATCH", b"", accept="text/html")
    assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == "/session/menu", (target["name"], "patch redeem", status)
    return {"generate": 302, "revoked": 400, "show": 200, "redeem": 302, "replay": 400, "patch_redeem": 302}


def rust_expiry_and_race(target):
    account = target["account"]
    status, _, raw = request(target, f"/{account}/users", bearer=True)
    owner = next(user for user in json.loads(raw) if user["role"] == "owner")
    user_id = owner["id"]

    def generate():
        status, _, _ = request(target, f"/{account}/transfer_token", "POST", b"", accept="text/html")
        assert status == 302, ("generate", status)
        return transfer_path(target, user_id)

    expired = generate()
    expired_qr = verify_qr(target, user_id, expired)
    token = expired.rsplit("/", 1)[-1]
    expired_at = (datetime.now(timezone.utc) - timedelta(hours=5)).isoformat(
        timespec="milliseconds").replace("+00:00", "Z")
    if os.environ.get("RIZZY_TEST_MYSQL_URL"):
        from mysql_core import mysql_scalar
        mysql_scalar(os.environ["RIZZY_TEST_MYSQL_URL"],
                     f"UPDATE identity_transfers SET created_at='{expired_at}' WHERE token='{token}'")
    else:
        with sqlite3.connect(os.environ["RIZZY_DB_PATH"]) as db:
            db.execute("UPDATE identity_transfers SET created_at=? WHERE token=?",
                       (expired_at, token))
    status, _, _ = request({**target, "opener": opener()}, expired, "PUT", b"", accept="text/html")
    assert status == 400, ("expired", status)
    status, _, _ = request(target, expired_qr, accept="image/svg+xml")
    assert status == 200, ("expired transfer QR", status)
    changed = expired_qr[:-1] + ("a" if expired_qr[-1] != "a" else "b")
    status, _, _ = request(target, changed, accept="image/svg+xml")
    assert status == 500, ("tampered QR", status)
    other_account = expired_qr.replace(f"/{account}/qr_codes/", "/999999/qr_codes/", 1)
    status, _, _ = request(target, other_account, accept="image/svg+xml")
    assert status == 302, ("cross-account QR", status)
    race = generate()

    def redeem_once(_):
        return request({**target, "opener": opener()}, race, "PUT", b"", accept="text/html")[0]

    with ThreadPoolExecutor(max_workers=2) as pool:
        results = sorted(pool.map(redeem_once, range(2)))
    assert results == [302, 400], ("concurrent redeem", results)


def main():
    fizzy_db = os.environ.get("FIZZY_DB_PATH", "upstream/storage/production.sqlite3")
    with sqlite3.connect(fizzy_db) as db:
        email = db.execute(
            "SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
            (int(os.environ["FIZZY_ACCOUNT"]),),
        ).fetchone()[0]
    targets = []
    for name in ("FIZZY", "RIZZY"):
        targets.append({"name": name.lower(), "url": os.environ[f"{name}_URL"],
                        "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
                        "email": email if name == "FIZZY" else os.environ["RIZZY_EMAIL"],
                        "db": fizzy_db, "opener": opener()})
    source, rust = (run(target) for target in targets)
    assert source == rust, (source, rust)
    if os.environ.get("RIZZY_DB_PATH") or os.environ.get("RIZZY_TEST_MYSQL_URL"):
        rust_expiry_and_race(targets[1])
    print("Session transfer parity matched: generate, revoke, show, PUT/PATCH redeem, replay; Rust expiry and concurrent single use checked")


if __name__ == "__main__":
    main()
