#!/usr/bin/env python3
"""Two card viewers receive an edit written through another Rizzy process."""

import http.client
import json
import os
import re
import subprocess
import tempfile
import urllib.parse
import urllib.request
from pathlib import Path

from auth import BINARY, ROOT, call, cookie, request_code
from board_html_cached_etag import port, start


def api(url, token, path, method="GET", payload=None):
    request = urllib.request.Request(
        url + path,
        json.dumps(payload).encode() if payload is not None else None,
        {"Authorization": f"Bearer {token}", "Accept": "application/json",
         "Content-Type": "application/json"},
        method=method,
    )
    with urllib.request.urlopen(request, timeout=10) as response:
        return None if response.status == 204 else json.load(response)


def subscribe(url, path, session):
    address = urllib.parse.urlsplit(url)
    connection = http.client.HTTPConnection(address.hostname, address.port, timeout=8)
    connection.request("GET", path, headers={"Cookie": session, "Accept": "text/event-stream"})
    response = connection.getresponse()
    assert response.status == 200 and response.getheader("Content-Type").startswith("text/event-stream")
    return connection, response


def event(response):
    while True:
        line = response.readline().decode().strip()
        if line.startswith("data:"):
            return line[5:].strip()


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        email = "card-live@example.test"
        database = Path(temp) / "rizzy.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}",
               "RIZZY_STORAGE_DIR": str(Path(temp) / "storage"),
               "RIZZY_DEV_MAGIC_LINKS": "1", "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT,
                                env={**env, "RIZZY_SEED_EMAIL": email},
                                capture_output=True, text=True, check=True).stdout
        account = re.search(r"Account: (\d+)", seeded).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded).group(1)
        servers = []
        streams = []
        try:
            for index in range(2):
                process, url = start(env, f"127.0.0.1:{port()}", Path(temp) / f"server-{index}.log")
                servers.append((process, url))
            first, second = (item[1] for item in servers)
            first_port = urllib.parse.urlsplit(first).port
            pending, code = request_code(first_port, email)
            _, headers = call(first_port, "/session/magic_link", "POST", {"code": code}, [pending])
            session = cookie(headers, "session_token")
            board = api(second, token, f"/{account}/boards", "POST", {"board": {"name": "Live card"}})
            card = api(second, token, f"/{account}/boards/{board['id']}/cards", "POST",
                       {"card": {"title": "Before", "description": "Old description"}})
            path = f"/{account}/cards/{card['number']}"
            other = api(second, token, f"/{account}/boards/{board['id']}/cards", "POST",
                        {"card": {"title": "Other before"}})
            other_path = f"/{account}/cards/{other['number']}"
            for _ in range(2):
                streams.append(subscribe(first, path + "/live", session))
            streams.append(subscribe(first, other_path + "/live", session))
            initial = [event(response) for _, response in streams]
            assert initial[0] == initial[1], initial
            api(second, token, path, "PATCH",
                {"card": {"title": "After", "description": "New description"}})
            api(second, token, other_path, "PATCH", {"card": {"title": "Other after"}})
            def revision(path):
                request = urllib.request.Request(first + path + "/revision",
                                                 headers={"Cookie": session, "Accept": "application/json"})
                with urllib.request.urlopen(request, timeout=5) as response:
                    return json.load(response)["revision"]
            current = [revision(path), revision(path), revision(other_path)]
            assert all(before != after for before, after in zip(initial, current))
            for (_, response), expected in zip(streams, current):
                for _ in range(3):
                    if event(response) == expected:
                        break
                else:
                    raise AssertionError("Viewer missed the external card revision")
            step = api(second, token, path + "/steps", "POST",
                       {"step": {"content": "External step"}})
            for method, payload in (("PATCH", {"step": {"completed": True}}),
                                    ("DELETE", None)):
                expected = revision(path)
                for _, response in streams[:2]:
                    for _ in range(3):
                        if event(response) == expected:
                            break
                    else:
                        raise AssertionError("Viewer missed an external step revision")
                api(second, token, path + "/steps/" + step["id"], method, payload)
            expected = revision(path)
            for _, response in streams[:2]:
                for _ in range(3):
                    if event(response) == expected:
                        break
                else:
                    raise AssertionError("Viewer missed the deleted step revision")
            print("Live card stream passed: three viewers, two cards, external writer and step changes")
        finally:
            for connection, _ in streams:
                connection.close()
            for process, _ in servers:
                process.terminate()
                process.wait(timeout=5)


if __name__ == "__main__":
    main()
