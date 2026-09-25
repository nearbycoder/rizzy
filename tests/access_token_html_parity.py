#!/usr/bin/env python3
"""Compare the browser personal-access-token lifecycle with pinned Fizzy."""
import json
import hashlib
import re
import sqlite3
import time
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request
from users_events_html_parity import target


def check(item):
    login(item)
    path = f"/{item['account']}/my/access_tokens"
    status, _, profile = request(item, f"/{item['account']}/users/{item['user_id']}", accept="text/html")
    assert status == 200 and b"personal access tokens" in profile
    status, _, raw = request(item, path + ".json", bearer=True)
    assert status == 200
    before = {entry["id"] for entry in json.loads(raw)}
    status, _, index = request(item, path, accept="text/html")
    assert status == 200 and b"Personal access tokens" in index
    assert b"Generate a new access token" in index
    status, _, global_index = request(item, "/my/access_tokens", accept="text/html")
    assert status == 200 and b"Personal access tokens" in global_index
    status, _, form_page = request(item, path + "/new", accept="text/html")
    assert status == 200 and b"Access token description" in form_page
    assert b'access_token[permission]' in form_page
    csrf = re.search(rb'<meta name="csrf-token" content="([^"]+)"', form_page)
    description = f"Browser token parity {time.time_ns()}"
    form = urllib.parse.urlencode({"access_token[description]": description,
                                   "access_token[permission]": "read"}).encode()
    token_id = None
    try:
        status, headers, _ = request(item, path, "POST", form, accept="text/html",
                                      csrf=csrf.group(1).decode() if csrf else None)
        assert status == 302, (item["name"], status)
        reveal = urllib.parse.urlparse(headers["Location"]).path
        assert "/my/access_tokens/" in reveal and reveal != path
        status, reveal_headers, page = request(item, reveal, accept="text/html")
        assert status == 200 and description.encode() in page
        if item["name"] == "rizzy":
            assert reveal_headers.get("Cache-Control") == "no-store"
            marker = str(time.time_ns())
            other_identity, other_session = "reveal-identity-" + marker, "reveal-session-" + marker
            with sqlite3.connect(item["db"]) as db:
                db.execute("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)",
                           (other_identity, f"reveal-{marker}@example.test", "2026-01-01T00:00:00Z"))
                db.execute("INSERT INTO sessions(id,identity_id,token_hash,created_at) VALUES(?,?,?,?)",
                           (other_session, other_identity, hashlib.sha256(other_session.encode()).hexdigest(),
                            "2026-01-01T00:00:00Z"))
            try:
                other_item = {**item, "opener": urllib.request.build_opener(NoRedirect()),
                              "extra_headers": {"Cookie": f"session_token={other_session}"}}
                global_reveal = "/my/access_tokens/" + reveal.rsplit("/", 1)[-1]
                assert request(other_item, global_reveal, accept="text/html")[0] == 404
            finally:
                with sqlite3.connect(item["db"]) as db:
                    db.execute("DELETE FROM sessions WHERE id=?", (other_session,))
                    db.execute("DELETE FROM identities WHERE id=?", (other_identity,))
        assert b"Copy access token" in page and b"you won" in page
        match = re.search(rb'<input\b[^>]*value="([^"]+)"[^>]*readonly', page)
        assert match, (item["name"], page[:300])
        raw_token = match.group(1).decode()
        assert len(raw_token) >= 24
        revealed_item = {**item, "token": raw_token,
                         "opener": urllib.request.build_opener(NoRedirect())}
        assert request(revealed_item, f"/{item['account']}/boards.json", bearer=True)[0] == 200
        status, _, raw = request(item, path + ".json", bearer=True)
        assert status == 200
        entries = json.loads(raw)
        created = [entry for entry in entries if entry["id"] not in before]
        assert len(created) == 1 and created[0]["description"] == description
        assert created[0]["permission"] == "read" and "token" not in created[0]
        token_id = created[0]["id"]
        status, _, index = request(item, path, accept="text/html")
        assert status == 200 and description.encode() in index and raw_token.encode() not in index
        status, _, _ = request(item, path + "/invalid-reveal", accept="text/html")
        assert status == 302
        if item["name"] == "rizzy":
            with sqlite3.connect(item["db"]) as db:
                db.execute("UPDATE access_token_reveals SET expires_at='2000-01-01T00:00:00Z' WHERE access_token_id=?", (token_id,))
            status, expired_headers, _ = request(item, reveal, accept="text/html")
            assert status == 302 and urllib.parse.urlparse(expired_headers["Location"]).path == path
        csrf = re.search(rb'<meta name="csrf-token" content="([^"]+)"', index)
        status, headers, _ = request(item, path + f"/{token_id}", "POST",
                                      urllib.parse.urlencode({"_method": "delete"}).encode(),
                                      accept="text/html", csrf=csrf.group(1).decode() if csrf else None)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == path
        status, _, raw = request(item, path + ".json", bearer=True)
        assert token_id not in {entry["id"] for entry in json.loads(raw)}
        token_id = None
        assert request(revealed_item, f"/{item['account']}/boards.json", bearer=True)[0] == 401
        return {"list": True, "new": True, "form_create": True, "show_once": True,
                "index_hides_token": True, "invalid_reveal": True, "revoke": True}
    finally:
        if token_id:
            status, _, _ = request(item, path + f"/{token_id}.json", "DELETE", bearer=True)
            assert status == 204, (item["name"], status)
            status, _, raw = request(item, path + ".json", bearer=True)
            assert token_id not in {entry["id"] for entry in json.loads(raw)}


def main():
    source = check(target("fizzy"))
    rust = check(target("rizzy"))
    assert source == rust
    print("Access token HTML matched: index, new form, create, timed reveal, hidden list value and revoke")


if __name__ == "__main__":
    main()
