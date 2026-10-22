#!/usr/bin/env python3
"""Check that Action Text image previews are purged with their original blob."""

import os
import sqlite3
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from sustained_mixed import prepare_rust, start_server, stop_server  # noqa: E402
from card_image_parity import IMAGE  # noqa: E402
from embed_parity import upload  # noqa: E402


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-variant-cleanup-") as temporary:
        directory = Path(temporary)
        env, credentials = prepare_rust(directory)
        process, url = start_server("rizzy", directory, env, sorted(os.sched_getaffinity(0))[:2])
        target = {"name": "rizzy", "url": url, "account": str(credentials["account"]),
                  "token": credentials["token"]}
        try:
            board, _ = expect(target, "/boards", "POST", {"board": {"name": "Variant cleanup"}}, 201)
            card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                             {"card": {"title": "Variant cleanup"}}, 201)
            blob = upload(target, IMAGE, "moon.jpg", "image/jpeg")
            description = (f'<p><action-text-attachment sgid="{blob["attachable_sgid"]}">'
                           '</action-text-attachment></p>')
            expect(target, f"/cards/{card['number']}", "PUT",
                   {"card": {"description": description}}, 200)
            database = sqlite3.connect(directory / "rizzy.sqlite3")
            rows = database.execute("""SELECT b.storage_key FROM storage_variant_records v
                JOIN storage_attachments a ON a.record_id=v.id
                JOIN storage_blobs b ON b.id=a.blob_id WHERE v.blob_id=?""", (blob["id"],)).fetchall()
            assert len(rows) == 2, rows
            storage = Path(env["RIZZY_STORAGE_DIR"])
            keys = [blob["key"], *(row[0] for row in rows)]
            assert all((storage / key).exists() for key in keys), keys
            expect(target, f"/cards/{card['number']}", "DELETE", status=204)
            assert database.execute("SELECT COUNT(*) FROM storage_variant_records WHERE blob_id=?",
                                    (blob["id"],)).fetchone()[0] == 0
            assert database.execute("SELECT COUNT(*) FROM storage_blobs WHERE id=?",
                                    (blob["id"],)).fetchone()[0] == 0
            assert all(not (storage / key).exists() for key in keys), keys
            database.close()
            print("Action Text preview record and file cleanup passed")
        finally:
            stop_server(process)


if __name__ == "__main__":
    main()
