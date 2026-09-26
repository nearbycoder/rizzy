#!/usr/bin/env python3
"""Run the draft browser flow on MySQL and hand a draft between two servers.

Set RIZZY_TEST_MYSQL_URL to a dedicated MySQL database URL.
"""

import concurrent.futures
import http.cookiejar
import json
import os
import re
import signal
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request
import uuid
from pathlib import Path

from draft_workflow_parity import NoRedirect, request, run
from mysql_cluster import call, free_port


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def publish_many(target, bases, account, token):
    drafts = []
    try:
        for index in range(32):
            target["url"] = bases[index % 2]
            status, _, raw = request(target, f"/{account}/boards", "POST",
                                     {"board": {"name": f"Parallel draft {index}"}}, bearer=True)
            assert status == 201, (index, status, raw[:160])
            board = json.loads(raw)
            status, headers, _ = request(
                target, f"/{account}/boards/{board['id']}/cards", "POST", b"",
                accept="text/html")
            assert status == 302, (index, status)
            path = urllib.parse.urlparse(headers["Location"]).path
            number = re.fullmatch(rf"/{account}/cards/(\d+)/draft", path).group(1)
            drafts.append((board["id"], number))

        def publish(index):
            _, number = drafts[index]
            return call(bases[index % 2], account, token,
                        f"/cards/{number}/publish", "POST", {})

        with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
            results = list(pool.map(publish, range(len(drafts))))
        failures = [(index, status, value) for index, (status, value, _) in enumerate(results)
                    if status != 201]
        assert not failures, failures[:5]
        for index, (_, number) in enumerate(drafts):
            status, card, _ = call(bases[(index + 1) % 2], account, token,
                                   f"/cards/{number}")
            assert status == 200 and card["status"] == "published", (index, status, card)
    finally:
        for board_id, _ in drafts:
            call(bases[0], account, token, f"/boards/{board_id}", "DELETE")


def main():
    url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if not url or not url.startswith("mysql:"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL to a dedicated MySQL database URL")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-draft-") as temp:
        directory = Path(temp)
        email = "draft-" + uuid.uuid4().hex + "@example.test"
        env = {**os.environ, "RIZZY_DATABASE_URL": url,
               "RIZZY_SEED_EMAIL": email,
               "RIZZY_STORAGE_DIR": str(directory / "storage"),
               "RIZZY_DEV_MAGIC_LINKS": "1",
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
               "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True).stdout
        account = re.search(r"Account: (\d+)", seeded).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded).group(1)
        processes, bases, logs = [], [], []
        try:
            for index in range(2):
                port = free_port()
                local_env = {**env, "RIZZY_BIND": f"127.0.0.1:{port}"}
                log_path = directory / f"rizzy-{index}.log"
                log = log_path.open("w")
                process = subprocess.Popen([BINARY], cwd=ROOT, env=local_env,
                                           stdout=log, stderr=subprocess.STDOUT,
                                           start_new_session=True)
                processes.append((process, log))
                bases.append(f"http://127.0.0.1:{port}")
                logs.append(log_path)
            for index, (base, (process, _)) in enumerate(zip(bases, processes)):
                for _ in range(100):
                    if process.poll() is not None:
                        raise AssertionError("Rizzy exited: " + logs[index].read_text()[-2000:])
                    try:
                        with urllib.request.urlopen(base + "/up", timeout=.2):
                            break
                    except OSError:
                        time.sleep(.05)
                else:
                    raise AssertionError("Rizzy did not start")

            target = {"name": "rizzy", "url": bases[0], "account": account,
                      "token": token, "email": email,
                      "opener": urllib.request.build_opener(
                          urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                          NoRedirect())}
            run(target)

            status, _, raw = request(target, f"/{account}/boards", "POST",
                                     {"board": {"name": "Cross-process draft"}}, bearer=True)
            assert status == 201, (status, raw[:200])
            board = json.loads(raw)
            try:
                status, headers, _ = request(
                    target, f"/{account}/boards/{board['id']}/cards", "POST", b"",
                    accept="text/html")
                assert status == 302, status
                path = urllib.parse.urlparse(headers["Location"]).path
                number = re.fullmatch(rf"/{account}/cards/(\d+)/draft", path).group(1)
                target["url"] = bases[1]
                status, _, html = request(target, path, accept="text/html")
                assert status == 200 and b"card-perma--draft" in html
                status, _, raw = request(
                    target, f"/{account}/cards/{number}.json", "PATCH",
                    {"card": {"title": "Across processes", "description": "MySQL draft"}},
                    bearer=True)
                assert status == 200 and json.loads(raw)["status"] == "drafted"
                target["url"] = bases[0]
                status, _, raw = request(target, f"/{account}/cards/{number}/publish.json",
                                         "POST", b"", bearer=True)
                assert status == 201 and raw == b"", (status, raw[:200])
                target["url"] = bases[1]
                status, _, raw = request(target, f"/{account}/cards/{number}.json", bearer=True)
                card = json.loads(raw)
                assert status == 200 and card["status"] == "published"
                assert card["title"] == "Across processes" and card["description"] == "MySQL draft"
                status, _, raw = request(target, f"/{account}/activities.json?board_ids%5B%5D={board['id']}", bearer=True)
                activities = json.loads(raw)
                assert status == 200 and [item["action"] for item in activities] == ["card_published"]
            finally:
                request(target, f"/{account}/boards/{board['id']}", "DELETE", bearer=True)
            publish_many(target, bases, account, token)
            print("MySQL draft flow passed across two servers, including 32 concurrent publishes")
        except Exception:
            for log_path in logs:
                if log_path.exists():
                    print(log_path.read_text()[-2500:])
            raise
        finally:
            for process, log in processes:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=10)
                log.close()


if __name__ == "__main__":
    main()
