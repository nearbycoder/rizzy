#!/usr/bin/env python3
"""Exercise queued webhook delivery with two Rizzy workers on one MySQL DB.

Set RIZZY_TEST_MYSQL_URL and RIZZY_TEST_MYSQL_CLIENT to a disposable database
and a MySQL client. The destination is a blocked loopback address.
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

from mysql_core import call, mysql_scalar


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def start(env, temp, name):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    log_path = Path(temp) / f"{name}.log"
    with log_path.open("w") as log:
        process = subprocess.Popen(
            [BINARY], cwd=ROOT, env={**env, "RIZZY_BIND": f"127.0.0.1:{port}"},
            stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
        )
    for _ in range(100):
        if process.poll() is not None:
            raise AssertionError(f"{name} exited: {log_path.read_text()[-1200:]}")
        try:
            with urllib.request.urlopen(base + "/up", timeout=.2) as response:
                if response.status == 200:
                    return base, process, log_path
        except OSError:
            time.sleep(.1)
    stop(process)
    raise AssertionError(f"{name} did not start: {log_path.read_text()[-1200:]}")


def stop(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
    process.wait(timeout=10)


def main():
    url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if not url or not url.startswith("mysql:") or not os.environ.get("RIZZY_TEST_MYSQL_CLIENT"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL and RIZZY_TEST_MYSQL_CLIENT")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-webhooks-") as temp:
        env = {
            **os.environ,
            "RIZZY_DATABASE_URL": url,
            "RIZZY_STORAGE_DIR": str(Path(temp) / "storage"),
            "RIZZY_SEED_EMAIL": "mysql-webhooks-" + uuid.uuid4().hex + "@example.test",
            "RIZZY_DISABLE_PUSH_DELIVERY": "1",
        }
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True).stdout
        account = re.search(r"Account: (\d+)", seeded).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded).group(1)
        processes = []
        logs = []
        try:
            base, process, log = start({**env, "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}, temp, "queue")
            processes.append(process)
            logs.append(log)
            scope = f"/{account}"
            board = call(base, token, scope + "/boards", "POST",
                         {"board": {"name": "MySQL webhook queue"}}, 201)
            path = scope + f"/boards/{board['id']}/webhooks"
            webhook = call(base, token, path, "POST",
                           {"webhook": {"name": "Blocked target", "url": "http://127.0.0.1:1/hook",
                                        "subscribed_actions": ["card_closed", "card_closed", "unknown"]}}, 201)
            assert webhook["subscribed_actions"] == ["card_closed"]
            updated = call(base, token, path + "/" + webhook["id"], "PATCH",
                           {"webhook": {"name": "Updated target", "url": "https://example.invalid/hook",
                                        "subscribed_actions": ["card_published"]}})
            assert updated["name"] == "Updated target"
            assert updated["subscribed_actions"] == ["card_published"]
            assert updated["payload_url"] == webhook["payload_url"]
            delivery_path = path + "/" + webhook["id"] + "/deliveries"
            call(base, token, scope + f"/boards/{board['id']}/cards", "POST",
                 {"card": {"title": "Webhook event"}}, 201)
            queued = call(base, token, delivery_path)
            assert len(queued) == 1 and queued[0]["state"] == "pending", queued
            mysql_scalar(url, f"UPDATE webhook_delinquency_trackers SET consecutive_failures_count=9,first_failure_at='2025-01-01T00:00:00.000Z' WHERE webhook_id='{webhook['id']}'")
            stop(process)
            processes.clear()

            for index in range(2):
                worker_base, worker, log = start(env, temp, f"worker-{index}")
                processes.append(worker)
                logs.append(log)
            for _ in range(100):
                delivered = call(worker_base, token, delivery_path)
                if delivered[0]["state"] in ("completed", "errored"):
                    break
                time.sleep(.1)
            assert delivered[0]["state"] == "completed", delivered
            assert delivered[0]["response"] == {"code": None, "error": "private_uri"}, delivered
            assert delivered[0]["request"]["headers"]["User-Agent"] == "fizzy/1.0.0 Webhook"
            assert "X-Webhook-Signature" not in delivered[0]["request"]["headers"]
            assert mysql_scalar(url, f"SELECT COUNT(*) FROM webhook_deliveries WHERE webhook_id='{webhook['id']}'") == "1"
            assert mysql_scalar(url, f"SELECT consecutive_failures_count FROM webhook_delinquency_trackers WHERE webhook_id='{webhook['id']}'") == "10"
            assert call(worker_base, token, path + "/" + webhook["id"])["active"] is False
            assert call(worker_base, token, path + "/" + webhook["id"] + "/activation",
                        "POST", expected=201)["active"] is True
            call(worker_base, token, path + "/" + webhook["id"], "DELETE", expected=204)
            assert mysql_scalar(url, f"SELECT COUNT(*) FROM webhook_deliveries WHERE webhook_id='{webhook['id']}'") == "0"
            print("MySQL webhooks passed: CRUD, queue, two-worker claim, blocked target, one delivery, delinquency")
        except Exception:
            for log in logs:
                print(f"{log}: {log.read_text()[-1800:]}")
            raise
        finally:
            for process in processes:
                stop(process)


if __name__ == "__main__":
    main()
