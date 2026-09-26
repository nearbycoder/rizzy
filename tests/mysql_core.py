#!/usr/bin/env python3
"""Exercise Rizzy's initial MySQL-backed API workflow on a disposable database.

Set RIZZY_TEST_MYSQL_URL to a pre-created MySQL database URL. This check adds
one account and its records; use a database dedicated to tests.
"""

import json
import os
import re
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from urllib.parse import unquote, urlparse


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def call(base, token, path, method="GET", body=None, expected=200):
    request = urllib.request.Request(
        base + path,
        json.dumps(body).encode() if body is not None else None,
        method=method,
        headers={
            "Authorization": "Bearer " + token,
            "Accept": "application/json",
            "Content-Type": "application/json",
        },
    )
    try:
        response = urllib.request.urlopen(request, timeout=8)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        data = response.read()
        assert response.status == expected, (path, response.status, data[:400])
        return json.loads(data) if data else None


def check_metadata(url, account):
    client = os.environ.get("RIZZY_TEST_MYSQL_CLIENT") or shutil.which("mariadb") or shutil.which("mysql")
    if not client:
        print("MySQL client unavailable; direct metadata checks skipped")
        return False
    parsed = urlparse(url)
    statement = f"""SELECT
      (SELECT COUNT(*) FROM entropy_records WHERE account_id=a.id AND container_type='Account'),
      (SELECT COUNT(*) FROM user_attachables WHERE account_id=a.id),
      (SELECT COUNT(*) FROM user_settings_records WHERE account_id=a.id),
      (SELECT COUNT(*) FROM access_records WHERE account_id=a.id),
      (SELECT COUNT(*) FROM boards WHERE account_id=a.id AND updated_at>created_at)
      FROM accounts a WHERE external_account_id={int(account)}"""
    client_env = {**os.environ, "MYSQL_PWD": unquote(parsed.password or "")}
    result = subprocess.run(
        [client, "--no-defaults", "--host=" + (parsed.hostname or "localhost"),
         "--port=" + str(parsed.port or 3306), "--user=" + unquote(parsed.username or ""),
         "--database=" + parsed.path.lstrip("/"), "--skip-column-names", "--execute=" + statement],
        env=client_env, check=True, capture_output=True, text=True,
    )
    values = [int(value) for value in result.stdout.split()]
    assert values == [1, 2, 1, 1, 1], ("MySQL metadata", values)
    return True


def main():
    url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if not url or not url.startswith("mysql:"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL to a dedicated MySQL database URL")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-core-") as temp:
        email = "mysql-core-" + uuid.uuid4().hex + "@example.test"
        env = {
            **os.environ,
            "RIZZY_DATABASE_URL": url,
            "RIZZY_STORAGE_DIR": str(Path(temp) / "storage"),
            "RIZZY_SEED_EMAIL": email,
            "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
            "RIZZY_DISABLE_PUSH_DELIVERY": "1",
        }
        subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env, check=True, capture_output=True)
        seed = subprocess.run(
            [BINARY, "seed"], cwd=ROOT, env=env, check=True, capture_output=True, text=True
        ).stdout
        account = re.search(r"Account: (\d+)", seed).group(1)
        token = re.search(r"Bearer token: (\w+)", seed).group(1)
        with socket.socket() as port_socket:
            port_socket.bind(("127.0.0.1", 0))
            port = port_socket.getsockname()[1]
        env["RIZZY_BIND"] = f"127.0.0.1:{port}"
        log_path = Path(temp) / "server.log"
        with log_path.open("w") as log:
            process = subprocess.Popen(
                [BINARY], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT,
                start_new_session=True,
            )
        base = f"http://127.0.0.1:{port}"
        try:
            for _ in range(100):
                if process.poll() is not None:
                    raise AssertionError("Rizzy exited: " + log_path.read_text()[-1200:])
                try:
                    with urllib.request.urlopen(base + "/up", timeout=.2) as response:
                        if response.status == 200:
                            break
                except OSError:
                    time.sleep(.1)
            else:
                raise AssertionError("Rizzy did not start: " + log_path.read_text()[-1200:])
            scope = f"/{account}"
            board = call(base, token, scope + "/boards", "POST", {"board": {"name": "MySQL core"}}, 201)
            card = call(
                base, token, scope + f"/boards/{board['id']}/cards", "POST",
                {"card": {"title": "MySQL card", "description": "MySQL text"}}, 201,
            )
            number = card["number"]
            assert [item["number"] for item in call(base, token, scope + "/cards")] == [number]
            comment = call(
                base, token, scope + f"/cards/{number}/comments", "POST",
                {"comment": {"body": "MySQL comment"}}, 201,
            )
            call(base, token, scope + f"/cards/{number}/closure", "POST", expected=204)
            loaded = call(base, token, scope + f"/cards/{number}")
            assert loaded["closed"] is True and loaded["title"] == "MySQL card", loaded
            assert comment["body"]["plain_text"] == "MySQL comment", comment
            assert call(base, token, scope + "/cards") == []
            assert len(call(base, token, scope + f"/cards/{number}/comments")) >= 1
            assert len(call(base, token, scope + "/activities")) >= 3
            metadata_checked = check_metadata(url, account)
            detail = ", and metadata" if metadata_checked else ""
            print(f"MySQL core passed: seed, board, card, comment, close, reads{detail}")
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=10)


if __name__ == "__main__":
    main()
