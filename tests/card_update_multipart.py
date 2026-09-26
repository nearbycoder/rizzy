#!/usr/bin/env python3
"""Exercise the pinned CardsController multipart image and Turbo update cases."""
import json
import sqlite3
import tempfile
import urllib.request
from pathlib import Path

from auth import stop
from draft_workflow_parity import login, request
from search_html_parity import ROOT, rust_target


def multipart(fields, image):
    boundary = "rizzy-card-update-boundary"
    body = bytearray()
    for name, value in fields.items():
        body.extend(f"--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n".encode())
    body.extend(f"--{boundary}\r\nContent-Disposition: form-data; name=\"card[image]\"; filename=\"moon.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n".encode())
    body.extend(image)
    body.extend(f"\r\n--{boundary}--\r\n".encode())
    return boundary, body


def update(target, number, fields, image, method="PATCH"):
    if method == "POST":
        fields = {"_method": "patch", **fields}
    boundary, body = multipart(fields, image)
    call = urllib.request.Request(
        f"{target['url']}/{target['account']}/cards/{number}",
        data=body,
        method=method,
        headers={"Accept": "text/vnd.turbo-stream.html",
                 "Content-Type": f"multipart/form-data; boundary={boundary}"},
    )
    with target["opener"].open(call, timeout=10) as response:
        raw = response.read().decode()
        assert response.status == 200, (response.status, raw[:200])
        assert response.headers.get_content_type() == "text/vnd.turbo-stream.html"
        return raw


def main():
    image = (ROOT / "upstream/test/fixtures/files/moon.jpg").read_bytes()
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        try:
            login(target)
            account = target["account"]
            status, _, raw = request(target, f"/{account}/boards", "POST",
                                     {"board": {"name": "Multipart card edits"}}, bearer=True)
            assert status == 201, (status, raw[:200])
            board = json.loads(raw)
            status, _, raw = request(target, f"/{account}/boards/{board['id']}/cards", "POST",
                                     {"card": {"title": "Original"}}, bearer=True)
            assert status == 201, (status, raw[:200])
            published = json.loads(raw)
            stream = update(target, published["number"],
                            {"card[title]": "Logo needs to change",
                             "card[description]": "Something more in-depth"}, image)
            for target_id in ("card_container_card_", "edit_card_", "card_closure_toggle_card_"):
                assert f'target="{target_id}{published["id"]}"' in stream
            assert "Logo needs to change" in stream and "Something more in-depth" in stream
            status, _, raw = request(target, f"/{account}/cards/{published['number']}.json", bearer=True)
            card = json.loads(raw)
            assert status == 200 and card["title"] == "Logo needs to change"
            assert card["image_url"] and card["description"] == "Something more in-depth"
            with sqlite3.connect(target["db"]) as db:
                filename, key, checksum = db.execute(
                    "SELECT b.filename,b.storage_key,b.checksum FROM storage_attachments a "
                    "JOIN storage_blobs b ON b.id=a.blob_id WHERE a.record_id=? AND a.name='image'",
                    (published["id"],)).fetchone()
                assert filename == "moon.jpg" and checksum
                assert (Path(temp) / "storage" / key).read_bytes() == image

            status, headers, _ = request(target, f"/{account}/boards/{board['id']}/cards",
                                         "POST", b"", accept="text/html")
            assert status == 302, status
            number = headers["Location"].split("/cards/")[1].split("/")[0]
            with sqlite3.connect(target["db"]) as db:
                draft_id = db.execute("SELECT id FROM cards WHERE number=?", (number,)).fetchone()[0]
            stream = update(target, number, {}, image, method="POST")
            assert "card-perma--draft" in stream and "reactions" not in stream
            with sqlite3.connect(target["db"]) as db:
                assert db.execute("SELECT b.filename FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.record_id=? AND a.name='image'", (draft_id,)).fetchone()[0] == "moon.jpg"
        finally:
            stop(process)
    print("Multipart card image edits and Turbo responses passed for published and draft cards")


if __name__ == "__main__":
    main()
