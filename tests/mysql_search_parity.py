#!/usr/bin/env python3
"""Compare Fizzy's MySQL search adapter with Rizzy on one disposable workflow.

Set FIZZY_URL, FIZZY_ACCOUNT, FIZZY_TOKEN, FIZZY_EMAIL, RIZZY_TEST_MYSQL_URL,
RIZZY_TEST_MYSQL_CLIENT, and MYSQL_HOST/PORT/USER/PASSWORD. The Fizzy account
needs a write token. The Rizzy database must be dedicated to tests.
"""

import json
import http.cookiejar
import os
import re
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def api(target, path, method="GET", payload=None, expected=200):
    request = urllib.request.Request(
        target["url"] + f"/{target['account']}" + path,
        json.dumps(payload).encode() if payload is not None else None,
        method=method,
        headers={
            "Authorization": "Bearer " + target["token"],
            "Accept": "application/json",
            "Content-Type": "application/json",
        },
    )
    try:
        response = urllib.request.urlopen(request, timeout=12)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        body = response.read()
        assert response.status == expected, (target["name"], method, path, response.status, body[:400])
        return json.loads(body) if body else None


def search(target, term):
    cards = api(target, "/search?" + urllib.parse.urlencode({"q": term}))
    return sorted(card["title"] for card in cards)


def filtered_cards(target, term):
    cards = api(target, "/cards?" + urllib.parse.urlencode({"terms[]": term}))
    return sorted(card["title"] for card in cards)


def browser_login(target):
    target["opener"] = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())
    )

    def post(path, payload):
        request = urllib.request.Request(
            target["url"] + path, json.dumps(payload).encode(), method="POST",
            headers={"Accept": "application/json", "Content-Type": "application/json"},
        )
        return target["opener"].open(request, timeout=12)

    with post("/session", {"email_address": target["email"]}) as response:
        assert response.status == 201
        code = response.headers.get("X-Magic-Link-Code")
    if target["name"] == "fizzy":
        client = os.environ["RIZZY_TEST_MYSQL_CLIENT"]
        mysql_env = {**os.environ, "MYSQL_PWD": os.environ["MYSQL_PASSWORD"]}
        statement = (
            "SELECT m.code FROM magic_links m JOIN identities i ON i.id=m.identity_id "
            "WHERE i.email_address='" + target["email"].replace("'", "''") + "' "
            "ORDER BY m.created_at DESC LIMIT 1"
        )
        code = subprocess.run(
            [client, "--no-defaults", "--host=" + os.environ["MYSQL_HOST"],
             "--port=" + os.environ["MYSQL_PORT"], "--user=" + os.environ["MYSQL_USER"],
             "--database=fizzy_development", "--skip-column-names", "--execute=" + statement],
            env=mysql_env, check=True, capture_output=True, text=True,
        ).stdout.strip()
    assert code, target["name"]
    with post("/session/magic_link", {"code": code}) as response:
        assert response.status == 200


def browser_hit_count(target, term):
    request = urllib.request.Request(
        target["url"] + f"/{target['account']}/search?" + urllib.parse.urlencode({"q": term}),
        headers={"Accept": "text/html"},
    )
    with target["opener"].open(request, timeout=12) as response:
        assert response.status == 200
        html = response.read().decode()
        return len(re.findall(r'<h3[^>]*class="[^"]*search__title[^"]*"', html))


def run(target, tag):
    board = api(target, "/boards", "POST", {"board": {"name": "MySQL search " + tag}}, 201)
    board_id = board["id"]
    try:
        def card(title, description=""):
            return api(target, f"/boards/{board_id}/cards", "POST", {"card": {"title": title, "description": description}}, 201)

        first = card("Layout is broken", "Look at this mess")
        second = card("Some card")
        third = card("Just haggis", "More haggis")
        stemmed = card("Running authentication")
        comment = api(target, f"/cards/{second['number']}/comments", "POST", {"comment": {"body": "overflowing text issue"}}, 201)
        api(target, f"/cards/{third['number']}/comments", "POST", {"comment": {"body": "I love haggis"}}, 201)
        walk = api(target, f"/cards/{stemmed['number']}/comments", "POST", {"comment": {"body": "Walking swiftly"}}, 201)
        initial = {term: search(target, term) for term in (
            "broken", "overflowing", "haggis", "run", "authenticate", "walk",
            '"Layout is broken"', "missing", "café", "東京",
        )}
        filtered = {term: filtered_cards(target, term) for term in ("run", "haggis", "missing")}
        browser = {term: browser_hit_count(target, term) for term in ("run", "haggis", "overflowing", "missing")}
        api(target, f"/cards/{first['number']}", "PUT", {"card": {"title": "Repaired layout"}})
        api(target, f"/cards/{second['number']}/comments/{comment['id']}", "PUT", {"comment": {"body": "Sprinting text issue"}})
        edited = {term: search(target, term) for term in ("repaired", "sprint", "overflowing")}
        api(target, f"/cards/{stemmed['number']}/comments/{walk['id']}", "DELETE", expected=204)
        api(target, f"/cards/{third['number']}", "DELETE", expected=204)
        removed = {term: search(target, term) for term in ("walk", "haggis")}
        return {"initial": initial, "filtered": filtered, "browser": browser,
                "edited": edited, "removed": removed}
    finally:
        api(target, f"/boards/{board_id}", "DELETE", expected=204)


def main():
    required = ("FIZZY_URL", "FIZZY_ACCOUNT", "FIZZY_TOKEN", "FIZZY_EMAIL",
                "RIZZY_TEST_MYSQL_URL", "RIZZY_TEST_MYSQL_CLIENT",
                "MYSQL_HOST", "MYSQL_PORT", "MYSQL_USER", "MYSQL_PASSWORD")
    missing = [name for name in required if not os.environ.get(name)]
    if missing:
        raise SystemExit("Set " + ", ".join(missing))
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    source = {"name": "fizzy", "url": os.environ["FIZZY_URL"].rstrip("/"),
              "account": os.environ["FIZZY_ACCOUNT"], "token": os.environ["FIZZY_TOKEN"],
              "email": os.environ["FIZZY_EMAIL"]}
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-search-") as temp:
        env = {**os.environ, "RIZZY_DATABASE_URL": os.environ["RIZZY_TEST_MYSQL_URL"],
               "RIZZY_STORAGE_DIR": str(Path(temp) / "storage"),
               "RIZZY_SEED_EMAIL": "mysql-search-" + uuid.uuid4().hex + "@example.test",
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1", "RIZZY_DISABLE_PUSH_DELIVERY": "1",
               "RIZZY_DEV_MAGIC_LINKS": "1"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True).stdout
        account = re.search(r"Account: (\d+)", seeded).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded).group(1)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        rust = {"name": "rizzy", "url": f"http://127.0.0.1:{port}",
                "account": account, "token": token, "email": env["RIZZY_SEED_EMAIL"]}
        env["RIZZY_BIND"] = f"127.0.0.1:{port}"
        log_path = Path(temp) / "server.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([BINARY], cwd=ROOT, env=env,
                                       stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            for _ in range(100):
                if process.poll() is not None:
                    raise AssertionError("Rizzy exited: " + log_path.read_text()[-2000:])
                try:
                    urllib.request.urlopen(rust["url"] + "/up", timeout=.2).close()
                    break
                except OSError:
                    time.sleep(.1)
            else:
                raise AssertionError("Rizzy did not start: " + log_path.read_text()[-2000:])
            browser_login(source)
            browser_login(rust)
            source_result = run(source, uuid.uuid4().hex[:8])
            rust_result = run(rust, uuid.uuid4().hex[:8])
            assert source_result == rust_result, (source_result, rust_result)
            print("MySQL search matched Fizzy: JSON and browser hits, stemming, card filters, edits, and deletes")
        except Exception:
            print(log_path.read_text()[-2000:])
            raise
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=10)


if __name__ == "__main__":
    main()
