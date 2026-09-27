#!/usr/bin/env python3
"""Run the existing passkey parity flow against a disposable MySQL Rizzy account.

Set RIZZY_TEST_MYSQL_URL and RIZZY_TEST_MYSQL_CLIENT to a dedicated database.
Each run seeds a fresh account and keeps its records for inspection.
"""

import os
import re
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request
import uuid
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def main():
    url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if not url or not url.startswith("mysql:") or not os.environ.get("RIZZY_TEST_MYSQL_CLIENT"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL and RIZZY_TEST_MYSQL_CLIENT")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-passkeys-") as temp:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        email = "passkey-" + uuid.uuid4().hex + "@example.test"
        base = f"http://localhost:{port}"
        env = {**os.environ,
               "RIZZY_DATABASE_URL": url,
               "RIZZY_SEED_EMAIL": email,
               "RIZZY_BIND": f"127.0.0.1:{port}",
               "RIZZY_PASSKEY_ORIGIN": base,
               "RIZZY_DEV_MAGIC_LINKS": "1",
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
               "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                check=True, capture_output=True, text=True)
        account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
        from mysql_core import mysql_scalar
        mysql_scalar(url, "DELETE FROM authentication_attempts")
        log_path = Path(temp) / "server.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([BINARY], cwd=ROOT, env=env,
                                       stdout=log, stderr=subprocess.STDOUT,
                                       start_new_session=True)
        try:
            for _ in range(100):
                if process.poll() is not None:
                    raise RuntimeError("Rizzy exited: " + log_path.read_text()[-3000:])
                try:
                    with urllib.request.urlopen(base + "/up", timeout=.2):
                        break
                except OSError:
                    time.sleep(.05)
            else:
                raise RuntimeError("Rizzy did not start: " + log_path.read_text()[-3000:])
            result = subprocess.run(
                ["python3", "tests/passkey_parity.py"], cwd=ROOT,
                env={**env, "RIZZY_URL": base, "RIZZY_EMAIL": email,
                     "RIZZY_ACCOUNT": account, "PASSKEY_RACE": "1"},
                capture_output=True, text=True,
            )
            if result.returncode:
                raise AssertionError(result.stdout + result.stderr)
            print(result.stdout.strip())
        except Exception:
            print(log_path.read_text()[-5000:])
            raise
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=10)


if __name__ == "__main__":
    main()
