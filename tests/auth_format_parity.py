#!/usr/bin/env python3
"""Compare Fizzy and Rizzy bearer format and method restrictions."""
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request


def call(target, path, token, *, method="GET", accept="application/json", payload=None, global_path=False, extra_headers=None):
    root = target["url"] if global_path else f"{target['url']}/{target['account']}"
    headers = {"Accept": accept}
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    if payload is not None:
        headers["Content-Type"] = "application/json"
    if extra_headers:
        headers.update(extra_headers)
    request = urllib.request.Request(root + path, data=json.dumps(payload).encode() if payload is not None else None, headers=headers, method=method)
    try:
        response = urllib.request.urlopen(request, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.read(), response.headers


def checked(target, path, token, expected, **kwargs):
    status, body, headers = call(target, path, token, **kwargs)
    assert status == expected, (path, expected, status, body[:200])
    if status == 401:
        assert body == b"HTTP Token: Access denied.\n", body
        assert headers.get("WWW-Authenticate") == 'Token realm="Application"', headers
        assert headers.get_content_type() == "text/plain"
    return json.loads(body) if status in (200, 201) and body else None


def run(target):
    owner = target["token"]
    boards = checked(target, "/boards", owner, 200)
    assert boards
    board_id = boards[0]["id"]
    outcome = {}
    for accept in ("text/html", "*/*"):
        checked(target, "/boards", owner, 401, accept=accept)
        outcome[f"bearer_{accept}"] = 401
    checked(target, "/filters", owner, 401, method="POST", accept="text/vnd.turbo-stream.html", payload={})
    outcome["bearer_turbo"] = 401
    checked(target, "/boards", owner, 401, accept="text/html", extra_headers={"X-Rizzy-Json-Request":"1"})
    outcome["format_header_spoof"] = 401
    checked(target, f"/boards/{board_id}.json", owner, 200, accept="text/html")
    outcome["json_suffix"] = 200
    checked(target, "/boards", "invalid-bearer-token", 401)
    outcome["invalid_token"] = 401
    status, body, _ = call(target, "/boards", None)
    assert status == 406 and json.loads(body) == {"status":406,"error":"Not Acceptable"}
    outcome["missing_auth_json"] = status
    status, body, _ = call(target, "/boards", None, method="POST", payload={"board":{"name":"Should not exist"}})
    assert status == 406 and json.loads(body) == {"status":406,"error":"Not Acceptable"}
    outcome["missing_auth_post"] = status
    status, body, _ = call(target, "/boards", None, accept="text/html")
    assert status == 200 and b'<form' in body
    outcome["missing_auth_html"] = status
    status, _, headers = call(target, "/account/settings", None, method="PATCH", payload={})
    assert status == 302 and urllib.parse.urlparse(headers["Location"]).path == "/session/new"
    outcome["missing_auth_patch"] = status
    read_token = checked(target, "/my/access_tokens", owner, 201,
        method="POST", payload={"access_token": {"description": f"format-parity-{time.time_ns()}", "permission": "read"}}, global_path=True)
    try:
        checked(target, "/boards", read_token["token"], 200)
        outcome["read_get"] = 200
        checked(target, "/boards", read_token["token"], 401, method="POST", payload={"board": {"name": "Should not exist"}})
        outcome["read_post"] = 401
        checked(target, "/boards", read_token["token"], 401, method="POST", payload={"board": {"name": "Should not exist"}}, extra_headers={"X-Rizzy-Write-Request":"0"})
        outcome["method_header_spoof"] = 401
        checked(target, f"/my/access_tokens/{read_token['id']}", read_token["token"], 401, method="DELETE", global_path=True)
        outcome["read_delete"] = 401
    finally:
        status, _, _ = call(target, f"/my/access_tokens/{read_token['id']}", owner, method="DELETE", global_path=True)
        assert status == 204
    return outcome


def main():
    targets = [
        {"url": os.environ[f"{name}_URL"], "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"]}
        for name in ("FIZZY", "RIZZY")
    ]
    source, rust = (run(target) for target in targets)
    assert source == rust, (source, rust)
    print(f"Bearer authentication matched on {len(source)} format, permission, and denial checks")


if __name__ == "__main__":
    main()
