#!/usr/bin/env python3
"""Compare signed-in saved-filter lifecycle and results with pinned Fizzy."""

import hashlib
import json
import re
import urllib.parse

from draft_workflow_parity import request


def checked(target, path, method="GET", data=None, *, accept="application/json", bearer=False,
            csrf=None, status=200):
    actual, headers, body = request(
        target, f"/{target['account']}{path}", method, data,
        accept=accept, bearer=bearer, csrf=csrf)
    assert actual == status, (target["name"], path, method, actual, body[:300])
    return headers, body


def stream(target, path, method, fields, csrf):
    data = urllib.parse.urlencode(fields, doseq=True).encode() if fields is not None else None
    headers, body = checked(target, path, method, data,
                            accept="text/vnd.turbo-stream.html", csrf=csrf)
    assert headers.get_content_type() == "text/vnd.turbo-stream.html", target["name"]
    assert b'<turbo-stream action="replace" target="filter-settings-save-toggle">' in body
    etag = headers.get("ETag")
    return body, (bool(etag), etag == 'W/"' + hashlib.sha256(body).hexdigest()[:32] + '"',
                  headers.get("Cache-Control"))


def check(target):
    board_id = None
    try:
        _, raw = checked(target, "/boards", "POST", {"board": {"name": "Saved view parity"}},
                         bearer=True, status=201)
        board_id = json.loads(raw)["id"]
        _, raw = checked(target, f"/boards/{board_id}/cards", "POST",
                         {"card": {"title": "Closed filter match"}}, bearer=True, status=201)
        number = json.loads(raw)["number"]
        checked(target, f"/cards/{number}/closure", "POST", {}, bearer=True, status=204)
        _, page = checked(target, "/cards", accept="text/html")
        match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
        csrf = match.group(1).decode() if match else None
        fields = {"indexed_by": "closed", "board_ids[]": [board_id]}
        body, create_headers = stream(target, "/filters", "POST", fields, csrf)
        match = re.search(rb'/filters/([0-9a-z]+)', body)
        assert match and b"Delete custom view" in body, (target["name"], body[:500])
        filter_id = match.group(1).decode()
        _, raw = checked(target, f"/cards?filter_id={filter_id}", bearer=True)
        titles = [card["title"] for card in json.loads(raw)]
        assert titles == ["Closed filter match"], (target["name"], titles)
        repeated, repeat_headers = stream(target, "/filters", "POST", fields, csrf)
        assert f"/filters/{filter_id}".encode() in repeated, target["name"]
        refreshed, refresh_headers = stream(
            target, "/filters/settings_refresh", "POST", {"filter_id": filter_id}, csrf)
        assert f"/filters/{filter_id}".encode() in refreshed, target["name"]
        deleted, delete_headers = stream(target, f"/filters/{filter_id}", "DELETE", None, csrf)
        assert b"Save custom view" in deleted, target["name"]
        checked(target, f"/cards?filter_id={filter_id}", bearer=True, status=404)
        json_status, json_headers, json_body = request(
            target, f"/{target['account']}/filters", "POST",
            urllib.parse.urlencode({"terms[]": "JSON format probe"}).encode(),
            accept="application/json", csrf=csrf)
        persisted, _ = stream(target, "/filters/settings_refresh", "POST",
                              {"terms[]": "JSON format probe"}, csrf)
        assert b"Delete custom view" in persisted, (target["name"], "JSON create persisted", persisted[:400])
        json_filter_id = re.search(rb'/filters/([0-9a-z]+)', persisted).group(1).decode()
        refresh_status, refresh_json_headers, refresh_json_body = request(
            target, f"/{target['account']}/filters/settings_refresh", "POST",
            urllib.parse.urlencode({"filter_id": json_filter_id}).encode(),
            accept="application/json", csrf=csrf)
        delete_status, delete_json_headers, delete_json_body = request(
            target, f"/{target['account']}/filters/{json_filter_id}", "DELETE",
            accept="application/json", csrf=csrf)
        after_delete, _ = stream(target, "/filters/settings_refresh", "POST",
                                 {"terms[]": "JSON format probe"}, csrf)
        assert b"Save custom view" in after_delete, (target["name"], "JSON delete persisted", after_delete[:400])
        return {"results": titles, "created": create_headers,
                "repeated": repeat_headers, "refreshed": refresh_headers,
                "deleted": delete_headers,
                "json_create": (json_status, json_headers.get_content_type(), bool(json_body)),
                "json_refresh": (refresh_status, refresh_json_headers.get_content_type(), bool(refresh_json_body)),
                "json_delete": (delete_status, delete_json_headers.get_content_type(), bool(delete_json_body))}
    finally:
        if board_id:
            checked(target, f"/boards/{board_id}", "DELETE", bearer=True, status=204)
