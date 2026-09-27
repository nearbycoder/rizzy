#!/usr/bin/env python3
"""Race JSON card and browser draft creation against board-access revocation."""

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

from draft_workflow_parity import NoRedirect, login, request
from mysql_cluster import call
from mysql_webhook_cleanup import MYSQL, SOCKET, root_mysql
from mysql_webhooks import BINARY, ROOT, start, stop


def main():
    if not SOCKET.is_socket():
        raise SystemExit("Start the disposable MySQL 8 server first")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    suffix = uuid.uuid4().hex[:12]
    database = "rzcreateaccess_" + suffix
    password = uuid.uuid4().hex
    root_mysql(f"CREATE DATABASE {database} CHARACTER SET utf8mb4 COLLATE utf8mb4_bin; "
               f"CREATE USER '{database}'@'127.0.0.1' IDENTIFIED BY '{password}'; "
               f"GRANT ALL PRIVILEGES ON {database}.* TO '{database}'@'127.0.0.1';")
    try:
        def scalar(statement):
            return root_mysql(f"USE {database}; {statement}")

        def board_waits():
            return int(root_mysql(
                "SELECT COUNT(*) FROM performance_schema.data_lock_waits w "
                "JOIN performance_schema.data_locks l ON "
                "l.ENGINE_LOCK_ID=w.REQUESTING_ENGINE_LOCK_ID "
                f"WHERE l.OBJECT_SCHEMA='{database}' AND l.OBJECT_NAME='boards';"))

        def wait_for_waiters(expected):
            deadline = time.monotonic() + 6
            count = 0
            while time.monotonic() < deadline:
                count = board_waits()
                if count >= expected:
                    return
                time.sleep(.1)
            raise AssertionError(("board lock waiters", expected, count))

        with tempfile.TemporaryDirectory(prefix="rizzy-card-creation-access-") as temporary:
            env = {**os.environ,
                   "RIZZY_DATABASE_URL": f"mysql://{database}:{password}@127.0.0.1:33080/{database}",
                   "RIZZY_STORAGE_DIR": str(Path(temporary) / "storage"),
                   "RIZZY_SEED_EMAIL": f"create-access-{suffix}@example.test",
                   "RIZZY_DEV_MAGIC_LINKS": "1",
                   "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
                   "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
            seed = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                  capture_output=True, text=True)
            assert seed.returncode == 0, seed.stderr
            account = re.search(r"Account: (\d+)", seed.stdout).group(1)
            owner_token = re.search(r"Bearer token: (\w+)", seed.stdout).group(1)
            servers = []
            blocker = None
            try:
                for name in ("owner", "member"):
                    servers.append(start(env, temporary, name))
                status, board, _ = call(servers[0][0], account, owner_token, "/boards", "POST",
                                        {"board": {"name": "Creation access", "all_access": False}})
                assert status == 201, (status, board)
                account_id = scalar(f"SELECT id FROM accounts WHERE external_account_id={account};")
                owner_id = scalar(f"SELECT id FROM users WHERE account_id='{account_id}' "
                                  "AND role='owner' LIMIT 1;")
                member_id = uuid.uuid4().hex
                identity_id = uuid.uuid4().hex
                member_token = "create-access-" + uuid.uuid4().hex
                member_hash = hashlib.sha256(member_token.encode()).hexdigest()
                stamp = datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace(
                    "+00:00", "Z")
                scalar(f"INSERT INTO identities(id,email_address,created_at) "
                       f"VALUES('{identity_id}','{identity_id}@example.test','{stamp}'); "
                       f"INSERT INTO users(id,account_id,identity_id,name,role,verified_at,created_at) "
                       f"VALUES('{member_id}','{account_id}','{identity_id}','Creation Member',"
                       f"'member','{stamp}','{stamp}'); "
                       f"INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) "
                       f"VALUES('{uuid.uuid4().hex}','{identity_id}','{member_hash}',"
                       f"'Creation access','write','{stamp}'); "
                       f"INSERT INTO accesses(board_id,user_id,involvement) "
                       f"VALUES('{board['id']}','{member_id}','access_only');")
                session_target = {
                    "name": "rizzy", "url": servers[1][0], "account": account,
                    "token": member_token, "email": identity_id + "@example.test",
                    "opener": urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                        NoRedirect()),
                }
                login(session_target)
                original_count = scalar(f"SELECT COUNT(*) FROM cards WHERE board_id='{board['id']}';")
                original_number = scalar(f"SELECT cards_count FROM accounts WHERE id='{account_id}';")

                def revoke():
                    return call(servers[0][0], account, owner_token,
                                f"/boards/{board['id']}", "PATCH",
                                {"board": {"name": "Creation access"}, "user_ids": [owner_id]})

                def create_json():
                    return call(servers[1][0], account, member_token,
                                f"/boards/{board['id']}/cards", "POST",
                                {"card": {"title": "Inaccessible JSON card"}})

                def create_draft():
                    return request(session_target, f"/{account}/boards/{board['id']}/cards",
                                   "POST", b"", accept="text/html")

                for index, writer in enumerate((create_json, create_draft)):
                    if index:
                        status, _, _ = call(servers[0][0], account, owner_token,
                                            f"/boards/{board['id']}", "PATCH",
                                            {"board": {"name": "Creation access"},
                                             "user_ids": [owner_id, member_id]})
                        assert status == 200, status
                        assert scalar(f"SELECT COUNT(*) FROM accesses WHERE board_id='{board['id']}' "
                                      f"AND user_id='{member_id}';") == "1"
                    blocker = subprocess.Popen(
                        [str(MYSQL), "--no-defaults", f"--socket={SOCKET}", "-uroot",
                         "--unbuffered", "--batch", "--skip-column-names",
                         "--execute=START TRANSACTION; "
                         f"SELECT id FROM {database}.boards WHERE id='{board['id']}' FOR UPDATE; "
                         "DO SLEEP(8); COMMIT;"],
                        env={**os.environ, "LD_LIBRARY_PATH": str(MYSQL.resolve().parents[1] / "lib")},
                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                    assert blocker.stdout.readline().strip() == board["id"]
                    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                        revoke_future = pool.submit(revoke)
                        wait_for_waiters(1)
                        create_future = pool.submit(writer)
                        wait_for_waiters(2)
                        revoked = revoke_future.result(timeout=20)
                        created = create_future.result(timeout=20)
                    assert revoked[0] == 200, revoked
                    assert created[0] == 404, created
                    output, error = blocker.communicate(timeout=10)
                    assert blocker.returncode == 0, (output, error)
                    blocker = None
                    assert scalar(f"SELECT COUNT(*) FROM accesses WHERE board_id='{board['id']}' "
                                  f"AND user_id='{member_id}';") == "0"
                    assert scalar(f"SELECT COUNT(*) FROM cards WHERE board_id='{board['id']}';") == original_count
                    assert scalar(f"SELECT cards_count FROM accounts WHERE id='{account_id}';") == original_number
                print("MySQL card and draft creation access races passed")
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
                   f"DROP USER IF EXISTS '{database}'@'127.0.0.1';")


if __name__ == "__main__":
    main()
