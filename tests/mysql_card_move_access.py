#!/usr/bin/env python3
"""Race member personal and card writes with inaccessible moves."""

import concurrent.futures
import hashlib
import http.cookiejar
import os
import re
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid
from datetime import datetime, timezone
from pathlib import Path

from mysql_cluster import call
from mysql_webhook_cleanup import MYSQL, SOCKET, root_mysql
from mysql_webhooks import BINARY, ROOT, start, stop
from draft_workflow_parity import NoRedirect, login, request


def main():
    if not SOCKET.is_socket():
        raise SystemExit("Start the disposable MySQL 8 server first")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    suffix = uuid.uuid4().hex[:12]
    database = "rzpinmove_" + suffix
    username = database
    password = uuid.uuid4().hex
    root_mysql(f"CREATE DATABASE {database} CHARACTER SET utf8mb4 COLLATE utf8mb4_bin; "
               f"CREATE USER '{username}'@'127.0.0.1' IDENTIFIED BY '{password}'; "
               f"GRANT ALL PRIVILEGES ON {database}.* TO '{username}'@'127.0.0.1';")
    try:
        def scalar(statement):
            return root_mysql(f"USE {database}; {statement}")

        with tempfile.TemporaryDirectory(prefix="rizzy-pin-move-") as temporary:
            env = {**os.environ,
                   "RIZZY_DATABASE_URL": f"mysql://{username}:{password}@127.0.0.1:33080/{database}",
                   "RIZZY_STORAGE_DIR": str(Path(temporary) / "storage"),
                   "RIZZY_SEED_EMAIL": "pin-move-" + suffix + "@example.test",
                   "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
                   "RIZZY_DISABLE_PUSH_DELIVERY": "1",
                   "RIZZY_DEV_MAGIC_LINKS": "1"}
            seed_result = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                         capture_output=True, text=True)
            assert seed_result.returncode == 0, seed_result.stderr
            seeded = seed_result.stdout
            account = re.search(r"Account: (\d+)", seeded).group(1)
            owner_token = re.search(r"Bearer token: (\w+)", seeded).group(1)
            servers = []
            blocker = None
            try:
                for name in ("owner", "member"):
                    servers.append(start(env, temporary, name))
                status, source, _ = call(servers[0][0], account, owner_token,
                                         "/boards", "POST",
                                         {"board": {"name": "Pin source", "all_access": False}})
                assert status == 201, (status, source)
                status, destination, _ = call(servers[0][0], account, owner_token,
                                              "/boards", "POST",
                                              {"board": {"name": "Private destination",
                                                         "all_access": False}})
                assert status == 201, (status, destination)
                status, card, _ = call(servers[0][0], account, owner_token,
                                       f"/boards/{source['id']}/cards", "POST",
                                       {"card": {"title": "Pin during move"}})
                assert status == 201, (status, card)
                account_id = scalar(f"SELECT id FROM accounts WHERE external_account_id={account};")
                owner_id = scalar(f"SELECT id FROM users WHERE account_id='{account_id}' "
                                  "AND role='owner' LIMIT 1;")
                member_id = uuid.uuid4().hex
                identity_id = uuid.uuid4().hex
                member_token = "pin-move-" + uuid.uuid4().hex
                member_hash = hashlib.sha256(member_token.encode()).hexdigest()
                stamp = datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace(
                    "+00:00", "Z")
                scalar(f"INSERT INTO identities(id,email_address,created_at) "
                       f"VALUES('{identity_id}','{identity_id}@example.test','{stamp}'); "
                       f"INSERT INTO users(id,account_id,identity_id,name,role,verified_at,created_at) "
                       f"VALUES('{member_id}','{account_id}','{identity_id}','Pin Member',"
                       f"'member','{stamp}','{stamp}'); "
                       f"INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) "
                       f"VALUES('{uuid.uuid4().hex}','{identity_id}','{member_hash}',"
                       f"'Pin move','write','{stamp}'); "
                       f"INSERT INTO accesses(board_id,user_id,involvement) "
                       f"VALUES('{source['id']}','{member_id}','access_only');")
                pin_path = f"/cards/{card['number']}/pin"
                status, _, _ = call(servers[1][0], account, member_token, pin_path, "POST")
                assert status == 204, status
                assert scalar(f"SELECT COUNT(*) FROM pins WHERE card_id='{card['id']}' "
                              f"AND user_id='{member_id}';") == "1"
                status, _, _ = call(servers[1][0], account, member_token, pin_path, "DELETE")
                assert status == 204, status

                blocker = subprocess.Popen(
                    [str(MYSQL), "--no-defaults", f"--socket={SOCKET}", "-uroot",
                     "--unbuffered", "--batch", "--skip-column-names",
                     "--execute=START TRANSACTION; "
                     f"SELECT id FROM {database}.boards WHERE id='{source['id']}' FOR UPDATE; "
                     "DO SLEEP(8); COMMIT;"],
                    env={**os.environ, "LD_LIBRARY_PATH": str(MYSQL.resolve().parents[1] / "lib")},
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                assert blocker.stdout.readline().strip() == source["id"]

                def move():
                    return call(servers[0][0], account, owner_token,
                                f"/cards/{card['number']}/board", "PATCH",
                                {"board_id": destination["id"]})

                def pin():
                    return call(servers[1][0], account, member_token, pin_path, "POST")

                with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                    move_future = pool.submit(move)
                    pin_future = pool.submit(pin)
                    deadline = time.monotonic() + 6
                    waits = 0
                    while time.monotonic() < deadline:
                        waits = int(root_mysql(
                            "SELECT COUNT(*) FROM performance_schema.data_lock_waits w "
                            "JOIN performance_schema.data_locks l ON "
                            "l.ENGINE_LOCK_ID=w.REQUESTING_ENGINE_LOCK_ID "
                            "WHERE l.OBJECT_NAME='boards';"))
                        if waits >= 2:
                            break
                        time.sleep(.1)
                    assert waits >= 2, ("both requests did not reach the board lock", waits)
                    moved = move_future.result(timeout=20)
                    pinned = pin_future.result(timeout=20)
                assert moved[0] == 200, moved
                assert pinned[0] in (204, 404), pinned
                output, error = blocker.communicate(timeout=10)
                assert blocker.returncode == 0, (output, error)
                blocker = None

                assert scalar(f"SELECT board_id FROM cards WHERE id='{card['id']}';") == destination["id"]
                assert scalar(f"SELECT COUNT(*) FROM pins WHERE card_id='{card['id']}' "
                              f"AND user_id='{member_id}';") == "0"
                status, _, _ = call(servers[1][0], account, member_token,
                                    f"/cards/{card['number']}")
                assert status == 404, status
                status, _, _ = call(servers[1][0], account, member_token, pin_path, "POST")
                assert status == 404, status

                status, watch_card, _ = call(servers[0][0], account, owner_token,
                                             f"/boards/{source['id']}/cards", "POST",
                                             {"card": {"title": "Watch during move"}})
                assert status == 201, (status, watch_card)
                watch_path = f"/cards/{watch_card['number']}/watch"
                status, _, _ = call(servers[1][0], account, member_token, watch_path, "POST")
                assert status == 204, status
                status, _, _ = call(servers[1][0], account, member_token, watch_path, "DELETE")
                assert status == 204, status
                assert scalar(f"SELECT watching FROM watches WHERE card_id='{watch_card['id']}' "
                              f"AND user_id='{member_id}';") == "0"

                blocker = subprocess.Popen(
                    [str(MYSQL), "--no-defaults", f"--socket={SOCKET}", "-uroot",
                     "--unbuffered", "--batch", "--skip-column-names",
                     "--execute=START TRANSACTION; "
                     f"SELECT id FROM {database}.boards WHERE id='{source['id']}' FOR UPDATE; "
                     "DO SLEEP(8); COMMIT;"],
                    env={**os.environ, "LD_LIBRARY_PATH": str(MYSQL.resolve().parents[1] / "lib")},
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                assert blocker.stdout.readline().strip() == source["id"]

                def move_watched_card():
                    return call(servers[0][0], account, owner_token,
                                f"/cards/{watch_card['number']}/board", "PATCH",
                                {"board_id": destination["id"]})

                def watch():
                    return call(servers[1][0], account, member_token, watch_path, "POST")

                with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                    move_future = pool.submit(move_watched_card)
                    watch_future = pool.submit(watch)
                    deadline = time.monotonic() + 6
                    waits = 0
                    while time.monotonic() < deadline:
                        waits = int(root_mysql(
                            "SELECT COUNT(*) FROM performance_schema.data_lock_waits w "
                            "JOIN performance_schema.data_locks l ON "
                            "l.ENGINE_LOCK_ID=w.REQUESTING_ENGINE_LOCK_ID "
                            "WHERE l.OBJECT_NAME='boards';"))
                        if waits >= 2:
                            break
                        time.sleep(.1)
                    assert waits >= 2, ("watch and move did not reach the board lock", waits)
                    moved = move_future.result(timeout=20)
                    watched = watch_future.result(timeout=20)
                assert moved[0] == 200, moved
                assert watched[0] in (204, 404), watched
                output, error = blocker.communicate(timeout=10)
                assert blocker.returncode == 0, (output, error)
                blocker = None

                assert scalar(f"SELECT board_id FROM cards WHERE id='{watch_card['id']}';") == destination["id"]
                assert scalar(f"SELECT COUNT(*) FROM watches WHERE card_id='{watch_card['id']}' "
                              f"AND user_id='{member_id}';") == "0"
                status, _, _ = call(servers[1][0], account, member_token, watch_path, "POST")
                assert status == 404, status

                status, reaction_card, _ = call(servers[0][0], account, owner_token,
                                                f"/boards/{source['id']}/cards", "POST",
                                                {"card": {"title": "Reaction during move"}})
                assert status == 201, (status, reaction_card)
                reaction_path = f"/cards/{reaction_card['number']}/reactions"
                blocker = subprocess.Popen(
                    [str(MYSQL), "--no-defaults", f"--socket={SOCKET}", "-uroot",
                     "--unbuffered", "--batch", "--skip-column-names",
                     "--execute=START TRANSACTION; "
                     f"SELECT id FROM {database}.boards WHERE id='{source['id']}' FOR UPDATE; "
                     "DO SLEEP(8); COMMIT;"],
                    env={**os.environ, "LD_LIBRARY_PATH": str(MYSQL.resolve().parents[1] / "lib")},
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                assert blocker.stdout.readline().strip() == source["id"]

                def move_reaction_card():
                    return call(servers[0][0], account, owner_token,
                                f"/cards/{reaction_card['number']}/board", "PATCH",
                                {"board_id": destination["id"]})

                def react():
                    return call(servers[1][0], account, member_token, reaction_path, "POST",
                                {"reaction": {"content": "👍"}})

                def tag():
                    return call(servers[1][0], account, member_token,
                                f"/cards/{reaction_card['number']}/taggings", "POST",
                                {"tag_title": "inaccessible-race-tag"})

                def assign():
                    return call(servers[1][0], account, member_token,
                                f"/cards/{reaction_card['number']}/assignments", "POST",
                                {"assignee_id": owner_id})

                def add_step():
                    return call(servers[1][0], account, member_token,
                                f"/cards/{reaction_card['number']}/steps", "POST",
                                {"step": {"content": "Inaccessible race step"}})

                before_assignment = scalar(
                    f"SELECT COUNT(*) FROM assignments WHERE card_id='{reaction_card['id']}' "
                    f"AND assignee_id='{owner_id}';")
                assert before_assignment == "0", before_assignment
                with concurrent.futures.ThreadPoolExecutor(max_workers=5) as pool:
                    move_future = pool.submit(move_reaction_card)
                    deadline = time.monotonic() + 6
                    waits = 0
                    while time.monotonic() < deadline:
                        waits = int(root_mysql(
                            "SELECT COUNT(*) FROM performance_schema.data_lock_waits w "
                            "JOIN performance_schema.data_locks l ON "
                            "l.ENGINE_LOCK_ID=w.REQUESTING_ENGINE_LOCK_ID "
                            "WHERE l.OBJECT_NAME='boards';"))
                        if waits >= 1:
                            break
                        time.sleep(.1)
                    assert waits >= 1, ("move did not reach the board lock", waits)
                    reaction_future = pool.submit(react)
                    tag_future = pool.submit(tag)
                    assignment_future = pool.submit(assign)
                    step_future = pool.submit(add_step)
                    deadline = time.monotonic() + 6
                    waits = 0
                    while time.monotonic() < deadline:
                        waits = int(root_mysql(
                            "SELECT COUNT(*) FROM performance_schema.data_lock_waits w "
                            "JOIN performance_schema.data_locks l ON "
                            "l.ENGINE_LOCK_ID=w.REQUESTING_ENGINE_LOCK_ID "
                            "WHERE l.OBJECT_NAME='boards';"))
                        if waits >= 5:
                            break
                        time.sleep(.1)
                    assert waits >= 5, ("card writes did not reach the board lock", waits)
                    moved = move_future.result(timeout=20)
                    reacted = reaction_future.result(timeout=20)
                    tagged = tag_future.result(timeout=20)
                    assigned = assignment_future.result(timeout=20)
                    stepped = step_future.result(timeout=20)
                assert moved[0] == 200, moved
                assert reacted[0] == 404, reacted
                assert tagged[0] == 404, tagged
                assert assigned[0] == 404, assigned
                assert stepped[0] == 404, stepped
                output, error = blocker.communicate(timeout=10)
                assert blocker.returncode == 0, (output, error)
                blocker = None
                assert scalar(f"SELECT board_id FROM cards WHERE id='{reaction_card['id']}';") == destination["id"]
                assert scalar(f"SELECT COUNT(*) FROM reactions WHERE reactable_type='Card' "
                              f"AND reactable_id='{reaction_card['id']}' AND reacter_id='{member_id}';") == "0"
                assert scalar("SELECT COUNT(*) FROM tags WHERE title='inaccessible-race-tag';") == "0"
                assert scalar(f"SELECT COUNT(*) FROM assignments WHERE card_id='{reaction_card['id']}' "
                              f"AND assignee_id='{owner_id}';") == before_assignment
                assert scalar(f"SELECT COUNT(*) FROM steps WHERE card_id='{reaction_card['id']}' "
                              "AND content='Inaccessible race step';") == "0"

                status, delete_card, _ = call(servers[1][0], account, member_token,
                                              f"/boards/{source['id']}/cards", "POST",
                                              {"card": {"title": "Delete during move"}})
                assert status == 201, (status, delete_card)
                blocker = subprocess.Popen(
                    [str(MYSQL), "--no-defaults", f"--socket={SOCKET}", "-uroot",
                     "--unbuffered", "--batch", "--skip-column-names",
                     "--execute=START TRANSACTION; "
                     f"SELECT id FROM {database}.boards WHERE id='{source['id']}' FOR UPDATE; "
                     "DO SLEEP(8); COMMIT;"],
                    env={**os.environ, "LD_LIBRARY_PATH": str(MYSQL.resolve().parents[1] / "lib")},
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                assert blocker.stdout.readline().strip() == source["id"]

                def move_member_card():
                    return call(servers[0][0], account, owner_token,
                                f"/cards/{delete_card['number']}/board", "PATCH",
                                {"board_id": destination["id"]})

                def delete_member_card():
                    return call(servers[1][0], account, member_token,
                                f"/cards/{delete_card['number']}", "DELETE")

                def read_member_card():
                    return call(servers[1][0], account, member_token,
                                f"/cards/{delete_card['number']}/reading", "POST")

                with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
                    move_future = pool.submit(move_member_card)
                    deadline = time.monotonic() + 6
                    waits = 0
                    while time.monotonic() < deadline:
                        waits = int(root_mysql(
                            "SELECT COUNT(*) FROM performance_schema.data_lock_waits w "
                            "JOIN performance_schema.data_locks l ON "
                            "l.ENGINE_LOCK_ID=w.REQUESTING_ENGINE_LOCK_ID "
                            "WHERE l.OBJECT_NAME='boards';"))
                        if waits >= 1:
                            break
                        time.sleep(.1)
                    assert waits >= 1, ("member-card move did not reach the board lock", waits)
                    delete_future = pool.submit(delete_member_card)
                    read_future = pool.submit(read_member_card)
                    deadline = time.monotonic() + 6
                    waits = 0
                    while time.monotonic() < deadline:
                        waits = int(root_mysql(
                            "SELECT COUNT(*) FROM performance_schema.data_lock_waits w "
                            "JOIN performance_schema.data_locks l ON "
                            "l.ENGINE_LOCK_ID=w.REQUESTING_ENGINE_LOCK_ID "
                            "WHERE l.OBJECT_NAME='boards';"))
                        if waits >= 3:
                            break
                        time.sleep(.1)
                    assert waits >= 3, ("delete and read did not reach the board lock", waits)
                    moved = move_future.result(timeout=20)
                    deleted = delete_future.result(timeout=20)
                    read = read_future.result(timeout=20)
                assert moved[0] == 200, moved
                assert deleted[0] == 404, deleted
                assert read[0] == 404, read
                output, error = blocker.communicate(timeout=10)
                assert blocker.returncode == 0, (output, error)
                blocker = None
                assert scalar(f"SELECT board_id FROM cards WHERE id='{delete_card['id']}';") == destination["id"]

                session_target = {
                    "name": "rizzy", "url": servers[1][0], "account": account,
                    "token": member_token, "email": identity_id + "@example.test",
                    "opener": urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                        NoRedirect()),
                }
                login(session_target)
                draft_status, draft_headers, _ = request(
                    session_target, f"/{account}/boards/{source['id']}/cards",
                    "POST", b"", accept="text/html")
                assert draft_status == 302, draft_status
                draft_location = draft_headers["Location"]
                draft_number = re.search(r"/cards/(\d+)/draft$", draft_location).group(1)
                draft_id = scalar(f"SELECT id FROM cards WHERE account_id='{account_id}' "
                                  f"AND number={draft_number};")
                assert scalar(f"SELECT status FROM cards WHERE id='{draft_id}';") == "drafted"
                before_events = scalar(f"SELECT COUNT(*) FROM events WHERE card_id='{draft_id}' "
                                       "AND action='card_published';")
                blocker = subprocess.Popen(
                    [str(MYSQL), "--no-defaults", f"--socket={SOCKET}", "-uroot",
                     "--unbuffered", "--batch", "--skip-column-names",
                     "--execute=START TRANSACTION; "
                     f"SELECT id FROM {database}.boards WHERE id='{source['id']}' FOR UPDATE; "
                     "DO SLEEP(8); COMMIT;"],
                    env={**os.environ, "LD_LIBRARY_PATH": str(MYSQL.resolve().parents[1] / "lib")},
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                assert blocker.stdout.readline().strip() == source["id"]

                def move_draft():
                    return call(servers[0][0], account, owner_token,
                                f"/cards/{draft_number}/board", "PATCH",
                                {"board_id": destination["id"]})

                def publish_draft():
                    return call(servers[1][0], account, member_token,
                                f"/cards/{draft_number}/publish", "POST")

                with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                    move_future = pool.submit(move_draft)
                    deadline = time.monotonic() + 6
                    waits = 0
                    while time.monotonic() < deadline:
                        waits = int(root_mysql(
                            "SELECT COUNT(*) FROM performance_schema.data_lock_waits w "
                            "JOIN performance_schema.data_locks l ON "
                            "l.ENGINE_LOCK_ID=w.REQUESTING_ENGINE_LOCK_ID "
                            "WHERE l.OBJECT_NAME='boards';"))
                        if waits >= 1:
                            break
                        time.sleep(.1)
                    assert waits >= 1, ("draft move did not reach the board lock", waits)
                    publish_future = pool.submit(publish_draft)
                    deadline = time.monotonic() + 6
                    waits = 0
                    while time.monotonic() < deadline:
                        waits = int(root_mysql(
                            "SELECT COUNT(*) FROM performance_schema.data_lock_waits w "
                            "JOIN performance_schema.data_locks l ON "
                            "l.ENGINE_LOCK_ID=w.REQUESTING_ENGINE_LOCK_ID "
                            "WHERE l.OBJECT_NAME='boards';"))
                        if waits >= 2:
                            break
                        time.sleep(.1)
                    assert waits >= 2, ("publish did not reach the board lock", waits)
                    moved = move_future.result(timeout=20)
                    published = publish_future.result(timeout=20)
                assert moved[0] == 200, moved
                assert published[0] == 404, published
                output, error = blocker.communicate(timeout=10)
                assert blocker.returncode == 0, (output, error)
                blocker = None
                assert scalar(f"SELECT board_id FROM cards WHERE id='{draft_id}';") == destination["id"]
                assert scalar(f"SELECT status FROM cards WHERE id='{draft_id}';") == "drafted"
                assert scalar(f"SELECT COUNT(*) FROM events WHERE card_id='{draft_id}' "
                              "AND action='card_published';") == before_events
                print("MySQL inaccessible card-write/move races passed")
            except Exception:
                for _, _, log in servers:
                    print(log.read_text()[-2000:], file=sys.stderr)
                raise
            finally:
                if blocker and blocker.poll() is None:
                    blocker.terminate()
                    blocker.communicate(timeout=10)
                for _, process, _ in servers:
                    stop(process)
    finally:
        root_mysql(f"DROP DATABASE IF EXISTS {database}; "
                   f"DROP USER IF EXISTS '{username}'@'127.0.0.1';")


if __name__ == "__main__":
    main()
