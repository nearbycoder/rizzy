#!/usr/bin/env python3
"""Exercise Rizzy's initial MySQL-backed API workflow on a disposable database.

Set RIZZY_TEST_MYSQL_URL to a pre-created MySQL database URL and
RIZZY_TEST_MYSQL_CLIENT to a MySQL CLI. This check adds one account and its
records; use a database dedicated to tests.
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
from datetime import datetime, timedelta, timezone
from pathlib import Path
from urllib.parse import quote, unquote, urlencode, urlparse

from auth import call as auth_call, cookie


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def call(base, token, path, method="GET", body=None, expected=200, extra_headers=None):
    request = urllib.request.Request(
        base + path,
        json.dumps(body).encode() if body is not None else None,
        method=method,
        headers={
            "Authorization": "Bearer " + token,
            "Accept": "application/json",
            "Content-Type": "application/json",
            **(extra_headers or {}),
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


def mysql_scalar(url, statement):
    parsed = urlparse(url)
    client_env = {**os.environ, "MYSQL_PWD": unquote(parsed.password or "")}
    result = subprocess.run(
        [os.environ["RIZZY_TEST_MYSQL_CLIENT"], "--no-defaults",
         "--host=" + (parsed.hostname or "localhost"),
         "--port=" + str(parsed.port or 3306),
         "--user=" + unquote(parsed.username or ""),
         "--database=" + parsed.path.lstrip("/"),
         "--skip-column-names", "--execute=" + statement],
        env=client_env, check=True, capture_output=True, text=True,
    )
    return result.stdout.strip()


def save_filter(base, session, scope, board_id):
    request = urllib.request.Request(
        base + scope + "/filters",
        urlencode({"indexed_by": "closed", "board_ids[]": board_id}).encode(),
        method="POST",
        headers={"Cookie": session,
                 "Accept": "text/vnd.turbo-stream.html",
                 "Content-Type": "application/x-www-form-urlencoded"},
    )
    with urllib.request.urlopen(request, timeout=8) as response:
        assert response.status == 200
        body = response.read().decode()
    return re.search(r"/filters/([a-z0-9-]+)", body).group(1)


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
    if not os.environ.get("RIZZY_TEST_MYSQL_CLIENT"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_CLIENT to the MySQL CLI")
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
            "RIZZY_DEV_MAGIC_LINKS": "1",
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
            assert [item["id"] for item in call(base, token, scope + "/search?q=" + quote("MySQL card"))] == [card["id"]]
            soon_path = scope + "/cards?" + quote("board_ids[]") + "=" + board["id"] + "&indexed_by=postponing_soon"

            def age_card(days):
                stamp = (datetime.now(timezone.utc) - timedelta(days=days)).isoformat(
                    timespec="milliseconds").replace("+00:00", "Z")
                mysql_scalar(url, f"UPDATE cards SET last_active_at='{stamp}' WHERE id='{card['id']}'")

            assert call(base, token, soon_path) == []
            age_card(25)
            assert [item["id"] for item in call(base, token, soon_path)] == [card["id"]]
            age_card(31)
            assert call(base, token, soon_path) == []
            age_card(0)
            column = call(
                base, token, scope + f"/boards/{board['id']}/columns", "POST",
                {"column": {"name": "MySQL column"}}, 201,
            )
            call(
                base, token, scope + f"/cards/{number}/triage", "POST",
                {"column_id": column["id"]}, 204,
            )
            assert call(base, token, scope + f"/cards/{number}")["column"]["id"] == column["id"]
            step = call(
                base, token, scope + f"/cards/{number}/steps", "POST",
                {"step": {"content": "MySQL step"}}, 201,
            )
            call(
                base, token, scope + f"/cards/{number}/steps/{step['id']}", "PUT",
                {"step": {"completed": True}},
            )
            assert call(base, token, scope + f"/cards/{number}")["steps"][0]["completed"] is True
            call(
                base, token, scope + f"/cards/{number}/taggings", "POST",
                {"tag_title": "#mysql"}, 204,
            )
            assert call(base, token, scope + f"/cards/{number}")["tags"] == ["mysql"]
            assert call(base, token, scope + "/account/settings")["auto_postpone_period_in_days"] == 30
            assert call(
                base, token, scope + "/account/entropy", "PUT",
                {"entropy": {"auto_postpone_period_in_days": 7}},
            )["auto_postpone_period_in_days"] == 7
            age_card(6)
            assert [item["id"] for item in call(base, token, soon_path)] == [card["id"]]
            age_card(8)
            assert call(base, token, soon_path) == []
            age_card(0)
            assert call(base, token, scope + f"/boards/{board['id']}/entropy", "PUT",
                        {"board": {"auto_postpone_period_in_days": 11}})["auto_postpone_period_in_days"] == 11
            age_card(10)
            assert [item["id"] for item in call(base, token, soon_path)] == [card["id"]]
            age_card(13)
            assert call(base, token, soon_path) == []
            age_card(0)
            controls = {"X-Rizzy-Json-Request": "1"}
            call(base, token, scope + f"/cards/{number}/self_assignment", "POST",
                 expected=204, extra_headers=controls)
            spike = mysql_scalar(url, f"SELECT id FROM card_activity_spikes WHERE card_id='{card['id']}'")
            assert spike
            call(base, token, scope + f"/cards/{number}/self_assignment", "POST",
                 expected=204, extra_headers=controls)
            call(base, token, scope + f"/cards/{number}/self_assignment", "POST",
                 expected=204, extra_headers=controls)
            assert mysql_scalar(url, f"SELECT id FROM card_activity_spikes WHERE card_id='{card['id']}'") == spike
            def watch_record():
                return tuple(mysql_scalar(url, f"SELECT id,watching,created_at,updated_at FROM watches WHERE card_id='{card['id']}'").split("\t"))

            first_watch = watch_record()
            assert len(first_watch) == 4 and first_watch[1] == "1"
            call(base, token, scope + f"/cards/{number}/watch", "DELETE",
                 expected=204, extra_headers=controls)
            off_watch = watch_record()
            assert off_watch[0] == first_watch[0] and off_watch[1] == "0"
            call(base, token, scope + f"/cards/{number}/watch", "DELETE",
                 expected=204, extra_headers=controls)
            assert watch_record() == off_watch
            call(base, token, scope + f"/cards/{number}/watch", "POST",
                 expected=204, extra_headers=controls)
            on_watch = watch_record()
            assert on_watch[0] == first_watch[0] and on_watch[1] == "1"
            call(base, token, scope + f"/cards/{number}/watch", "POST",
                 expected=204, extra_headers=controls)
            assert watch_record() == on_watch
            call(base, token, scope + f"/cards/{number}/pin", "POST",
                 expected=204, extra_headers=controls)
            pin = mysql_scalar(url, f"SELECT id FROM pins WHERE card_id='{card['id']}'")
            call(base, token, scope + f"/cards/{number}/pin", "POST",
                 expected=204, extra_headers=controls)
            assert pin and mysql_scalar(url, f"SELECT id FROM pins WHERE card_id='{card['id']}'") == pin
            published = call(
                base, token, scope + f"/boards/{board['id']}/publication", "POST",
                expected=201,
            )
            with urllib.request.urlopen(published["public_url"], timeout=8) as response:
                assert "MySQL core" in response.read().decode()
            call(base, token, scope + f"/boards/{board['id']}/publication", "DELETE", expected=204)
            comment = call(
                base, token, scope + f"/cards/{number}/comments", "POST",
                {"comment": {"body": "MySQL comment"}}, 201,
            )
            assert [item["id"] for item in call(base, token, scope + "/search?q=" + quote("comment"))] == [card["id"]]
            stemmed = call(
                base, token, scope + f"/boards/{board['id']}/cards", "POST",
                {"card": {"title": "Running authentication"}}, 201,
            )
            stemmed_number = stemmed["number"]
            assert [item["id"] for item in call(base, token, scope + "/search?q=run")] == [stemmed["id"]]
            assert [item["id"] for item in call(base, token, scope + "/search?q=authenticate")] == [stemmed["id"]]
            assert [item["id"] for item in call(base, token, scope + "/cards?terms%5B%5D=authenticate")] == [stemmed["id"]]
            _, login_headers = auth_call(port, "/session", "POST", {"email_address": email}, status=201)
            pending = cookie(login_headers, "pending_authentication_token")
            _, login_headers = auth_call(
                port, "/session/magic_link", "POST",
                {"code": login_headers["X-Magic-Link-Code"]}, [pending],
            )
            session = cookie(login_headers, "session_token")
            html_request = urllib.request.Request(
                base + scope + "/search?q=run",
                headers={"Accept": "text/html", "Cookie": session},
            )
            with urllib.request.urlopen(html_request, timeout=8) as response:
                html = response.read().decode()
            assert "authentication" in html and 'class="search__title"' in html, html[:4000]
            for path, marker in ((f"/cards/{number}", "MySQL card"),
                                 ("/my/menu", "MySQL core")):
                request = urllib.request.Request(
                    base + scope + path,
                    headers={"Accept": "text/html", "Cookie": session},
                )
                with urllib.request.urlopen(request, timeout=8) as response:
                    assert marker in response.read().decode(), path
            prompt_request = urllib.request.Request(
                base + scope + "/prompts/cards?filter=authenticate",
                headers={"Accept": "text/html", "Cookie": session},
            )
            with urllib.request.urlopen(prompt_request, timeout=8) as response:
                assert "Running authentication" in response.read().decode()
            walk = call(
                base, token, scope + f"/cards/{stemmed_number}/comments", "POST",
                {"comment": {"body": "Walking swiftly"}}, 201,
            )
            assert [item["id"] for item in call(base, token, scope + "/search?q=walk")] == [stemmed["id"]]
            call(
                base, token, scope + f"/cards/{stemmed_number}/comments/{walk['id']}", "PUT",
                {"comment": {"body": "Sprinting swiftly"}},
            )
            assert call(base, token, scope + "/search?q=walk") == []
            assert [item["id"] for item in call(base, token, scope + "/search?q=sprint")] == [stemmed["id"]]
            call(
                base, token, scope + f"/cards/{stemmed_number}/comments/{walk['id']}",
                "DELETE", expected=204,
            )
            assert call(base, token, scope + "/search?q=sprint") == []
            call(
                base, token, scope + f"/cards/{stemmed_number}", "PUT",
                {"card": {"title": "Jumping authentication"}},
            )
            # The title-change system comment intentionally retains the old title.
            assert [item["id"] for item in call(base, token, scope + "/search?q=jump")] == [stemmed["id"]]
            call(base, token, scope + f"/cards/{stemmed_number}", "DELETE", expected=204)
            assert call(base, token, scope + "/search?q=jump") == []
            saved_id = save_filter(base, session, scope, board["id"])
            assert save_filter(base, session, scope, board["id"]) == saved_id
            assert call(base, token, scope + "/cards?filter_id=" + saved_id) == []
            assert mysql_scalar(url, f"SELECT COUNT(*) FROM boards_filters WHERE filter_id='{saved_id}'") == "1"
            call(base, token, scope + f"/cards/{number}/closure", "POST", expected=204)
            loaded = call(base, token, scope + f"/cards/{number}")
            assert loaded["closed"] is True and loaded["title"] == "MySQL card", loaded
            assert comment["body"]["plain_text"] == "MySQL comment", comment
            assert call(base, token, scope + "/cards") == []
            assert [item["id"] for item in call(base, token, scope + "/cards?filter_id=" + saved_id)] == [card["id"]]
            filter_delete = urllib.request.Request(
                base + scope + "/filters/" + saved_id, method="DELETE",
                headers={"Cookie": session,
                         "Accept": "text/vnd.turbo-stream.html"},
            )
            with urllib.request.urlopen(filter_delete, timeout=8) as response:
                assert response.status == 200
            assert len(call(base, token, scope + f"/cards/{number}/comments")) >= 1
            assert len(call(base, token, scope + "/activities")) >= 3
            metadata_checked = check_metadata(url, account)
            detail = ", and metadata" if metadata_checked else ""
            print(f"MySQL core passed: seed, board, card, column, triage, step, tag, entropy, closing soon, assignment, watch, pin, saved filter, public board, comment, search, browser card/menu, close, reads{detail}")
        except Exception:
            print(log_path.read_text()[-2000:])
            raise
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=10)


if __name__ == "__main__":
    main()
