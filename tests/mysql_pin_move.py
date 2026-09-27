#!/usr/bin/env python3
"""Race a member pin with a move to a private board they cannot access."""

import concurrent.futures
import hashlib
import os
import re
import subprocess
import sys
import tempfile
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path

from mysql_cluster import call
from mysql_webhook_cleanup import MYSQL, SOCKET, root_mysql
from mysql_webhooks import BINARY, ROOT, start, stop


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
                   "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
            seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                    capture_output=True, text=True, check=True).stdout
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
                print("MySQL pin/move passed: no inaccessible pin after two blocked requests")
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
