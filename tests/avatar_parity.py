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
VARIANT_DIGESTS = {
    "jpg": "QCHtwKaNWtgBrxkcWm+AKwElcC4=",
    "png": "Ul0XHozM8YT3OtF7o1bEa1I7itc=",
    "gif": "jv+O+AdtkSYeg5C+IHiR/97I5NI=",
    "webp": "+6T9y7mPFOY1C9jjOyjmh065U5o=",
}


def preprocessed_avatar(target, upload, extension):
    if not target.get("database") and not target.get("mysql_database"):
        return None
    source = target.get("name") == "fizzy"
    blobs = "active_storage_blobs" if source else "storage_blobs"
    variants = "active_storage_variant_records" if source else "storage_variant_records"
    attachments = "active_storage_attachments" if source else "storage_attachments"
    key_column = "key" if source else "storage_key"
    query = f"""SELECT v.variation_digest, derived.byte_size, derived.checksum,
                    derived.metadata, derived.{key_column}
                 FROM {blobs} original JOIN {variants} v ON v.blob_id=original.id
                 JOIN {attachments} a ON a.record_type='ActiveStorage::VariantRecord'
                    AND a.record_id=v.id AND a.name='image'
                 JOIN {blobs} derived ON derived.id=a.blob_id
                 WHERE original.{key_column}=?"""
    if target.get("mysql_database"):
        from sustained_mixed_mysql import mysql_query
        assert upload["key"].isalnum()
        statement = query.replace("?", f"'{upload['key']}'")
        raw = mysql_query(target["mysql_client"], target["mysql_socket"],
                          target["mysql_database"], statement)
        rows = [row.split("\t") for row in raw.splitlines()] if raw else []
    else:
        with sqlite3.connect(target["database"]) as db:
            rows = db.execute(query, (upload["key"],)).fetchall()
    assert len(rows) == 1, (target.get("name"), extension, rows)
    digest, size, checksum, metadata, derived_key = rows[0]
    assert digest == VARIANT_DIGESTS[extension], (extension, digest)
    assert json.loads(metadata)["width"] == json.loads(metadata)["height"] == 256
    return int(size), checksum, derived_key


def assert_derived_bytes(derived, thumbnail):
    if derived:
        assert derived[:2] == (len(thumbnail), base64.b64encode(hashlib.md5(thumbnail).digest()).decode())


def assert_derived_removed(target, derived):
    if not derived or not target.get("storage"):
        return
    assert not (target["storage"] / derived[2]).exists()
    if target.get("mysql_database"):
        from sustained_mixed_mysql import mysql_query
        assert derived[2].isalnum()
        count = mysql_query(target["mysql_client"], target["mysql_socket"],
                            target["mysql_database"],
                            f"SELECT COUNT(*) FROM storage_blobs WHERE storage_key='{derived[2]}'")
        assert count == "0", count
    else:
        with sqlite3.connect(target["database"]) as db:
            assert db.execute("SELECT COUNT(*) FROM storage_blobs WHERE storage_key=?",
                              (derived[2],)).fetchone()[0] == 0


def assert_original_removed(target, upload):
    if not target.get("storage"):
        return
    assert not (target["storage"] / upload["key"]).exists()
    if target.get("mysql_database"):
        from sustained_mixed_mysql import mysql_query
        assert upload["key"].isalnum()
        count = mysql_query(target["mysql_client"], target["mysql_socket"],
                            target["mysql_database"],
                            f"SELECT COUNT(*) FROM storage_blobs WHERE storage_key='{upload['key']}'")
        assert count == "0", count
    else:
        with sqlite3.connect(target["database"]) as db:
            assert db.execute("SELECT COUNT(*) FROM storage_blobs WHERE signed_id=?",
                              (upload["signed_id"],)).fetchone()[0] == 0


def run(target):
    thumbnail_hashes = {}
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
        derived = preprocessed_avatar(target, upload, "jpg")

        status, headers, _ = call(target, path, bearer=False)
        assert status == 302 and "/representations/redirect/" in headers["Location"], (status, headers)
        assert headers.get("Content-Type") == "text/html; charset=utf-8", headers
        status, headers, _ = call(target, headers["Location"], bearer=False)
        assert status == 302 and "/rails/active_storage/disk/" in headers["Location"], (status, headers)
        assert headers.get("Content-Type") == "text/html; charset=utf-8", headers
        status, headers, thumbnail = call(target, headers["Location"], bearer=False)
        assert status == 200 and headers.get_content_type() == "image/jpeg", (status, headers)
        assert thumbnail.startswith(b"\xff\xd8") and thumbnail != IMAGE, len(thumbnail)
        assert_derived_bytes(derived, thumbnail)
        thumbnail_hashes["jpeg"] = hashlib.sha256(thumbnail).hexdigest()

        original = prefix + "/rails/active_storage/blobs/redirect/" + upload["signed_id"] + "/moon.jpg"
        status, headers, _ = call(target, original, bearer=False)
        assert status == 302 and "/rails/active_storage/disk/" in headers["Location"], (status, headers)
        assert headers.get("Content-Type") == "text/html; charset=utf-8", headers

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
        derived = preprocessed_avatar(target, png_upload, "png")
        status, headers, _ = call(target, path, bearer=False)
        assert status == 302, status
        status, headers, _ = call(target, headers["Location"], bearer=False)
        assert status == 302, status
        status, headers, thumbnail = call(target, headers["Location"], bearer=False)
        assert status == 200 and headers.get_content_type() == "image/png", (status, headers)
        assert thumbnail.startswith(b"\x89PNG\r\n\x1a\n")
        assert (int.from_bytes(thumbnail[16:20], "big"), int.from_bytes(thumbnail[20:24], "big")) == (256, 256)
        assert_derived_bytes(derived, thumbnail)
        thumbnail_hashes["png"] = hashlib.sha256(thumbnail).hexdigest()

        for filename, content_type, signature in (
            ("animated.gif", "image/gif", b"GIF89a"),
            ("animated.webp", "image/webp", b"RIFF"),
        ):
            data = (Path(__file__).resolve().parent / "fixtures" / filename).read_bytes()
            animated_blob = {"filename": filename, "byte_size": len(data),
                             "checksum": base64.b64encode(hashlib.md5(data).digest()).decode(),
                             "content_type": content_type}
            status, _, raw = call(target, prefix + "/rails/active_storage/direct_uploads", "POST",
                                  {"blob": animated_blob})
            assert status == 200, (status, raw[:100])
            animated_upload = json.loads(raw)
            status, _, raw = call(target, animated_upload["direct_upload"]["url"], "PUT", data,
                                  bearer=False, headers=animated_upload["direct_upload"]["headers"])
            assert status == 204, (status, raw[:100])
            status, _, raw = call(target, prefix + f"/users/{user['id']}.json", "PATCH",
                                  {"user": {"avatar": animated_upload["signed_id"]}})
            assert status == 200, (status, raw[:100])
            derived = preprocessed_avatar(target, animated_upload, filename.rsplit(".", 1)[1])
            status, headers, _ = call(target, path, bearer=False)
            assert status == 302 and headers.get("Content-Type") == "text/html; charset=utf-8", (status, headers)
            status, headers, _ = call(target, headers["Location"], bearer=False)
            assert status == 302 and headers.get("Content-Type") == "text/html; charset=utf-8", (status, headers)
            status, headers, thumbnail = call(target, headers["Location"], bearer=False)
            assert status == 200 and headers.get_content_type() == content_type, (status, headers)
            assert thumbnail.startswith(signature) and thumbnail != data, len(thumbnail)
            assert_derived_bytes(derived, thumbnail)
            if filename.endswith(".gif"):
                assert (int.from_bytes(thumbnail[6:8], "little"),
                        int.from_bytes(thumbnail[8:10], "little")) == (256, 256)
            else:
                assert thumbnail[8:12] == b"WEBP"
            thumbnail_hashes[filename] = hashlib.sha256(thumbnail).hexdigest()

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
                "png_thumbnail": 200, "animated_gif_thumbnail": 200,
                "animated_webp_thumbnail": 200, "invalid_svg": 422,
                "thumbnail_sha256": thumbnail_hashes}
    finally:
        status, _, raw = call(target, path + ".json", "DELETE")
        assert status == 204, (status, raw[:100])
        status, headers, body = call(target, path, bearer=False)
        assert status == 200 and headers.get_content_type() == "image/svg+xml", (status, headers)


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        if not os.environ.get(f"{name}_URL"):
            continue
        target = {"url": os.environ[f"{name}_URL"],
                  "name": name.lower(), "account": os.environ[f"{name}_ACCOUNT"],
                  "token": os.environ[f"{name}_TOKEN"]}
        if os.environ.get(f"{name}_DB_PATH"):
            target["database"] = Path(os.environ[f"{name}_DB_PATH"])
        if name == "RIZZY" and os.environ.get("RIZZY_STORAGE_DIR") and os.environ.get("RIZZY_DB_PATH"):
            target.update(storage=Path(os.environ["RIZZY_STORAGE_DIR"]),
                          database=os.environ["RIZZY_DB_PATH"])
        results[name] = run(target)
    if len(results) == 2:
        assert results["FIZZY"] == results["RIZZY"], results
    print(f"Avatar flow passed on {', '.join(results)}")


if __name__ == "__main__":
    main()
