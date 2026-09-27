#!/usr/bin/env python3
"""Compare Action Text image embeds and their storage permissions."""
import base64
import hashlib
import json
import os
import re
import sqlite3
import time
import urllib.parse
import urllib.request
from pathlib import Path

from card_image_parity import IMAGE, call
from avatar_parity import PNG, SVG

FILE = b"hello text\n"
BMP = (Path(__file__).resolve().parents[1] / "tests/fixtures/tiny.bmp").read_bytes()


def request(target, path, method="GET", payload=None, *, bearer=True):
    status, headers, raw = call(target, f"/{target['account']}{path}", method, payload, bearer=bearer)
    return status, headers, json.loads(raw) if raw and headers.get_content_type() == "application/json" else raw


def upload(target, content=IMAGE, filename="moon.jpg", content_type="image/jpeg"):
    blob = {"filename": filename, "byte_size": len(content),
            "checksum": base64.b64encode(hashlib.md5(content).digest()).decode(),
            "content_type": content_type}
    status, _, result = request(target, "/rails/active_storage/direct_uploads", "POST", {"blob": blob})
    assert status == 200, (status, result)
    assert result["signed_id"] != result["attachable_sgid"]
    status, _, raw = call(target, result["direct_upload"]["url"], "PUT", content,
                          bearer=False, headers=result["direct_upload"]["headers"])
    assert status == 204 and raw == b"", (status, raw[:150])
    return result


def normalized(html, target, blob):
    html = html.replace(blob["signed_id"], "[SIGNED]")
    html = html.replace(blob["attachable_sgid"], "[SGID]")
    html = re.sub(r"(/representations/redirect/\[SIGNED\]/)[^/]+/", r"\1[VARIANT]/", html)
    return html.replace(f"/{target['account']}/", "/[ACCOUNT]/")


def assert_blob_access(target, blob, *, public=False):
    path = f"/{target['account']}/rails/active_storage/blobs/redirect/{blob['signed_id']}/moon.jpg"
    status, headers, _ = call(target, path, bearer=False)
    if public:
        assert status == 302 and "/rails/active_storage/" in headers["Location"], status
    else:
        assert status == 302 and urllib.parse.urlparse(headers["Location"]).path.endswith("/session/new"), status
    status, headers, _ = call(target, path)
    assert status == 302 and "/rails/active_storage/" in headers["Location"], status
    with urllib.request.urlopen(urllib.parse.urljoin(target["url"], headers["Location"]), timeout=10) as response:
        assert response.status == 200 and response.read() == IMAGE
        assert response.headers.get_content_type() == "image/jpeg"
    status, headers, _ = call(target, path + "?disposition=attachment")
    assert status == 302, status
    with urllib.request.urlopen(urllib.parse.urljoin(target["url"], headers["Location"]), timeout=10) as response:
        assert response.read() == IMAGE
        assert response.headers["Content-Disposition"].startswith("attachment"), response.headers


def run(target):
    status, _, board = request(target, "/boards", "POST", {"board": {"name": f"Embed parity {time.time_ns()}"}})
    assert status == 201, (status, board)
    results = {}
    board_blob = None
    try:
        blob = upload(target)
        # A blob's signed ID is not its Action Text attachable SGID.
        invalid = f'<p>Wrong <action-text-attachment sgid="{blob["signed_id"]}"></action-text-attachment></p>'
        status, _, card = request(target, f"/boards/{board['id']}/cards", "POST",
                                  {"card": {"title": "Invalid embed", "description": invalid}})
        assert status == 201, (status, card)
        results["invalid"] = (card["description"], card["has_attachments"],
                              normalized(card["description_html"], target, blob))

        markup = f'<p>See file</p><p><action-text-attachment sgid="{blob["attachable_sgid"]}"></action-text-attachment></p>'
        status, _, card = request(target, f"/boards/{board['id']}/cards", "POST",
                                  {"card": {"title": "Valid embed", "description": markup}})
        assert status == 201, (status, card)
        assert card["has_attachments"] is True
        results["card"] = (card["description"], card["has_attachments"],
                           normalized(card["description_html"], target, blob))
        assert_blob_access(target, blob)

        # The representation URL in rendered HTML must be a usable image URL.
        preview = re.search(r'href="([^" ]*/representations/redirect/[^" ]+)"', card["description_html"])
        assert preview, card["description_html"]
        status, headers, _ = call(target, preview.group(1))
        assert status == 302, status
        with urllib.request.urlopen(urllib.parse.urljoin(target["url"], headers["Location"]), timeout=10) as response:
            image = response.read()
            assert response.status == 200 and response.headers.get_content_type() == "image/jpeg"
            assert image[:2] == b"\xff\xd8"
            assert image != IMAGE, "Preview should be a processed representation"
        results["preview"] = (status, len(image) > 0)

        status, _, comment = request(target, f"/cards/{card['number']}/comments", "POST",
                                     {"comment": {"body": markup}})
        assert status == 201, (status, comment)
        results["comment"] = (comment["body"]["plain_text"],
                              normalized(comment["body"]["html"], target, blob))
        file_blob = upload(target, FILE, "notes.txt", "text/plain")
        file_markup = f'<p>Read <action-text-attachment sgid="{file_blob["attachable_sgid"]}"></action-text-attachment></p>'
        status, _, file_card = request(target, f"/boards/{board['id']}/cards", "POST",
                                       {"card": {"title": "File embed", "description": file_markup}})
        assert status == 201, (status, file_card)
        results["file"] = (file_card["description"], file_card["has_attachments"],
                           normalized(file_card["description_html"], target, file_blob))
        for label, content, filename, content_type in (
            ("svg", SVG, "avatar.svg", "image/svg+xml"),
            ("bmp", BMP, "tiny.bmp", "image/bmp"),
        ):
            nonvariable_blob = upload(target, content, filename, content_type)
            markup = (f'<p><action-text-attachment sgid="{nonvariable_blob["attachable_sgid"]}">'
                      '</action-text-attachment></p>')
            status, _, nonvariable_card = request(
                target, f"/boards/{board['id']}/cards", "POST",
                {"card": {"title": f"{label.upper()} embed", "description": markup}})
            assert status == 201, (status, nonvariable_card)
            assert "attachment--file" in nonvariable_card["description_html"]
            assert "/representations/" not in nonvariable_card["description_html"]
            results[label] = (nonvariable_card["description"], nonvariable_card["has_attachments"],
                              normalized(nonvariable_card["description_html"], target, nonvariable_blob))
            path = (f"/{target['account']}/rails/active_storage/blobs/redirect/"
                    f"{nonvariable_blob['signed_id']}/{filename}")
            status, headers, _ = call(target, path)
            assert status == 302, status
            with urllib.request.urlopen(urllib.parse.urljoin(target["url"], headers["Location"]), timeout=10) as response:
                assert response.read() == content
                downloaded_type = response.headers.get_content_type()
                assert downloaded_type == ("application/octet-stream" if label == "svg" else "image/bmp")
                results[f"{label}_download_type"] = downloaded_type
        png_blob = upload(target, PNG, "avatar.png", "image/png")
        png_markup = f'<p><action-text-attachment sgid="{png_blob["attachable_sgid"]}"></action-text-attachment></p>'
        status, _, png_card = request(target, f"/boards/{board['id']}/cards", "POST",
                                      {"card": {"title": "PNG embed", "description": png_markup}})
        assert status == 201, (status, png_card)
        png_preview = re.search(r'href="([^" ]*/representations/redirect/[^" ]+)"', png_card["description_html"])
        assert png_preview
        status, headers, _ = call(target, png_preview.group(1))
        assert status == 302
        with urllib.request.urlopen(urllib.parse.urljoin(target["url"], headers["Location"]), timeout=10) as response:
            image = response.read()
            assert response.headers.get_content_type() == "image/png"
            assert image.startswith(b"\x89PNG\r\n\x1a\n") and image != PNG
            png_dimensions = tuple(int.from_bytes(image[offset:offset + 4], "big") for offset in (16, 20))
        results["png"] = (png_card["description"], png_card["has_attachments"],
                          normalized(png_card["description_html"], target, png_blob), png_dimensions)
        bare_blob = upload(target)
        bare_markup = f'<action-text-attachment sgid="{bare_blob["attachable_sgid"]}"></action-text-attachment>'
        status, _, bare_card = request(target, f"/boards/{board['id']}/cards", "POST",
                                       {"card": {"title": "Bare embed", "description": bare_markup}})
        assert status == 201, (status, bare_card)
        results["bare"] = (bare_card["description"], bare_card["has_attachments"],
                           normalized(bare_card["description_html"], target, bare_blob))
        status, _, _ = request(target, f"/cards/{bare_card['number']}", "DELETE")
        assert status == 204, status
        if target.get("database"):
            assert_deleted(target, bare_blob)
        status, _, updated = request(target, f"/cards/{card['number']}.json", "PATCH",
                                     {"card": {"description": "Removed image"}})
        assert status == 200, (status, updated)
        results["removed"] = (updated["description"], updated["has_attachments"])
        # The comment still owns the blob, even after removal from the card.
        assert_blob_access(target, blob)

        status, _, _ = request(target, f"/boards/{board['id']}/publication", "POST")
        assert status == 201, status
        assert_blob_access(target, blob, public=True)
        status, _, _ = request(target, f"/cards/{card['number']}/comments/{comment['id']}", "DELETE")
        assert status == 204, status
        if target.get("database"):
            db = sqlite3.connect(target["database"])
            try:
                assert db.execute("SELECT COUNT(*) FROM storage_blobs WHERE signed_id=?",
                                  (blob["signed_id"],)).fetchone()[0] == 0
                assert not (Path(target["storage_dir"]) / blob["key"]).exists()
                assert not (Path(target["storage_dir"]) / (blob["key"] + ".large.jpg")).exists()
            finally:
                db.close()
        results["delete_comment"] = status
        status, _, _ = request(target, f"/cards/{file_card['number']}", "DELETE")
        assert status == 204, status
        if target.get("database"):
            assert_deleted(target, file_blob)
        results["delete_card"] = status
        board_blob = upload(target)
        board_markup = f'<p><action-text-attachment sgid="{board_blob["attachable_sgid"]}"></action-text-attachment></p>'
        status, _, _ = request(target, f"/boards/{board['id']}/cards", "POST",
                               {"card": {"title": "Board cleanup", "description": board_markup}})
        assert status == 201, status
        return results
    finally:
        status, _, _ = request(target, f"/boards/{board['id']}", "DELETE")
        assert status == 204, status
        if target.get("database") and board_blob:
            assert_deleted(target, board_blob)


def assert_deleted(target, blob):
    db = sqlite3.connect(target["database"])
    try:
        assert db.execute("SELECT COUNT(*) FROM storage_blobs WHERE signed_id=?",
                          (blob["signed_id"],)).fetchone()[0] == 0
        assert not (Path(target["storage_dir"]) / blob["key"]).exists()
        assert not (Path(target["storage_dir"]) / (blob["key"] + ".large.jpg")).exists()
    finally:
        db.close()


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        if not os.environ.get(f"{name}_URL"):
            continue
        target = {"url": os.environ[f"{name}_URL"], "account": os.environ[f"{name}_ACCOUNT"],
                  "token": os.environ[f"{name}_TOKEN"]}
        if name == "RIZZY" and os.environ.get("RIZZY_DB_PATH"):
            target["database"] = os.environ["RIZZY_DB_PATH"]
            target["storage_dir"] = os.environ["RIZZY_STORAGE_DIR"]
        results[name] = run(target)
    if len(results) == 2:
        assert results["FIZZY"] == results["RIZZY"], json.dumps(results, indent=2)
    print(f"Action Text embed parity passed on {', '.join(results)}")


if __name__ == "__main__":
    main()
