#!/usr/bin/env python3
"""Compare account-scoped passkey browser routes on paired databases."""

import json
import urllib.parse

from draft_workflow_parity import login, request


def check(target):
    login(target)
    scoped = f"/{target['account']}/my/passkeys"
    status, headers, _ = request(target, "/my/passkeys", accept="text/html")
    assert status == 302, (target["name"], "unscoped", status)
    unscoped_redirect = urllib.parse.urlparse(headers["Location"]).path

    status, _, page = request(target, scoped, accept="text/html")
    assert status == 200 and b"Register a passkey" in page, (target["name"], "index", status)
    assert scoped.encode() in page, (target["name"], "registration path")
    status, headers, _ = request(target, scoped + ".json", bearer=True)
    assert status == 406, (target["name"], "index JSON", status)

    status, _, raw = request(target, f"/{target['account']}/users", bearer=True)
    assert status == 200, (target["name"], "users", status)
    owner = next(user for user in json.loads(raw) if user["role"] == "owner")
    status, _, profile = request(target, f"/{target['account']}/users/{owner['id']}",
                                 accept="text/html")
    assert status == 200 and scoped.encode() in profile, (target["name"], "profile link")

    missing = scoped + "/missing/edit"
    status, headers, body = request(target, missing, accept="text/html")
    missing_html = (status, headers.get_content_type(), body)
    status, headers, body = request(target, missing + ".json", bearer=True)
    missing_json = (status, headers.get_content_type(), json.loads(body))
    status, headers, body = request(target, scoped + "/missing.json", "PATCH",
                                    {"passkey": {"name": "Missing"}}, bearer=True)
    missing_update = (status, headers.get_content_type(), json.loads(body))
    status, headers, body = request(target, scoped + "/missing.json", "DELETE", bearer=True)
    missing_delete = (status, headers.get_content_type(), json.loads(body))
    return {"unscoped_redirect": unscoped_redirect, "index": 200,
            "index_json": 406, "profile_link": True,
            "missing_html": missing_html, "missing_json": missing_json,
            "missing_update": missing_update, "missing_delete": missing_delete}
