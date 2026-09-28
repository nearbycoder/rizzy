#!/usr/bin/env python3
"""Check MySQL polymorphic cleanup through comment, card, and board deletion."""

import os
import re
import signal
import subprocess
import tempfile
import time
import urllib.request
import uuid
from pathlib import Path

from mysql_cluster import free_port
from mysql_core import call, mysql_scalar


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"
STAMP = "2026-01-01T00:00:00.000Z"


def main():
    url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if not url or not url.startswith("mysql:"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL to a dedicated MySQL database")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-delete-") as temp:
        env = {**os.environ, "RIZZY_DATABASE_URL": url,
               "RIZZY_SEED_EMAIL": "delete-" + uuid.uuid4().hex + "@example.test",
               "RIZZY_STORAGE_DIR": str(Path(temp) / "storage"),
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
               "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True).stdout
        account = re.search(r"Account: (\d+)", seeded).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded).group(1)
        account_id = mysql_scalar(url, f"SELECT id FROM accounts WHERE external_account_id={account}")
        user_id = mysql_scalar(url, f"SELECT id FROM users WHERE account_id='{account_id}' AND role='owner'")
        port = free_port()
        base = f"http://127.0.0.1:{port}"
        scope = f"/{account}"
        log_path = Path(temp) / "server.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([BINARY], cwd=ROOT,
                                       env={**env, "RIZZY_BIND": f"127.0.0.1:{port}"},
                                       stdout=log, stderr=subprocess.STDOUT,
                                       start_new_session=True)
        try:
            for _ in range(100):
                if process.poll() is not None:
                    raise AssertionError(log_path.read_text()[-2000:])
                try:
                    with urllib.request.urlopen(base + "/up", timeout=.2):
                        break
                except OSError:
                    time.sleep(.1)
            else:
                raise AssertionError("server did not start")

            def rows(table, condition):
                return int(mysql_scalar(url, f"SELECT COUNT(*) FROM {table} WHERE {condition}"))

            def board(name):
                return call(base, token, scope + "/boards", "POST",
                            {"board": {"name": name, "public_description": "Board text"}}, 201)

            def card(board_id, title):
                return call(base, token, scope + f"/boards/{board_id}/cards", "POST",
                            {"card": {"title": title, "description": "Card text"}}, 201)

            def comment(card_number):
                return call(base, token, scope + f"/cards/{card_number}/comments", "POST",
                            {"comment": {"body": "Comment text"}}, 201)

            def polymorphic_fixture(card_id, comment_id, notify=False):
                mention_id = uuid.uuid4().hex
                for kind, target in (("Card", card_id), ("Comment", comment_id)):
                    mysql_scalar(url, f"INSERT INTO reactions(id,account_id,reactable_type,reactable_id,reacter_id,content,created_at) "
                                      f"VALUES('{uuid.uuid4().hex}','{account_id}','{kind}','{target}','{user_id}','ok','{STAMP}')")
                    mysql_scalar(url, f"INSERT INTO mentions(id,account_id,mentionee_id,mentioner_id,source_type,source_id,created_at,updated_at) "
                                      f"VALUES('{mention_id if kind == 'Comment' else uuid.uuid4().hex}','{account_id}','{user_id}','{user_id}','{kind}','{target}','{STAMP}','{STAMP}')")
                if notify:
                    mysql_scalar(url, f"INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,created_at,updated_at) "
                                      f"VALUES('{uuid.uuid4().hex}','{account_id}','{user_id}','{user_id}','{card_id}','{mention_id}','Mention','{STAMP}','{STAMP}')")

            first = board("Delete comment")
            first_card = card(first["id"], "Comment cleanup")
            first_comment = comment(first_card["number"])
            polymorphic_fixture(first_card["id"], first_comment["id"], notify=True)
            assert rows("rich_text_records", f"record_type='Comment' AND record_id='{first_comment['id']}'") == 1
            call(base, token, scope + f"/cards/{first_card['number']}/comments/{first_comment['id']}",
                 "DELETE", expected=204)
            for table, kind in (("rich_text_records", "record_type"),
                                ("mentions", "source_type"), ("reactions", "reactable_type")):
                assert rows(table, f"{kind}='Comment' AND " +
                            ("record_id" if table == "rich_text_records" else
                             "source_id" if table == "mentions" else "reactable_id") +
                            f"='{first_comment['id']}'") == 0, table
            assert rows("notifications", f"card_id='{first_card['id']}' AND source_type='Mention'") == 0

            event_comment = comment(first_card["number"])
            event_id = mysql_scalar(url, f"SELECT id FROM events WHERE eventable_type='Comment' AND eventable_id='{event_comment['id']}'")
            assert event_id
            mysql_scalar(url, f"INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,created_at,updated_at) "
                              f"VALUES('{uuid.uuid4().hex}','{account_id}','{user_id}','{user_id}','{first_card['id']}','{event_id}','Event','{STAMP}','{STAMP}')")
            call(base, token, scope + f"/cards/{first_card['number']}/comments/{event_comment['id']}",
                 "DELETE", expected=204)
            assert rows("notifications", f"card_id='{first_card['id']}' AND source_type='Event'") == 0

            second_card = card(first["id"], "Card cleanup")
            second_comment = comment(second_card["number"])
            polymorphic_fixture(second_card["id"], second_comment["id"])
            call(base, token, scope + f"/cards/{second_card['number']}", "DELETE", expected=204)
            assert_absent(rows, second_card["id"], second_comment["id"])

            third = board("Delete board")
            third_card = card(third["id"], "Board cleanup")
            third_comment = comment(third_card["number"])
            polymorphic_fixture(third_card["id"], third_comment["id"])
            moved_event = uuid.uuid4().hex
            mysql_scalar(url, f"INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,created_at) "
                              f"VALUES('{moved_event}','{account_id}','{third['id']}','{first_card['id']}','{user_id}','Card','{first_card['id']}','card_changed','{STAMP}')")
            mysql_scalar(url, f"INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,created_at,updated_at) "
                              f"VALUES('{uuid.uuid4().hex}','{account_id}','{user_id}','{user_id}','{first_card['id']}','{moved_event}','Event','{STAMP}','{STAMP}')")
            call(base, token, scope + f"/boards/{third['id']}", "DELETE", expected=204)
            assert_absent(rows, third_card["id"], third_comment["id"])
            assert rows("rich_text_records", f"record_type='Board' AND record_id='{third['id']}'") == 0
            assert rows("notifications", f"source_type='Event' AND source_id='{moved_event}'") == 0
            assert rows("cards", f"id='{first_card['id']}'") == 1
            print("MySQL deletion cleanup passed: comment, card, board")
        except Exception:
            print(log_path.read_text()[-2000:])
            raise
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=10)


def assert_absent(rows, card_id, comment_id):
    for table, kind, key in (("rich_text_records", "record_type", "record_id"),
                             ("mentions", "source_type", "source_id"),
                             ("reactions", "reactable_type", "reactable_id")):
        for type_name, target in (("Card", card_id), ("Comment", comment_id)):
            assert rows(table, f"{kind}='{type_name}' AND {key}='{target}'") == 0, (table, type_name)


if __name__ == "__main__":
    main()
