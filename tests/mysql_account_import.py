#!/usr/bin/env python3
"""Restore a populated account ZIP into a dedicated MySQL database.

Set RIZZY_TEST_MYSQL_URL and RIZZY_TEST_MYSQL_CLIENT. An optional ZIP argument
can supply a Fizzy export; otherwise the existing Rizzy export fixture is used.
"""

import json
import os
import re
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
import zipfile
from collections import Counter
from pathlib import Path

from account_import import await_result, cross_account_reference, upload
from mysql_core import mysql_scalar


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"
GROUP_TABLES = {
    "user_settings": "user_settings_records",
    "accesses": "access_records",
    "entropies": "entropy_records",
    "active_storage_blobs": "storage_blobs",
    "active_storage_attachments": "storage_attachments",
    "active_storage_variant_records": "storage_variant_records",
    "action_text_rich_texts": "rich_text_records",
}
CARD_SCOPED_GROUPS = {"comments", "steps", "taggings", "watches"}


def main():
    url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if not url or not url.startswith("mysql:") or not os.environ.get("RIZZY_TEST_MYSQL_CLIENT"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL and RIZZY_TEST_MYSQL_CLIENT")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-import-") as temp:
        directory = Path(temp)
        archive_path = Path(sys.argv[1]) if len(sys.argv) > 1 else directory / "account.zip"
        if len(sys.argv) == 1:
            subprocess.run([sys.executable, str(ROOT / "tests/account_transfer.py")],
                           cwd=ROOT, env={**os.environ, "RIZZY_TRANSFER_TEST_OUTPUT": str(archive_path)},
                           check=True, capture_output=True, text=True)
        archive = archive_path.read_bytes()
        with zipfile.ZipFile(archive_path) as zipped:
            records = {name: json.loads(zipped.read(name)) for name in zipped.namelist()
                       if name.startswith("data/") and name.endswith(".json") and name != "data/account.json"}
            account = json.loads(zipped.read("data/account.json"))
            owner = next(item for name, item in records.items()
                         if name.startswith("data/users/") and item["role"] == "owner")
            counts = Counter(name.split("/")[1] for name in records)
            blobs = [(item["id"], zipped.read(f"storage/{item['key']}"))
                     for name, item in records.items() if name.startswith("data/active_storage_blobs/")]
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        storage = directory / "storage"
        env = {**os.environ, "RIZZY_DATABASE_URL": url,
               "RIZZY_IMPORT_DIR": str(directory / "imports"),
               "RIZZY_STORAGE_DIR": str(storage),
               "RIZZY_EXPORT_DIR": str(directory / "exports"),
               "RIZZY_SEED_EMAIL": owner["email_address"],
               "RIZZY_SEED_NAME": owner["name"],
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
               "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
        seed = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                              capture_output=True, text=True, check=True).stdout
        seed_account = int(re.search(r"Account: (\d+)", seed).group(1))
        token = re.search(r"Bearer token: (\w+)", seed).group(1)
        foreign_user = mysql_scalar(
            url,
            "SELECT u.id FROM users u JOIN accounts a ON a.id=u.account_id "
            f"WHERE a.external_account_id={seed_account} AND u.role='owner'",
        )
        log_path = directory / "server.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([BINARY], cwd=ROOT,
                                       env={**env, "RIZZY_BIND": f"127.0.0.1:{port}"},
                                       stdout=log, stderr=subprocess.STDOUT,
                                       start_new_session=True)
        base = f"http://127.0.0.1:{port}"
        try:
            for _ in range(100):
                if process.poll() is not None:
                    raise RuntimeError("Rizzy exited: " + log_path.read_text()[-4000:])
                try:
                    with urllib.request.urlopen(base + "/up", timeout=.2):
                        break
                except OSError:
                    time.sleep(.05)
            else:
                raise RuntimeError("Rizzy did not start: " + log_path.read_text()[-4000:])
            rejected = upload(base, token, cross_account_reference(archive, foreign_user))
            rejected_result = await_result(base, token, rejected)
            assert rejected_result["status"] == "failed" and rejected_result["failure_reason"] == "invalid_export", rejected_result
            rejected_account = mysql_scalar(url, f"SELECT id FROM accounts WHERE external_account_id={int(rejected['account'])}")
            for table in ("boards", "cards", "storage_blobs", "rich_text_records"):
                assert mysql_scalar(url, f"SELECT COUNT(*) FROM `{table}` WHERE account_id='{rejected_account}'") == "0", table

            created = upload(base, token, archive)
            result = await_result(base, token, created)
            assert result["status"] == "completed", (result, log_path.read_text()[-4000:])
            account_id = mysql_scalar(url, f"SELECT id FROM accounts WHERE external_account_id={int(created['account'])}")
            assert mysql_scalar(url, f"SELECT name FROM accounts WHERE id='{account_id}'") == account["name"]
            for group, expected in counts.items():
                table = GROUP_TABLES.get(group, group)
                if group in CARD_SCOPED_GROUPS:
                    query = (f"SELECT COUNT(*) FROM `{table}` r JOIN cards c ON c.id=r.card_id "
                             f"WHERE c.account_id='{account_id}'")
                else:
                    query = f"SELECT COUNT(*) FROM `{table}` WHERE account_id='{account_id}'"
                actual = int(mysql_scalar(url, query))
                assert actual == expected, (group, actual, expected)
            for blob_id, source_bytes in blobs:
                key = mysql_scalar(url, f"SELECT storage_key FROM storage_blobs WHERE id='{blob_id}' AND account_id='{account_id}'")
                assert key and (storage / key).read_bytes() == source_bytes, blob_id
            assert mysql_scalar(url, f"SELECT COUNT(*) FROM cards WHERE account_id='{account_id}'") == str(account["cards_count"])
            duplicate = upload(base, token, archive)
            conflict = await_result(base, token, duplicate)
            assert conflict["status"] == "failed" and conflict["failure_reason"] == "conflict", conflict
            duplicate_account = mysql_scalar(url, f"SELECT id FROM accounts WHERE external_account_id={int(duplicate['account'])}")
            assert mysql_scalar(url, f"SELECT COUNT(*) FROM cards WHERE account_id='{duplicate_account}'") == "0"
            assert mysql_scalar(url, f"SELECT COUNT(*) FROM cards WHERE account_id='{account_id}'") == str(account["cards_count"])
            print(f"MySQL account import passed: invalid ZIP atomicity, duplicate conflict, {sum(counts.values())} records across {len(counts)} groups and {len(blobs)} files")
        except Exception:
            print(log_path.read_text()[-5000:])
            raise
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=10)


if __name__ == "__main__":
    main()
