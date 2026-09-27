#!/usr/bin/env python3
"""Check MySQL storage ledger, reconciliation, and concurrent totals workers."""

import concurrent.futures
import os
import re
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

from card_image_parity import IMAGE
from mysql_webhook_cleanup import SOCKET, root_mysql
from mysql_webhooks import BINARY, ROOT, start, stop
from storage_ledger_parity import FILE, api, upload


def main():
    if not SOCKET.is_socket():
        raise SystemExit("Start the disposable MySQL 8 server first")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    suffix = uuid.uuid4().hex[:12]
    database = "rzstorage_" + suffix
    username = "rzstorage_" + suffix
    password = uuid.uuid4().hex
    root_mysql(
        f"CREATE DATABASE {database} CHARACTER SET utf8mb4 COLLATE utf8mb4_bin; "
        f"CREATE USER '{username}'@'127.0.0.1' IDENTIFIED BY '{password}'; "
        f"GRANT ALL PRIVILEGES ON {database}.* TO '{username}'@'127.0.0.1';"
    )
    try:
        def scalar(statement):
            return root_mysql(f"USE {database}; {statement}")

        def total(owner_type, owner_id):
            value = scalar(
                "SELECT bytes_stored FROM storage_totals WHERE "
                f"owner_type='{owner_type}' AND owner_id='{owner_id}';"
            )
            return int(value) if value else None

        def ledger(account_id, board_id=None):
            scope = f"board_id='{board_id}'" if board_id else f"account_id='{account_id}'"
            return int(scalar(
                "SELECT CAST(COALESCE(SUM(delta),0) AS SIGNED) "
                f"FROM storage_entries WHERE {scope};"
            ))

        with tempfile.TemporaryDirectory(prefix="rizzy-mysql-storage-") as temporary:
            storage = Path(temporary) / "files"
            env = {
                **os.environ,
                "RIZZY_DATABASE_URL": f"mysql://{username}:{password}@127.0.0.1:33080/{database}",
                "RIZZY_STORAGE_DIR": str(storage),
                "RIZZY_SEED_EMAIL": "mysql-storage-" + suffix + "@example.test",
                "RIZZY_DISABLE_STORAGE_MATERIALIZATION": "1",
                "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
                "RIZZY_DISABLE_PUSH_DELIVERY": "1",
            }
            seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                    capture_output=True, text=True, check=True).stdout
            account = re.search(r"Account: (\d+)", seeded).group(1)
            token = re.search(r"Bearer token: (\w+)", seeded).group(1)
            account_id = scalar(
                f"SELECT id FROM accounts WHERE external_account_id={account};"
            )
            base, server, log = start(env, temporary, "storage")
            target = {"name": "rizzy", "url": base, "account": account, "token": token}

            def materialize(workers=2):
                jobs = [subprocess.Popen([BINARY, "materialize-storage"], cwd=ROOT,
                                         env=env, stdout=subprocess.PIPE,
                                         stderr=subprocess.PIPE, text=True)
                        for _ in range(workers)]
                for job in jobs:
                    out, err = job.communicate(timeout=30)
                    assert job.returncode == 0, (out, err, log.read_text()[-1600:])

            def reconcile():
                job = subprocess.run([BINARY, "reconcile-storage"], cwd=ROOT, env=env,
                                     capture_output=True, text=True)
                assert job.returncode == 0, (job.stdout, job.stderr)

            try:
                old = api(target, "/boards", "POST", {"board": {"name": "Storage old"}}, 201)
                new = api(target, "/boards", "POST", {"board": {"name": "Storage new"}}, 201)
                card = api(target, f"/boards/{old['id']}/cards", "POST",
                           {"card": {"title": "Moved image"}}, 201)
                image = upload(target, IMAGE, "moon.jpg", "image/jpeg")
                api(target, f"/cards/{card['number']}.json", "PATCH",
                    {"card": {"image": image["signed_id"]}})
                card_file = upload(target, FILE, "card.txt", "text/plain")
                card_markup = (f'<p><action-text-attachment sgid="{card_file["attachable_sgid"]}">'
                               "</action-text-attachment></p>")
                api(target, f"/cards/{card['number']}.json", "PATCH",
                    {"card": {"description": card_markup}})
                comment_file = upload(target, FILE, "comment.txt", "text/plain")
                comment_markup = (
                    f'<p><action-text-attachment sgid="{comment_file["attachable_sgid"]}">'
                    "</action-text-attachment></p>"
                )
                api(target, f"/cards/{card['number']}/comments", "POST",
                    {"comment": {"body": comment_markup}}, 201)
                attached_bytes = len(IMAGE) + 2 * len(FILE)
                assert ledger(account_id) == ledger(account_id, old["id"]) == attached_bytes
                assert total("Account", account_id) is None
                materialize()
                assert total("Account", account_id) == total("Board", old["id"]) == attached_bytes
                first_cursor = scalar(
                    "SELECT last_entry_id FROM storage_totals WHERE "
                    f"owner_type='Account' AND owner_id='{account_id}';"
                )
                materialize()
                assert scalar(
                    "SELECT last_entry_id FROM storage_totals WHERE "
                    f"owner_type='Account' AND owner_id='{account_id}';"
                ) == first_cursor

                api(target, f"/cards/{card['number']}/board", "PATCH",
                    {"board_id": new["id"]})
                assert ledger(account_id) == ledger(account_id, new["id"]) == attached_bytes
                assert ledger(account_id, old["id"]) == 0
                transfers = scalar(
                    "SELECT CONCAT(operation,':',recordable_type,':',delta) "
                    "FROM storage_entries WHERE operation IN ('transfer_out','transfer_in') "
                    "ORDER BY operation,recordable_type;"
                ).splitlines()
                expected_transfers = sorted([
                    f"transfer_out:Card:{-(len(IMAGE) + len(FILE))}",
                    f"transfer_out:Comment:{-len(FILE)}",
                    f"transfer_in:Card:{len(IMAGE) + len(FILE)}",
                    f"transfer_in:Comment:{len(FILE)}",
                ])
                assert sorted(transfers) == expected_transfers, transfers
                materialize()
                assert total("Account", account_id) == total("Board", new["id"]) == attached_bytes
                assert total("Board", old["id"]) == 0

                api(target, f"/cards/{card['number']}", "DELETE", expected=204)
                assert ledger(account_id) == ledger(account_id, new["id"]) == 0
                materialize()
                assert total("Account", account_id) == total("Board", new["id"]) == 0
                assert not (storage / image["key"]).exists()
                assert not (storage / card_file["key"]).exists()
                assert not (storage / comment_file["key"]).exists()

                board_file = upload(target, FILE, "board.txt", "text/plain")
                markup = (f'<p><action-text-attachment sgid="{board_file["attachable_sgid"]}">'
                          "</action-text-attachment></p>")
                api(target, f"/boards/{old['id']}", "PATCH",
                    {"board": {"public_description": markup}})
                materialize()
                assert total("Account", account_id) == total("Board", old["id"]) == len(FILE)

                backfill = api(target, "/boards", "POST",
                               {"board": {"name": "Storage backfill"}}, 201)
                missing = upload(target, FILE, "missing.txt", "text/plain")
                missing_markup = (f'<p><action-text-attachment sgid="{missing["attachable_sgid"]}">'
                                  "</action-text-attachment></p>")
                api(target, f"/boards/{backfill['id']}", "PATCH",
                    {"board": {"public_description": missing_markup}})
                assert ledger(account_id, backfill["id"]) == len(FILE)
                scalar(
                    "DELETE FROM storage_entries WHERE operation='attach' "
                    f"AND recordable_type='Board' AND board_id='{backfill['id']}';"
                )
                assert ledger(account_id, backfill["id"]) == 0
                reconcile()
                assert ledger(account_id, backfill["id"]) == len(FILE)
                before = scalar("SELECT COUNT(*) FROM storage_entries WHERE operation='reconcile';")
                reconcile()
                assert scalar("SELECT COUNT(*) FROM storage_entries WHERE operation='reconcile';") == before
                materialize()
                assert total("Account", account_id) == 2 * len(FILE)
                assert total("Board", backfill["id"]) == len(FILE)

                api(target, f"/boards/{old['id']}", "DELETE", expected=204)
                materialize()
                assert total("Board", old["id"]) is None
                assert total("Account", account_id) == len(FILE)
                api(target, f"/boards/{backfill['id']}", "DELETE", expected=204)
                materialize()
                assert total("Board", backfill["id"]) is None
                assert total("Account", account_id) == 0
                api(target, f"/boards/{new['id']}", "DELETE", expected=204)
                materialize()
                assert total("Board", new["id"]) is None
                assert total("Account", account_id) == 0
                assert not (storage / board_file["key"]).exists()
                assert not (storage / missing["key"]).exists()

                parallel = api(target, "/boards", "POST",
                               {"board": {"name": "Storage parallel"}}, 201)
                numbers = [api(target, f"/boards/{parallel['id']}/cards", "POST",
                               {"card": {"title": f"Parallel attachment {index}"}}, 201)["number"]
                           for index in range(16)]

                def attach(index_number):
                    index, number = index_number
                    blob = upload(target, FILE, f"parallel-{index}.txt", "text/plain")
                    text = (f'<p><action-text-attachment sgid="{blob["attachable_sgid"]}">'
                            "</action-text-attachment></p>")
                    api(target, f"/cards/{number}.json", "PATCH",
                        {"card": {"description": text}})
                    return blob["key"]

                with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
                    keys = list(pool.map(attach, enumerate(numbers)))
                assert ledger(account_id, parallel["id"]) == 16 * len(FILE)
                materialize()
                assert total("Account", account_id) == 16 * len(FILE)
                assert total("Board", parallel["id"]) == 16 * len(FILE)
                api(target, f"/boards/{parallel['id']}", "DELETE", expected=204)
                materialize()
                assert total("Board", parallel["id"]) is None
                assert total("Account", account_id) == 0
                assert all(not (storage / key).exists() for key in keys)
            except Exception:
                print(log.read_text()[-4000:], file=sys.stderr)
                raise
            finally:
                stop(server)
    finally:
        root_mysql(f"DROP DATABASE IF EXISTS {database}; DROP USER IF EXISTS '{username}'@'127.0.0.1';")
    print("MySQL storage passed: ledger, transfer, concurrent attachments and totals, reconcile, cleanup")


if __name__ == "__main__":
    main()
