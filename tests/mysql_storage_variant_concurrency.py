#!/usr/bin/env python3
"""Verify shared image variants through two Rust processes on disposable MySQL."""

import concurrent.futures
import os
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from card_image_parity import IMAGE  # noqa: E402
from embed_parity import upload  # noqa: E402
from sustained_mixed import prepare_rust, start_server, stop_server  # noqa: E402
from sustained_mixed_mysql import (create_databases, drop_databases, mysql_query, rust_env)  # noqa: E402

CLIENT = Path(os.environ.get("RIZZY_TEST_MYSQL_CLIENT", "/tmp/rizzy-mysql8/bin/mysql"))
SOCKET = Path(os.environ.get("RIZZY_TEST_MYSQL_SOCKET", "/tmp/rizzy-mysql8/mysql.sock"))
PORT = int(os.environ.get("MYSQL_PORT", "33080"))


def main():
    if not SOCKET.is_socket():
        raise SystemExit("Start the disposable MySQL server first")
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"], cwd=ROOT, check=True)
    database = "rzvariant_" + uuid.uuid4().hex[:12]
    create_databases(str(CLIENT), str(SOCKET), [database])
    try:
        with tempfile.TemporaryDirectory(prefix="rizzy-mysql-variant-") as temporary:
            directory = Path(temporary)
            env, credentials = prepare_rust(directory, rust_env(directory, "127.0.0.1", PORT, database))
            query = lambda statement: mysql_query(str(CLIENT), str(SOCKET), database, statement)
            assert query("SELECT COUNT(*) FROM storage_blobs") == "0"
            query("ALTER TABLE storage_blobs MODIFY COLUMN signed_id VARCHAR(64) NOT NULL")
            subprocess.run([ROOT / "target/release/rizzy", "init-db"], cwd=ROOT,
                           env=env, check=True, capture_output=True, text=True)
            assert query("SELECT character_maximum_length FROM information_schema.columns "
                         "WHERE table_schema=DATABASE() AND table_name='storage_blobs' "
                         "AND column_name='signed_id'") == "255"
            processes = []
            logs = []
            try:
                targets = []
                for index in range(2):
                    server_directory = directory / f"server-{index}"
                    server_directory.mkdir()
                    process, url = start_server("rizzy", server_directory, env,
                                                sorted(os.sched_getaffinity(0))[:4])
                    processes.append(process)
                    logs.append(server_directory / "rizzy.log")
                    targets.append({"name": "rizzy", "url": url,
                                    "account": str(credentials["account"]),
                                    "token": credentials["token"]})
                board, _ = expect(targets[0], "/boards", "POST",
                                  {"board": {"name": "MySQL variant concurrency"}}, 201)
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

                keys = query("SELECT b.storage_key FROM storage_variant_records v "
                             "JOIN storage_attachments a ON a.record_id=v.id "
                             "JOIN storage_blobs b ON b.id=a.blob_id "
                             f"WHERE v.blob_id='{blob['id']}'").splitlines()
                assert len(keys) == 2, keys
                storage = directory / "storage"
                assert all((storage / key).exists() for key in keys)
                assert query("SELECT COUNT(*) FROM storage_attachments "
                             f"WHERE blob_id='{blob['id']}'") == "8"

                for number in numbers[:-1]:
                    expect(targets[0], f"/cards/{number}", "DELETE", status=204)
                assert query("SELECT COUNT(*) FROM storage_variant_records "
                             f"WHERE blob_id='{blob['id']}'") == "2"
                expect(targets[1], f"/cards/{numbers[-1]}", "DELETE", status=204)
                assert query("SELECT COUNT(*) FROM storage_variant_records "
                             f"WHERE blob_id='{blob['id']}'") == "0"
                assert all(not (storage / key).exists() for key in keys)
                print("MySQL two-process variant creation and last-attachment cleanup passed")
            except Exception:
                for log in logs:
                    print(log.read_text()[-2400:], file=sys.stderr)
                raise
            finally:
                for process in reversed(processes):
                    stop_server(process)
    finally:
        drop_databases(str(CLIENT), str(SOCKET), [database])


if __name__ == "__main__":
    main()
