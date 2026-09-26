#!/usr/bin/env python3
"""Check concurrent card numbering through two MySQL-backed Rizzy processes.

Set RIZZY_TEST_MYSQL_URL to a MySQL database dedicated to tests. Each run adds
one account and 80 cards. This checks correctness, not a throughput target.
"""

import concurrent.futures
import json
import os
import re
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def call(base, account, token, path, method="GET", body=None):
    request = urllib.request.Request(
        base + f"/{account}" + path,
        json.dumps(body).encode() if body is not None else None,
        method=method,
        headers={
            "Authorization": "Bearer " + token,
            "Accept": "application/json",
            "Content-Type": "application/json",
        },
    )
    try:
        response = urllib.request.urlopen(request, timeout=20)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        data = response.read()
        return response.status, json.loads(data) if data else None, response.headers


def main():
    url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if not url or not url.startswith("mysql:"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL to a dedicated MySQL database URL")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-cluster-") as temp:
        env = {
            **os.environ,
            "RIZZY_DATABASE_URL": url,
            "RIZZY_SEED_EMAIL": "mysql-cluster-" + uuid.uuid4().hex + "@example.test",
            "RIZZY_STORAGE_DIR": str(Path(temp) / "storage"),
            "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
            "RIZZY_DISABLE_PUSH_DELIVERY": "1",
        }
        initializers = [
            subprocess.Popen(
                [BINARY, "init-db"], cwd=ROOT, env=env,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            )
            for _ in range(2)
        ]
        for process in initializers:
            _, error = process.communicate(timeout=30)
            assert process.returncode == 0, error[-1200:]
        seed = subprocess.run(
            [BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True
        ).stdout
        account = re.search(r"Account: (\d+)", seed).group(1)
        token = re.search(r"Bearer token: (\w+)", seed).group(1)
        processes = []
        bases = []
        try:
            for index in range(2):
                port = free_port()
                local_env = {**env, "RIZZY_BIND": f"127.0.0.1:{port}"}
                log = (Path(temp) / f"rizzy-{index}.log").open("w")
                process = subprocess.Popen(
                    [BINARY], cwd=ROOT, env=local_env, stdout=log,
                    stderr=subprocess.STDOUT, start_new_session=True,
                )
                processes.append((process, log))
                bases.append(f"http://127.0.0.1:{port}")
            for base, (process, log) in zip(bases, processes):
                for _ in range(100):
                    if process.poll() is not None:
                        raise AssertionError("Rizzy exited: " + Path(log.name).read_text()[-1200:])
                    try:
                        with urllib.request.urlopen(base + "/up", timeout=.2) as response:
                            if response.status == 200:
                                break
                    except OSError:
                        time.sleep(.1)
                else:
                    raise AssertionError("Rizzy did not start: " + Path(log.name).read_text()[-1200:])
            status, board, _ = call(
                bases[0], account, token, "/boards", "POST", {"board": {"name": "Cluster"}}
            )
            assert status == 201, (status, board)

            def create(index):
                return call(
                    bases[index % 2], account, token, f"/boards/{board['id']}/cards", "POST",
                    {"card": {"title": f"Concurrent {index}"}},
                )

            with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
                results = list(pool.map(create, range(80)))
            failures = [(index, status, value) for index, (status, value, _) in enumerate(results)
                        if status != 201]
            assert not failures, failures[:5]
            numbers = [value["number"] for _, value, _ in results]
            assert sorted(numbers) == list(range(1, 81)), numbers
            status, cards, headers = call(bases[1], account, token, "/cards")
            assert status == 200 and len(cards) == 15, (status, len(cards))
            assert int(headers["X-Total-Count"]) == 80, headers
            print("MySQL cluster passed: 80 unique cards across two processes")
        finally:
            for process, log in processes:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=10)
                log.close()


if __name__ == "__main__":
    main()
