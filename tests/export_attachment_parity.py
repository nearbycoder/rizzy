#!/usr/bin/env python3
"""Compare user-export JSON and original rich-text attachment ZIP entries."""
import io
import json
import os
import re
import time
import urllib.parse
import urllib.request
import zipfile

from card_image_parity import IMAGE, call
from embed_parity import FILE, upload


def request(target, path, method="GET", payload=None, expected=200):
    status, _, raw = call(target, f"/{target['account']}{path}", method, payload)
    assert status == expected, (method, path, status, raw[:160])
    return json.loads(raw) if raw else None


def normalize(value, number, blobs):
    for blob in blobs:
        value = value.replace(f"{number}/{blob['key']}_{blob['filename']}",
                              f"[CARD]/{blob['filename']}")
    return value


def download(target, url):
    absolute = urllib.parse.urljoin(target["url"], url)
    request = urllib.request.Request(absolute, headers={
        "Authorization": f"Bearer {target['token']}", "Accept": "application/zip"})
    with urllib.request.urlopen(request, timeout=30) as response:
        assert response.status == 200 and response.headers.get_content_type() == "application/zip"
        return response.read()


def run(target):
    board = request(target, "/boards", "POST", {"board": {"name": f"Export attachment parity {time.time_ns()}"}}, 201)
    try:
        image = upload(target)
        background = upload(target)
        description = f'<p>Sample <strong>bold</strong></p><p><action-text-attachment sgid="{image["attachable_sgid"]}"></action-text-attachment></p>'
        card = request(target, f"/boards/{board['id']}/cards", "POST",
                       {"card": {"title": "Export probe", "description": description,
                                 "image": background["signed_id"]}}, 201)
        file = upload(target, FILE, "notes.txt", "text/plain")
        comment_body = (f'<p>File <action-text-attachment sgid="{file["attachable_sgid"]}"></action-text-attachment>'
                        f' and <action-text-attachment sgid="{image["attachable_sgid"]}"></action-text-attachment></p>')
        comment = request(target, f"/cards/{card['number']}/comments", "POST",
                          {"comment": {"body": comment_body}}, 201)
        owner = next(row for row in request(target, "/users") if row["role"] == "owner")
        export = request(target, f"/users/{owner['id']}/data_exports", "POST", expected=201)
        for _ in range(160):
            status = request(target, f"/users/{owner['id']}/data_exports/{export['id']}")
            if status["status"] in ("completed", "failed"):
                break
            time.sleep(.25)
        assert status["status"] == "completed", status
        archive_bytes = download(target, status["download_url"])
        number = card["number"]
        with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
            names = [name for name in archive.namelist()
                     if name == f"{number}.json" or name.startswith(f"{number}/")]
            expected = {f"{number}.json", f"{number}/{image['key']}_moon.jpg",
                        f"{number}/{file['key']}_notes.txt"}
            assert set(names) == expected, names
            assert archive.read(f"{number}/{image['key']}_moon.jpg") == IMAGE
            assert archive.read(f"{number}/{file['key']}_notes.txt") == FILE
            assert all(archive.getinfo(name).compress_type == zipfile.ZIP_STORED
                       for name in names if name != f"{number}.json")
            payload = json.loads(archive.read(f"{number}.json"))
        selected = next(item for item in payload["comments"] if item["id"] == comment["id"])
        assert payload["title"] == "Export probe"
        assert background["key"] not in json.dumps(payload)
        return {"entries": ["[CARD].json", "[CARD]/moon.jpg", "[CARD]/notes.txt"],
                "description": normalize(payload["description"], number, (image, file)),
                "comment": normalize(selected["body"], number, (image, file)),
                "original_bytes": True, "stored_compression": True}
    finally:
        call(target, f"/{target['account']}/boards/{board['id']}", "DELETE")


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        if not os.environ.get(f"{name}_URL"):
            continue
        results[name] = run({"url": os.environ[f"{name}_URL"],
                             "account": os.environ[f"{name}_ACCOUNT"],
                             "token": os.environ[f"{name}_TOKEN"]})
    if len(results) == 2:
        assert results["FIZZY"] == results["RIZZY"], results
    print(f"User export attachment parity passed on {', '.join(results)}")


if __name__ == "__main__":
    main()
