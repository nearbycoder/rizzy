#!/usr/bin/env python3
"""A conditional board read must see writes from another Rizzy process."""

import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"
sys.path.insert(0, str(ROOT / "bench"))
from events_html import browser_cookie


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def request(url, token, path, method="GET", data=None, etag=None, cookie=None):
    headers = {"Authorization": f"Bearer {token}",
               "Accept": "text/html" if data is None else "application/json"}
    if data is not None:
        headers["Content-Type"] = "application/json"
    if etag:
        headers["If-None-Match"] = etag
    if cookie:
        headers["Cookie"] = cookie
    payload = json.dumps(data).encode() if data is not None else None
    req = urllib.request.Request(url + path, payload, headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=10) as response:
            return response.status, response.headers, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.headers, error.read()


def start(env, address, log, binary=BINARY):
    with open(log, "w") as output:
        process = subprocess.Popen([binary], cwd=ROOT,
                                   env={**env, "RIZZY_BIND": address},
                                   stdout=output, stderr=subprocess.STDOUT)
    url = f"http://{address}"
    for _ in range(100):
        if process.poll() is not None:
            raise RuntimeError(Path(log).read_text())
        try:
            with urllib.request.urlopen(url + "/up", timeout=.2):
                return process, url
        except OSError:
            time.sleep(.05)
    process.terminate()
    process.wait(timeout=5)
    raise RuntimeError(Path(log).read_text())


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "rizzy.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}",
               "RIZZY_STORAGE_DIR": str(Path(temp) / "storage"),
               "RIZZY_DEV_MAGIC_LINKS": "1",
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True).stdout
        account = re.search(r"Account: (\d+)", seeded).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded).group(1)
        processes = []
        try:
            for index in range(2):
                process, url = start(env, f"127.0.0.1:{port()}",
                                     Path(temp) / f"server-{index}.log")
                processes.append((process, url))
            first, second = processes[0][1], processes[1][1]
            cookie = browser_cookie({"name": "rizzy", "url": first, "account": account,
                                     "email": "owner@example.com", "db": str(database)})
            status, _, raw = request(second, token, f"/{account}/boards", "POST",
                                     {"board": {"name": "ETag board"}})
            assert status == 201, (status, raw)
            board = json.loads(raw)["id"]
            path = f"/{account}/boards/{board}"
            status, headers, raw = request(first, token, path, cookie=cookie)
            assert status == 200 and raw, status
            original = headers["ETag"]
            status, headers, raw = request(first, token, path, etag=original, cookie=cookie)
            assert status == 304 and headers["ETag"] == original and not raw
            expanded_path = path + "?expand_all=true"
            status, headers, raw = request(first, token, expanded_path, etag=original, cookie=cookie)
            assert status == 200 and b'board-filter is-expanded' in raw and headers["ETag"] != original
            expanded_etag = headers["ETag"]
            status, _, raw = request(first, token, expanded_path, etag=expanded_etag, cookie=cookie)
            assert status == 304 and not raw
            status, _, raw = request(first, token, path, etag=expanded_etag, cookie=cookie)
            assert status == 200 and b'board-filter is-expanded' not in raw
            status, _, _ = request(second, token, path, "PATCH",
                                   {"board": {"name": "Renamed ETag board"}})
            assert status == 200, status
            status, headers, raw = request(first, token, path, etag=original, cookie=cookie)
            assert status == 200 and raw and headers["ETag"] != original
            renamed = headers["ETag"]
            status, _, raw = request(first, token, path, etag=renamed, cookie=cookie)
            assert status == 304 and not raw
            status, _, raw = request(second, token, f"{path}/cards", "POST",
                                     {"card": {"title": "New card"}})
            assert status == 201, (status, raw)
            status, headers, raw = request(first, token, path, etag=renamed, cookie=cookie)
            assert status == 200 and b"New card" in raw and headers["ETag"] != renamed
            print("Board HTML conditional cache passed across two processes")
        finally:
            for process, _ in processes:
                process.terminate()
            for process, _ in processes:
                process.wait(timeout=5)


if __name__ == "__main__":
    main()
