#!/usr/bin/env python3
"""Check MySQL device-transfer links, expiry, replay, and concurrent single use.

Set RIZZY_TEST_MYSQL_URL and RIZZY_TEST_MYSQL_CLIENT to a dedicated database.
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

from mysql_core import mysql_scalar
from session_transfer_parity import opener, run, rust_expiry_and_race


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def main():
    url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if not url or not url.startswith("mysql:") or not os.environ.get("RIZZY_TEST_MYSQL_CLIENT"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL and RIZZY_TEST_MYSQL_CLIENT")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-transfer-") as temp:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        email = "transfer-" + uuid.uuid4().hex + "@example.test"
        env = {**os.environ, "RIZZY_DATABASE_URL": url, "RIZZY_SEED_EMAIL": email,
               "RIZZY_BIND": f"127.0.0.1:{port}", "RIZZY_DEV_MAGIC_LINKS": "1",
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
               "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
        seed = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                              capture_output=True, text=True, check=True).stdout
        account = re.search(r"Account: (\d+)", seed).group(1)
        token = re.search(r"Bearer token: (\w+)", seed).group(1)
        mysql_scalar(url, "DELETE FROM authentication_attempts")
        log_path = Path(temp) / "server.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([BINARY], cwd=ROOT, env=env,
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
            target = {"name": "rizzy", "url": base, "account": account,
                      "token": token, "email": email, "opener": opener()}
            run(target)
            rust_expiry_and_race(target)
            print("MySQL session transfer passed: generate, revoke, show, redeem, replay, expiry, concurrent single use")
        except Exception:
            print(log_path.read_text()[-5000:])
            raise
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=10)


if __name__ == "__main__":
    main()
