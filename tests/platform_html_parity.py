#!/usr/bin/env python3
"""Compare Fizzy's platform body attributes with Rizzy on full HTML pages."""

import http.cookiejar
import json
import os
import sqlite3
import time
import urllib.parse
import urllib.request
from html.parser import HTMLParser

from draft_workflow_parity import NoRedirect, login, request
from session_transfer_parity import transfer_path


AGENTS = {
    "desktop": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "mobile_ios": "Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2 Mobile/15E148 Safari/604.1",
    "mobile_android": "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36",
    "native_ios": "Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Hotwire Native iOS/1.0 bridge-components: [buttons overflow-menu]",
    "native_android": "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Hotwire Native Android/1.0 bridge-components: [buttons overflow-menu form]",
    "native_empty": "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Hotwire Native Android/1.0 bridge-components: []",
    "native_multiple": "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Hotwire Native Android/1.0 bridge-components: [buttons] metadata: [ignored]",
}


class BodyAttributes(HTMLParser):
    def __init__(self):
        super().__init__()
        self.attributes = None

    def handle_starttag(self, tag, attrs):
        if tag == "body" and self.attributes is None:
            self.attributes = dict(attrs)


def body_attributes(raw):
    parser = BodyAttributes()
    parser.feed(raw.decode())
    assert parser.attributes is not None
    return tuple(parser.attributes.get(name) for name in (
        "data-platform", "data-bridge-platform", "data-bridge-components",
    ))


def cookie_for(target, name, value):
    host = urllib.parse.urlparse(target["url"]).hostname
    return http.cookiejar.Cookie(
        version=0, name=name, value=urllib.parse.quote(value, safe=""),
        port=None, port_specified=False, domain=host, domain_specified=False,
        domain_initial_dot=False, path="/", path_specified=True, secure=False,
        expires=None, discard=True, comment=None, comment_url=None, rest={},
        rfc2109=False,
    )


def run(target):
    login(target)
    status, _, raw = request(
        target, f"/{target['account']}/boards", "POST",
        {"board": {"name": f"Platform parity {time.time_ns()}"}}, bearer=True,
    )
    assert status == 201, (target["name"], status, raw[:120])
    board_id = json.loads(raw)["id"]
    path = f"/{target['account']}/boards/{board_id}"
    results = {}
    try:
        for name, agent in AGENTS.items():
            target["extra_headers"]["User-Agent"] = agent
            status, _, raw = request(target, path, accept="text/html")
            assert status == 200, (target["name"], name, status)
            results[name] = body_attributes(raw)
            assert all(item is not None for item in results[name]), (name, results[name])

        target["extra_headers"]["User-Agent"] = AGENTS["desktop"]
        status, headers, _ = request(target, path, accept="text/html")
        assert status == 200
        etag = headers.get("ETag")
        results["etag_present"] = bool(etag)
        if etag:
            target["extra_headers"]["User-Agent"] = AGENTS["native_android"]
            target["extra_headers"]["If-None-Match"] = etag
            status, _, raw = request(target, path, accept="text/html")
            results["conditional_platform_change"] = (status, len(raw) == 0)
            target["extra_headers"].pop("If-None-Match")
            target["extra_headers"]["User-Agent"] = AGENTS["desktop"]
            status, _, _ = request(
                target, path, "PATCH",
                {"board": {"name": f"Platform renamed {time.time_ns()}"}}, bearer=True,
            )
            assert status == 200, (target["name"], "rename", status)
            target["extra_headers"]["If-None-Match"] = etag
            status, changed_headers, raw = request(target, path, accept="text/html")
            assert status == 200 and raw and changed_headers.get("ETag") != etag
            results["etag_invalidation"] = status
            target["extra_headers"].pop("If-None-Match")

        target["extra_headers"]["User-Agent"] = AGENTS["desktop"]
        target["jar"].set_cookie(cookie_for(target, "x_user_agent", AGENTS["native_ios"]))
        status, _, raw = request(target, path, accept="text/html")
        assert status == 200
        results["cookie_override"] = body_attributes(raw)
        target["jar"].clear(urllib.parse.urlparse(target["url"]).hostname, "/", "x_user_agent")

        anonymous = dict(target, opener=urllib.request.build_opener(NoRedirect()),
                         extra_headers={"User-Agent": AGENTS["desktop"]})
        status, _, raw = request(anonymous, "/session/new", accept="text/html")
        assert status == 200
        results["public_layout"] = body_attributes(raw)

        status, _, raw = request(
            target, f"/{target['account']}/boards/{board_id}/publication", "POST",
            b"", bearer=True,
        )
        assert status == 201
        key = json.loads(raw)["public_url"].split("/public/boards/")[1]
        status, _, raw = request(
            target, f"/{target['account']}/public/boards/{key}", accept="text/html",
        )
        assert status == 200
        results["published_layout"] = body_attributes(raw)

        status, _, raw = request(target, f"/{target['account']}/users", bearer=True)
        assert status == 200
        owner = next(user for user in json.loads(raw) if user["role"] == "owner")
        status, _, _ = request(
            target, f"/{target['account']}/transfer_token", "POST", b"", accept="text/html",
        )
        assert status == 302
        transfer = transfer_path(target, owner["id"])
        anonymous = dict(target, opener=urllib.request.build_opener(NoRedirect()),
                         extra_headers={"User-Agent": AGENTS["desktop"]})
        status, _, raw = request(anonymous, transfer, accept="text/html")
        assert status == 200
        results["transfer_layout"] = body_attributes(raw)
        assert all(item is not None for item in results["transfer_layout"])
        status, _, _ = request(anonymous, transfer, "PUT", b"", accept="text/html")
        assert status == 302
    finally:
        target["extra_headers"].pop("User-Agent", None)
        status, _, _ = request(target, path, "DELETE", bearer=True)
        assert status == 204
    return results


def main():
    source_db = os.environ.get("FIZZY_DB_PATH", "upstream/storage/production.sqlite3")
    source_account = os.environ["FIZZY_ACCOUNT"]
    with sqlite3.connect(source_db) as db:
        email = db.execute(
            "SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id "
            "WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
            (int(source_account),),
        ).fetchone()[0]
    targets = []
    for name in ("FIZZY", "RIZZY"):
        jar = http.cookiejar.CookieJar()
        targets.append({
            "name": name.lower(), "url": os.environ[f"{name}_URL"].rstrip("/"),
            "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
            "email": email if name == "FIZZY" else os.environ["RIZZY_EMAIL"],
            "db": source_db, "jar": jar,
            "opener": urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar), NoRedirect()),
            "extra_headers": {"X-Forwarded-For": f"198.51.100.{time.time_ns() % 200 + 30}"} if name == "FIZZY" else {},
        })
    source, rust = (run(target) for target in targets)
    assert source == rust, (source, rust)
    print(f"Platform body attributes matched on {len(source)} browser and layout cases")


if __name__ == "__main__":
    main()
