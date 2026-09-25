#!/usr/bin/env python3
"""Compare account-export lifecycle and ZIP envelope against Fizzy."""
import io
import json
import os
import time
import urllib.request
import zipfile

from card_image_parity import call


def request(target, path, method="GET", expected=200):
    status, _, raw = call(target, f"/{target['account']}{path}", method)
    assert status == expected, (path, status, raw[:160])
    return json.loads(raw) if raw and status in (200, 201) else None


def run(target):
    created = request(target, "/account/exports.json", "POST", 201)
    assert set(created) == {"id", "status", "created_at"} and created["status"] == "pending", created
    for _ in range(240):
        state = request(target, f"/account/exports/{created['id']}.json")
        if state["status"] in ("completed", "failed"):
            break
        time.sleep(.25)
    assert state["status"] == "completed" and set(state) == {
        "id", "status", "created_at", "download_url"}, state
    archive_request = urllib.request.Request(state["download_url"], headers={
        "Authorization": f"Bearer {target['token']}", "Accept": "application/zip"})
    with urllib.request.urlopen(archive_request, timeout=45) as response:
        assert response.status == 200 and response.headers.get_content_type() == "application/zip"
        assert response.headers.get("Content-Disposition", "").startswith("attachment;")
        archive_bytes = response.read()
    with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
        names = archive.namelist()
        assert "data/account.json" in names and any(name.startswith("data/users/") for name in names)
        account = json.loads(archive.read("data/account.json"))
        assert account["join_code"]["account_id"] == account["id"]
        shape = {"account_keys": sorted(account), "join_code_keys": sorted(account["join_code"])}
    request(target, "/account/exports/missing-export.json", expected=404)
    return shape


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        if not os.environ.get(f"{name}_URL"):
            continue
        target = {"url": os.environ[f"{name}_URL"],
                  "account": os.environ[f"{name}_ACCOUNT"],
                  "token": os.environ[f"{name}_TOKEN"]}
        results[name] = run(target)
    if len(results) == 2:
        assert results["FIZZY"] == results["RIZZY"], results
    print(f"Account export lifecycle and ZIP envelope passed on {', '.join(results)}")


if __name__ == "__main__":
    main()
