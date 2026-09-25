#!/usr/bin/env python3
"""Compare direct uploads, card image assignment, access, and removal."""
import base64
import hashlib
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
IMAGE = (ROOT / "upstream/test/fixtures/files/moon.jpg").read_bytes()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


NO_REDIRECT = urllib.request.build_opener(NoRedirect())


def call(target, path, method="GET", payload=None, *, bearer=True, headers=None):
    url = urllib.parse.urljoin(target["url"], path)
    request_headers = {"Accept": "application/json"}
    if bearer:
        request_headers["Authorization"] = f"Bearer {target['token']}"
    if isinstance(payload, dict):
        request_headers["Content-Type"] = "application/json"
        payload = json.dumps(payload).encode()
    request_headers.update(headers or {})
    request = urllib.request.Request(url, data=payload, headers=request_headers, method=method)
    try:
        response = NO_REDIRECT.open(request, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


def run(target):
    account = target["account"]
    prefix = f"/{account}"
    status, _, raw = call(target, prefix + "/boards", "POST", {"board": {"name": f"Image parity {time.time_ns()}"}})
    assert status == 201, (status, raw[:150])
    board = json.loads(raw)
    try:
        status, _, raw = call(target, prefix + f"/boards/{board['id']}/cards", "POST", {"card": {"title": "Image parity"}})
        assert status == 201, (status, raw[:150])
        number = json.loads(raw)["number"]
        blob = {"filename": "moon.jpg", "byte_size": len(IMAGE),
                "checksum": base64.b64encode(hashlib.md5(IMAGE).digest()).decode(),
                "content_type": "image/jpeg"}
        status, headers, _ = call(target, prefix + "/rails/active_storage/direct_uploads", "POST", {"blob": blob}, bearer=False)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path.endswith("/session/new"), status
        status, _, _ = call(target, prefix + "/rails/active_storage/direct_uploads", "POST", {"blob": blob},
                            bearer=False, headers={"Authorization": "Bearer invalid-token"})
        assert status == 401, status
        status, headers, raw = call(target, prefix + "/rails/active_storage/direct_uploads", "POST", {"blob": blob})
        assert status == 200 and headers.get_content_type() == "application/json", (status, raw[:150])
        upload = json.loads(raw)
        assert all(field in upload for field in ("id", "signed_id", "key", "filename", "byte_size", "checksum", "content_type", "direct_upload")), upload
        assert upload["byte_size"] == len(IMAGE) and upload["filename"] == "moon.jpg"
        upload_url = upload["direct_upload"]["url"]
        status, _, raw = call(target, upload_url, "PUT", IMAGE, bearer=False, headers=upload["direct_upload"]["headers"])
        assert status == 204 and raw == b"", (status, raw[:150])
        status, _, raw = call(target, prefix + f"/cards/{number}.json", "PATCH", {"card": {"image": upload["signed_id"]}})
        card = json.loads(raw)
        assert status == 200 and card["image_url"] and "/rails/active_storage/blobs/redirect/" in card["image_url"], (status, card)
        assert card["has_attachments"] is False
        status, _, raw = call(target, prefix + f"/boards/{board['id']}/cards", "POST", {"card": {"title": "Cannot reuse image"}})
        assert status == 201, (status, raw[:150])
        other_number = json.loads(raw)["number"]
        status, _, _ = call(target, prefix + f"/cards/{other_number}.json", "PATCH", {"card": {"image": upload["signed_id"]}})
        assert status == 422, status
        image_path = card["image_url"]
        status, headers, _ = call(target, image_path, bearer=False)
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path.endswith("/session/new"), (status, headers)
        status, headers, _ = call(target, image_path)
        assert status == 302 and "/rails/active_storage/" in headers["Location"], (status, headers)
        with urllib.request.urlopen(urllib.parse.urljoin(target["url"], headers["Location"]), timeout=10) as response:
            assert response.status == 200 and response.read() == IMAGE
            assert response.headers.get_content_type() == "image/jpeg", response.headers
            assert response.headers.get("Content-Disposition", "").startswith("inline"), response.headers

        status, _, _ = call(target, prefix + f"/boards/{board['id']}/publication", "POST")
        assert status == 201, status
        status, headers, _ = call(target, image_path, bearer=False)
        assert status == 302 and "/rails/active_storage/" in headers["Location"], (status, headers)
        status, _, raw = call(target, prefix + f"/cards/{number}/image.json", "DELETE")
        assert status == 204 and raw == b"", (status, raw[:150])
        status, _, raw = call(target, prefix + f"/cards/{number}.json")
        assert status == 200 and json.loads(raw)["image_url"] is None, raw[:150]
        status, _, raw = call(target, prefix + "/rails/active_storage/direct_uploads", "POST", {"blob": blob})
        assert status == 200, (status, raw[:150])
        created_upload = json.loads(raw)
        status, _, raw = call(target, created_upload["direct_upload"]["url"], "PUT", IMAGE,
                              bearer=False, headers=created_upload["direct_upload"]["headers"])
        assert status == 204, (status, raw[:150])
        status, _, raw = call(target, prefix + f"/boards/{board['id']}/cards", "POST",
                              {"card": {"title": "Created with image", "image": created_upload["signed_id"]}})
        assert status == 201 and json.loads(raw)["image_url"], (status, raw[:150])
        return {"direct_upload": 200, "upload": 204, "attach": 200, "private_denial": 302,
                "authorized_redirect": 302, "public_redirect": 302, "delete": 204,
                "create_with_image": 201}
    finally:
        status, _, _ = call(target, prefix + f"/boards/{board['id']}", "DELETE")
        assert status == 204, status


def main():
    names = [name for name in ("FIZZY", "RIZZY") if os.environ.get(f"{name}_URL")]
    results = {}
    for name in names:
        target = {"url": os.environ[f"{name}_URL"], "account": os.environ[f"{name}_ACCOUNT"],
                  "token": os.environ[f"{name}_TOKEN"]}
        results[name] = run(target)
    if len(results) == 2:
        assert results["FIZZY"] == results["RIZZY"], results
    print(f"Card image flow passed on {', '.join(results)}")


if __name__ == "__main__":
    main()
