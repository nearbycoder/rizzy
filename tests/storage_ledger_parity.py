#!/usr/bin/env python3
"""Compare original-upload storage ledger effects with a pinned Fizzy server."""

import base64
import collections
import hashlib
import json
import os
import sqlite3
import time

from card_image_parity import IMAGE, call

FILE = b"storage ledger text\n"


def api(target, path, method="GET", payload=None, expected=200):
    status, _, raw = call(target, f"/{target['account']}{path}", method, payload)
    assert status == expected, (target["name"], method, path, status, raw[:300])
    return json.loads(raw) if raw else None


def upload(target, content, filename, content_type):
    blob = api(target, "/rails/active_storage/direct_uploads", "POST", {
        "blob": {
            "filename": filename,
            "byte_size": len(content),
            "checksum": base64.b64encode(hashlib.md5(content).digest()).decode(),
            "content_type": content_type,
        }
    })
    status, _, raw = call(target, blob["direct_upload"]["url"], "PUT", content,
                          bearer=False, headers=blob["direct_upload"]["headers"])
    assert status == 204, (target["name"], status, raw[:200])
    return blob


def ledger(target, board_names):
    with sqlite3.connect(target["db"]) as db:
        account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?",
                                (int(target["account"]),)).fetchone()[0]
        board_ids = {db.execute("SELECT id FROM boards WHERE name=?", (name,)).fetchone()[0]: label
                     for label, name in board_names.items()}
        rows = db.execute(
            "SELECT operation,delta,recordable_type,board_id,blob_id "
            "FROM storage_entries WHERE account_id=?", (account_id,)
        ).fetchall()
    entries = collections.Counter((operation, delta, record_type, board_ids[board_id], blob_id is not None)
                                  for operation, delta, record_type, board_id, blob_id in rows)
    return entries


def check(target):
    marker = f"storage-ledger-{time.time_ns()}"
    board_names = {"old": marker + " old", "new": marker + " new"}
    boards = {}
    try:
        for label, name in board_names.items():
            boards[label] = api(target, "/boards", "POST", {"board": {"name": name}}, 201)
        card = api(target, f"/boards/{boards['old']['id']}/cards", "POST",
                   {"card": {"title": marker}}, 201)
        number = card["number"]
        assert not ledger(target, board_names), target["name"]

        image = upload(target, IMAGE, "moon.jpg", "image/jpeg")
        api(target, f"/cards/{number}.json", "PATCH", {"card": {"image": image["signed_id"]}})
        card_file = upload(target, FILE, "card.txt", "text/plain")
        card_markup = f'<p><action-text-attachment sgid="{card_file["attachable_sgid"]}"></action-text-attachment></p>'
        api(target, f"/cards/{number}.json", "PATCH", {"card": {"description": card_markup}})
        comment_file = upload(target, FILE, "comment.txt", "text/plain")
        comment_markup = f'<p><action-text-attachment sgid="{comment_file["attachable_sgid"]}"></action-text-attachment></p>'
        api(target, f"/cards/{number}/comments", "POST", {"comment": {"body": comment_markup}}, 201)
        attached = ledger(target, board_names)
        assert sum(delta * count for (_, delta, _, _, _), count in attached.items()) == len(IMAGE) + 2 * len(FILE), (target["name"], attached)

        api(target, f"/cards/{number}/board", "PATCH", {"board_id": boards["new"]["id"]})
        moved = ledger(target, board_names)
        assert sum(delta * count for (_, delta, _, _, _), count in moved.items()) == len(IMAGE) + 2 * len(FILE), (target["name"], moved)
        assert sum(delta * count for (_, delta, _, board, _), count in moved.items() if board == "old") == 0, (target["name"], moved)

        api(target, f"/cards/{number}", "DELETE", expected=204)
        finished = ledger(target, board_names)
        assert sum(delta * count for (_, delta, _, _, _), count in finished.items()) == 0, (target["name"], finished)
        assert sum(delta * count for (_, delta, _, board, _), count in finished.items() if board == "new") == 0, (target["name"], finished)
        return finished
    finally:
        for board in boards.values():
            api(target, f"/boards/{board['id']}", "DELETE", expected=204)


def main():
    targets = []
    for name in ("FIZZY", "RIZZY"):
        targets.append({"name": name.lower(), "url": os.environ[f"{name}_URL"],
                        "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
                        "db": os.environ[f"{name}_DB_PATH"]})
    source, rust = (check(target) for target in targets)
    assert source == rust, (source, rust)
    print("Storage ledger parity passed: card image, card/comment embeds, board transfer, and deletion")


if __name__ == "__main__":
    main()
