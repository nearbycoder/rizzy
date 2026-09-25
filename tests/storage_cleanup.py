#!/usr/bin/env python3
"""Check that deleting cards and boards purges their stored image bytes."""
import base64
import hashlib
import json
import os
import sqlite3
import time
from pathlib import Path

from card_image_parity import IMAGE, call


def check(status, raw, expected):
    assert status == expected, (status, raw[:200])
    return json.loads(raw) if raw else None


def upload_to_card(target, prefix, number):
    blob = {"filename": "moon.jpg", "byte_size": len(IMAGE),
            "checksum": base64.b64encode(hashlib.md5(IMAGE).digest()).decode(),
            "content_type": "image/jpeg"}
    status, _, raw = call(target, prefix + "/rails/active_storage/direct_uploads", "POST", {"blob": blob})
    upload = check(status, raw, 200)
    status, _, raw = call(target, upload["direct_upload"]["url"], "PUT", IMAGE,
                          bearer=False, headers=upload["direct_upload"]["headers"])
    check(status, raw, 204)
    status, _, raw = call(target, prefix + f"/cards/{number}.json", "PATCH",
                          {"card": {"image": upload["signed_id"]}})
    check(status, raw, 200)
    return upload


def attach(target, prefix, board_id):
    status, _, raw = call(target, prefix + f"/boards/{board_id}/cards", "POST",
                          {"card": {"title": "Storage cleanup"}})
    card = check(status, raw, 201)
    return card["number"], upload_to_card(target, prefix, card["number"])


def main():
    target = {"url": os.environ["RIZZY_URL"], "account": os.environ["RIZZY_ACCOUNT"],
              "token": os.environ["RIZZY_TOKEN"]}
    prefix = f"/{target['account']}"
    database = sqlite3.connect(os.environ["RIZZY_DB_PATH"])
    storage = Path(os.environ["RIZZY_STORAGE_DIR"])
    status, _, raw = call(target, prefix + "/boards", "POST",
                          {"board": {"name": f"Storage cleanup {time.time_ns()}"}})
    board = check(status, raw, 201)
    try:
        number, upload = attach(target, prefix, board["id"])
        assert (storage / upload["key"]).read_bytes() == IMAGE
        replacement = upload_to_card(target, prefix, number)
        assert database.execute("SELECT COUNT(*) FROM storage_blobs WHERE signed_id=?",
                                (upload["signed_id"],)).fetchone()[0] == 0
        assert not (storage / upload["key"]).exists()
        upload = replacement
        status, _, raw = call(target, prefix + f"/cards/{number}", "DELETE")
        check(status, raw, 204)
        assert database.execute("SELECT COUNT(*) FROM storage_blobs WHERE signed_id=?",
                                (upload["signed_id"],)).fetchone()[0] == 0
        assert not (storage / upload["key"]).exists()

        _, upload = attach(target, prefix, board["id"])
        assert (storage / upload["key"]).read_bytes() == IMAGE
        status, _, raw = call(target, prefix + f"/boards/{board['id']}", "DELETE")
        check(status, raw, 204)
        assert database.execute("SELECT COUNT(*) FROM storage_blobs WHERE signed_id=?",
                                (upload["signed_id"],)).fetchone()[0] == 0
        assert not (storage / upload["key"]).exists()
    finally:
        call(target, prefix + f"/boards/{board['id']}", "DELETE")
        database.close()
    print("Card and board image cleanup passed")


if __name__ == "__main__":
    main()
