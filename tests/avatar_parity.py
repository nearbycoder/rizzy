#!/usr/bin/env python3
"""Compare user avatar fallback, direct upload, thumbnail, and removal."""
import base64
import hashlib
import json
import os
import sqlite3
from pathlib import Path

from card_image_parity import IMAGE, call

PNG = (Path(__file__).resolve().parents[1] / "upstream/test/fixtures/files/avatar.png").read_bytes()
SVG = (Path(__file__).resolve().parents[1] / "upstream/test/fixtures/files/avatar.svg").read_bytes()


def run(target):
    prefix = f"/{target['account']}"
    status, _, raw = call(target, prefix + "/users")
    assert status == 200, (status, raw[:100])
    user = next(row for row in json.loads(raw) if row["role"] == "owner")
    path = prefix + f"/users/{user['id']}/avatar"
    status, headers, body = call(target, path, bearer=False)
    assert status == 200 and headers.get_content_type() == "image/svg+xml", (status, headers)
    assert b"<svg" in body and b"<text" in body, body[:100]
    etag = headers.get("ETag")
    assert etag
    status, _, _ = call(target, path, bearer=False, headers={"If-None-Match": etag})
    assert status == 304, status

    blob = {"filename": "moon.jpg", "byte_size": len(IMAGE),
            "checksum": base64.b64encode(hashlib.md5(IMAGE).digest()).decode(),
            "content_type": "image/jpeg"}
    status, _, raw = call(target, prefix + "/rails/active_storage/direct_uploads", "POST", {"blob": blob})
    assert status == 200, (status, raw[:100])
    upload = json.loads(raw)
    try:
        status, _, raw = call(target, upload["direct_upload"]["url"], "PUT", IMAGE,
                              bearer=False, headers=upload["direct_upload"]["headers"])
        assert status == 204, (status, raw[:100])
        status, _, raw = call(target, prefix + f"/users/{user['id']}.json", "PATCH",
                              {"user": {"avatar": upload["signed_id"]}})
        assert status == 200 and json.loads(raw)["avatar_url"].endswith(path), (status, raw[:150])

        status, headers, _ = call(target, path, bearer=False)
        assert status == 302 and "/representations/redirect/" in headers["Location"], (status, headers)
        status, headers, _ = call(target, headers["Location"], bearer=False)
        assert status == 302 and "/rails/active_storage/disk/" in headers["Location"], (status, headers)
        status, headers, thumbnail = call(target, headers["Location"], bearer=False)
        assert status == 200 and headers.get_content_type() == "image/jpeg", (status, headers)
        assert thumbnail.startswith(b"\xff\xd8") and thumbnail != IMAGE, len(thumbnail)

        original = prefix + "/rails/active_storage/blobs/redirect/" + upload["signed_id"] + "/moon.jpg"
        status, headers, _ = call(target, original, bearer=False)
        assert status == 302 and "/rails/active_storage/disk/" in headers["Location"], (status, headers)

        png_blob = {"filename": "avatar.png", "byte_size": len(PNG),
                    "checksum": base64.b64encode(hashlib.md5(PNG).digest()).decode(),
                    "content_type": "image/png"}
        status, _, raw = call(target, prefix + "/rails/active_storage/direct_uploads", "POST",
                              {"blob": png_blob})
        assert status == 200, (status, raw[:100])
        png_upload = json.loads(raw)
        status, _, raw = call(target, png_upload["direct_upload"]["url"], "PUT", PNG,
                              bearer=False, headers=png_upload["direct_upload"]["headers"])
        assert status == 204, (status, raw[:100])
        status, _, raw = call(target, prefix + f"/users/{user['id']}.json", "PATCH",
                              {"user": {"avatar": png_upload["signed_id"]}})
        assert status == 200, (status, raw[:150])
        if target.get("storage"):
            assert not (target["storage"] / upload["key"]).exists()
            with sqlite3.connect(target["database"]) as db:
                assert db.execute("SELECT COUNT(*) FROM storage_blobs WHERE signed_id=?",
                                  (upload["signed_id"],)).fetchone()[0] == 0
        status, headers, _ = call(target, path, bearer=False)
        assert status == 302, status
        status, headers, _ = call(target, headers["Location"], bearer=False)
        assert status == 302, status
        status, headers, thumbnail = call(target, headers["Location"], bearer=False)
        assert status == 200 and headers.get_content_type() == "image/png", (status, headers)
        assert thumbnail.startswith(b"\x89PNG\r\n\x1a\n")
        assert (int.from_bytes(thumbnail[16:20], "big"), int.from_bytes(thumbnail[20:24], "big")) == (256, 256)

        svg_blob = {"filename": "avatar.svg", "byte_size": len(SVG),
                    "checksum": base64.b64encode(hashlib.md5(SVG).digest()).decode(),
                    "content_type": "image/svg+xml"}
        status, _, raw = call(target, prefix + "/rails/active_storage/direct_uploads", "POST",
                              {"blob": svg_blob})
        assert status == 200, (status, raw[:100])
        svg_upload = json.loads(raw)
        status, _, raw = call(target, svg_upload["direct_upload"]["url"], "PUT", SVG,
                              bearer=False, headers=svg_upload["direct_upload"]["headers"])
        assert status == 204, (status, raw[:100])
        status, _, raw = call(target, prefix + f"/users/{user['id']}.json", "PATCH",
                              {"user": {"avatar": svg_upload["signed_id"]}})
        assert status == 422, (status, raw[:150])
        status, headers, _ = call(target, path, bearer=False)
        assert status == 302, status
        return {"initials": 200, "etag": 304, "attach": 200, "avatar": 302,
                "thumbnail_redirect": 302, "thumbnail": 200, "public_blob": 302,
                "png_thumbnail": 200, "invalid_svg": 422}
    finally:
        status, _, raw = call(target, path + ".json", "DELETE")
        assert status == 204, (status, raw[:100])
        status, headers, body = call(target, path, bearer=False)
        assert status == 200 and headers.get_content_type() == "image/svg+xml", (status, headers)
        if target.get("storage") and "png_upload" in locals():
            assert not (target["storage"] / png_upload["key"]).exists()
            with sqlite3.connect(target["database"]) as db:
                assert db.execute("SELECT COUNT(*) FROM storage_blobs WHERE signed_id=?",
                                  (png_upload["signed_id"],)).fetchone()[0] == 0


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        if not os.environ.get(f"{name}_URL"):
            continue
        target = {"url": os.environ[f"{name}_URL"],
                  "account": os.environ[f"{name}_ACCOUNT"],
                  "token": os.environ[f"{name}_TOKEN"]}
        if name == "RIZZY" and os.environ.get("RIZZY_STORAGE_DIR") and os.environ.get("RIZZY_DB_PATH"):
            target.update(storage=Path(os.environ["RIZZY_STORAGE_DIR"]),
                          database=os.environ["RIZZY_DB_PATH"])
        results[name] = run(target)
    if len(results) == 2:
        assert results["FIZZY"] == results["RIZZY"], results
    print(f"Avatar flow passed on {', '.join(results)}")


if __name__ == "__main__":
    main()
