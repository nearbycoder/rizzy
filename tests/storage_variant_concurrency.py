#!/usr/bin/env python3
"""Exercise one embedded image across simultaneous writes to two Rizzy processes."""

import concurrent.futures
import os
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from card_image_parity import IMAGE  # noqa: E402
from embed_parity import upload  # noqa: E402
from sustained_mixed import prepare_rust, rust_env, start_server, stop_server  # noqa: E402


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-variant-concurrency-") as temporary:
        directory = Path(temporary)
        env, credentials = prepare_rust(directory, {**rust_env(directory),
                                                    "RIZZY_DISABLE_STORAGE_PURGE": "1"})
        processes = []
        try:
            targets = []
            for _ in range(2):
                process, url = start_server("rizzy", directory, env,
                                            sorted(os.sched_getaffinity(0))[:4])
                processes.append(process)
                targets.append({"name": "rizzy", "url": url,
                                "account": str(credentials["account"]),
                                "token": credentials["token"]})
            board, _ = expect(targets[0], "/boards", "POST",
                              {"board": {"name": "Variant concurrency"}}, 201)
            blob = upload(targets[0], IMAGE, "moon.jpg", "image/jpeg")
            numbers = []
            for index in range(8):
                card, _ = expect(targets[0], f"/boards/{board['id']}/cards", "POST",
                                 {"card": {"title": f"Variant {index}"}}, 201)
                numbers.append(card["number"])
            description = (f'<p><action-text-attachment sgid="{blob["attachable_sgid"]}">'
                           '</action-text-attachment></p>')

            def attach(index):
                target = targets[index % 2]
                card, _ = expect(target, f"/cards/{numbers[index]}", "PUT",
                                 {"card": {"description": description}}, 200)
                assert card["has_attachments"]

            with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
                list(pool.map(attach, range(8)))

            database = sqlite3.connect(directory / "rizzy.sqlite3")
            storage = directory / "storage"
            rows = database.execute("""SELECT b.storage_key FROM storage_variant_records v
                JOIN storage_attachments a ON a.record_id=v.id
                JOIN storage_blobs b ON b.id=a.blob_id WHERE v.blob_id=?""", (blob["id"],)).fetchall()
            assert len(rows) == 2, rows
            derived_keys = [row[0] for row in rows]
            assert all((storage / key).exists() for key in derived_keys)
            originals = database.execute("SELECT COUNT(*) FROM storage_attachments WHERE blob_id=?",
                                         (blob["id"],)).fetchone()[0]
            assert originals == 8, originals

            for number in numbers[:-1]:
                expect(targets[0], f"/cards/{number}", "DELETE", status=204)
            assert database.execute("SELECT COUNT(*) FROM storage_variant_records WHERE blob_id=?",
                                    (blob["id"],)).fetchone()[0] == 2
            expect(targets[1], f"/cards/{numbers[-1]}", "DELETE", status=204)
            assert database.execute("SELECT COUNT(*) FROM storage_variant_records WHERE blob_id=?",
                                    (blob["id"],)).fetchone()[0] == 2
            assert database.execute("SELECT COUNT(*) FROM storage_purge_jobs WHERE state='pending'").fetchone()[0] == 1
            assert all((storage / key).exists() for key in derived_keys)
            result = subprocess.run([str(ROOT / "target/release/rizzy"), "purge-stored-blobs"],
                                    env=env, capture_output=True, text=True)
            assert result.returncode == 0, result.stderr[-1000:]
            assert database.execute("SELECT COUNT(*) FROM storage_variant_records WHERE blob_id=?",
                                    (blob["id"],)).fetchone()[0] == 0
            assert all(not (storage / key).exists() for key in derived_keys)
            database.close()
            print("Two-process variant creation and last-attachment cleanup passed")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
