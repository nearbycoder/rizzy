#!/usr/bin/env python3
"""Verify MySQL account cancellation, retention, scoped deletion, and file cleanup.

Set RIZZY_TEST_MYSQL_URL and RIZZY_TEST_MYSQL_CLIENT to a dedicated database.
Each run seeds two fresh accounts and ages only the first cancellation.
"""

import hashlib
import os
import re
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

from account_cancellation import request
from mysql_core import call, mysql_scalar


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def stamp(days=0):
    return (datetime.now(timezone.utc) - timedelta(days=days)).isoformat(
        timespec="milliseconds").replace("+00:00", "Z")


def seed(env, email):
    output = subprocess.run([BINARY, "seed"], cwd=ROOT,
                            env={**env, "RIZZY_SEED_EMAIL": email},
                            check=True, capture_output=True, text=True).stdout
    return re.search(r"Account: (\d+)", output).group(1), re.search(
        r"Bearer token: (\w+)", output).group(1)


def start(env, directory):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    log_path = directory / f"server-{port}.log"
    with log_path.open("w") as log:
        process = subprocess.Popen([BINARY], cwd=ROOT,
                                   env={**env, "RIZZY_BIND": f"127.0.0.1:{port}"},
                                   stdout=log, stderr=subprocess.STDOUT,
                                   start_new_session=True)
    base = f"http://127.0.0.1:{port}"
    for _ in range(100):
        if process.poll() is not None:
            raise RuntimeError("Rizzy exited: " + log_path.read_text()[-4000:])
        try:
            with urllib.request.urlopen(base + "/up", timeout=.2):
                return port, process, log_path
        except OSError:
            time.sleep(.05)
    raise RuntimeError("Rizzy did not start: " + log_path.read_text()[-4000:])


def stop(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
    process.wait(timeout=10)


def main():
    url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if not url or not url.startswith("mysql:") or not os.environ.get("RIZZY_TEST_MYSQL_CLIENT"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL and RIZZY_TEST_MYSQL_CLIENT")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-cancellation-") as temp:
        directory = Path(temp)
        storage, exports = directory / "storage", directory / "exports"
        storage.mkdir()
        exports.mkdir()
        env = {**os.environ, "RIZZY_DATABASE_URL": url,
               "RIZZY_STORAGE_DIR": str(storage), "RIZZY_EXPORT_DIR": str(exports),
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
               "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
        first, first_token = seed(env, "cancel-" + uuid.uuid4().hex + "@example.test")
        second, second_token = seed(env, "survivor-" + uuid.uuid4().hex + "@example.test")
        account_id, owner_id, identity_id = mysql_scalar(
            url, "SELECT a.id,u.id,u.identity_id FROM accounts a JOIN users u ON u.account_id=a.id "
                 f"WHERE a.external_account_id={int(first)} AND u.role='owner'").split("\t")
        other_account = mysql_scalar(url, f"SELECT id FROM accounts WHERE external_account_id={int(second)}")
        cookie = "cancel-" + uuid.uuid4().hex
        mysql_scalar(url, "INSERT INTO sessions(id,identity_id,token_hash,created_at) VALUES(" +
                     f"'{uuid.uuid4().hex}','{identity_id}','{hashlib.sha256(cookie.encode()).hexdigest()}','{stamp()}')")
        port, process, log = start(env, directory)
        try:
            first_board = call(f"http://127.0.0.1:{port}", first_token, f"/{first}/boards", "POST",
                               {"board": {"name": "Cancelled board"}}, 201)
            first_card = call(f"http://127.0.0.1:{port}", first_token,
                              f"/{first}/boards/{first_board['id']}/cards", "POST",
                              {"card": {"title": "Cancelled card"}}, 201)
            call(f"http://127.0.0.1:{port}", first_token,
                 f"/{first}/cards/{first_card['number']}/comments", "POST",
                 {"comment": {"body": "A comment"}}, 201)
            second_board = call(f"http://127.0.0.1:{port}", second_token, f"/{second}/boards", "POST",
                                {"board": {"name": "Surviving board"}}, 201)
            second_card = call(f"http://127.0.0.1:{port}", second_token,
                               f"/{second}/boards/{second_board['id']}/cards", "POST",
                               {"card": {"title": "Surviving card"}}, 201)
            assert request(port, f"/{first}/account/cancellation", "POST", cookie=cookie,
                           html=True)[0] == 302
            assert mysql_scalar(url, f"SELECT COUNT(*) FROM account_cancellations WHERE account_id='{account_id}'") == "0"
        finally:
            stop(process)

        env["RIZZY_MULTI_TENANT"] = "1"
        blob_id, export_id = uuid.uuid4().hex, uuid.uuid4().hex
        key = "cancel-" + uuid.uuid4().hex
        mysql_scalar(url, "INSERT INTO storage_blobs(id,account_id,storage_key,filename,content_type,byte_size,checksum,signed_id,attachable_sgid,upload_token_hash,upload_expires_at,created_at) VALUES(" +
                     f"'{blob_id}','{account_id}','{key}','file.txt','text/plain',4,'checksum','signed-{blob_id}','sgid-{blob_id}','upload-{blob_id}','{stamp(-1)}','{stamp()}')")
        mysql_scalar(url, "INSERT INTO exports(id,account_id,user_id,kind,status,created_at,updated_at) VALUES(" +
                     f"'{export_id}','{account_id}','{owner_id}','account','completed','{stamp()}','{stamp()}')")
        (storage / key).write_bytes(b"file")
        (exports / f"{export_id}.zip").write_bytes(b"archive")
        port, process, log = start(env, directory)
        try:
            status, headers, _ = request(port, f"/{first}/account/cancellation", "POST",
                                         cookie=cookie, html=True)
            assert status == 302 and headers["Location"] == "/session/menu"
            assert request(port, f"/{first}/account/settings.json", token=first_token)[0] == 403
            assert request(port, f"/{second}/account/settings.json", token=second_token)[0] == 200
            mysql_scalar(url, f"UPDATE account_cancellations SET created_at='{stamp(29)}' WHERE account_id='{account_id}'")
        finally:
            stop(process)
        port, process, log = start(env, directory)
        try:
            assert mysql_scalar(url, f"SELECT COUNT(*) FROM accounts WHERE id='{account_id}'") == "1"
            mysql_scalar(url, f"UPDATE account_cancellations SET created_at='{stamp(31)}' WHERE account_id='{account_id}'")
        finally:
            stop(process)
        port, process, log = start(env, directory)
        try:
            for _ in range(100):
                if mysql_scalar(url, f"SELECT COUNT(*) FROM accounts WHERE id='{account_id}'") == "0":
                    break
                time.sleep(.05)
            else:
                raise AssertionError("Cancelled account was not incinerated: " + log.read_text()[-4000:])
            tables = mysql_scalar(url, "SELECT DISTINCT table_name FROM information_schema.columns WHERE table_schema=DATABASE() AND column_name='account_id'").splitlines()
            for table in tables:
                count = mysql_scalar(url, f"SELECT COUNT(*) FROM `{table}` WHERE account_id='{account_id}'")
                assert count == "0", (table, count)
            assert mysql_scalar(url, f"SELECT COUNT(*) FROM accounts WHERE id='{other_account}'") == "1"
            assert mysql_scalar(url, f"SELECT COUNT(*) FROM cards WHERE id='{second_card['id']}'") == "1"
            assert mysql_scalar(url, f"SELECT COUNT(*) FROM cards WHERE id='{first_card['id']}'") == "0"
            assert mysql_scalar(url, f"SELECT COUNT(*) FROM comments WHERE card_id='{first_card['id']}'") == "0"
            assert not (storage / key).exists()
            assert not (exports / f"{export_id}.zip").exists()
            assert request(port, f"/{second}/account/settings.json", token=second_token)[0] == 200
            print("MySQL cancellation passed: guard, access, day-29 retention, day-31 scoped deletion, file cleanup")
        except Exception:
            print(log.read_text()[-5000:])
            raise
        finally:
            stop(process)


if __name__ == "__main__":
    main()
