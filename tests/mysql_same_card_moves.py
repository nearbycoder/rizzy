#!/usr/bin/env python3
"""Force two app processes to move one image-bearing card from a stale board."""

import concurrent.futures
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

from card_image_parity import IMAGE
from mysql_cluster import call
from mysql_webhook_cleanup import MYSQL, SOCKET, root_mysql
from mysql_webhooks import BINARY, ROOT, start, stop
from storage_ledger_parity import api, upload

PORT = int(os.environ.get("MYSQL_PORT", "33080"))


def main():
    if not SOCKET.is_socket():
        raise SystemExit("Start the disposable MySQL 8 server first")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    suffix = uuid.uuid4().hex[:12]
    database = "rzsamecard_" + suffix
    username = database
    password = uuid.uuid4().hex
    root_mysql(
        f"CREATE DATABASE {database} CHARACTER SET utf8mb4 COLLATE utf8mb4_bin; "
        f"CREATE USER '{username}'@'127.0.0.1' IDENTIFIED BY '{password}'; "
        f"GRANT ALL PRIVILEGES ON {database}.* TO '{username}'@'127.0.0.1';"
    )
    try:
        def scalar(statement):
            return root_mysql(f"USE {database}; {statement}")

        with tempfile.TemporaryDirectory(prefix="rizzy-same-card-moves-") as temporary:
            env = {
                **os.environ,
                "RIZZY_DATABASE_URL": f"mysql://{username}:{password}@127.0.0.1:{PORT}/{database}",
                "RIZZY_STORAGE_DIR": str(Path(temporary) / "storage"),
                "RIZZY_SEED_EMAIL": "same-card-" + suffix + "@example.test",
                "RIZZY_DISABLE_STORAGE_MATERIALIZATION": "1",
                "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
                "RIZZY_DISABLE_PUSH_DELIVERY": "1",
            }
            seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                    capture_output=True, text=True, check=True).stdout
            account = re.search(r"Account: (\d+)", seeded).group(1)
            token = re.search(r"Bearer token: (\w+)", seeded).group(1)
            servers = []
            blocker = None
            try:
                for name in ("first", "second"):
                    servers.append(start(env, temporary, name))
                target = {"name": "rizzy", "url": servers[0][0],
                          "account": account, "token": token}
                names = ("Move source", "Move destination B", "Move destination C")
                boards = [api(target, "/boards", "POST", {"board": {"name": name}}, 201)
                          for name in names]
                card = api(target, f"/boards/{boards[0]['id']}/cards", "POST",
                           {"card": {"title": "Concurrent image move"}}, 201)
                image = upload(target, IMAGE, "moon.jpg", "image/jpeg")
                api(target, f"/cards/{card['number']}.json", "PATCH",
                    {"card": {"image": image["signed_id"]}})
                card_id = card["id"]
                old_board = boards[0]["id"]

                # Hold the source board while both HTTP handlers load the old
                # card location, then release them into the same move race.
                blocker = subprocess.Popen(
                    [str(MYSQL), "--no-defaults", f"--socket={SOCKET}", "-uroot",
                     "--unbuffered", "--batch", "--skip-column-names",
                     "--execute=" + "START TRANSACTION; "
                     f"SELECT id FROM {database}.boards WHERE id='{old_board}' FOR UPDATE; "
                     "DO SLEEP(8); COMMIT;"],
                    env={**os.environ, "LD_LIBRARY_PATH": str(MYSQL.resolve().parents[1] / "lib")},
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                )
                assert blocker.stdout.readline().strip() == old_board

                def move(index):
                    return call(servers[index][0], account, token,
                                f"/cards/{card['number']}/board", "PATCH",
                                {"board_id": boards[index + 1]["id"]})

                with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                    moves = [pool.submit(move, index) for index in range(2)]
                    deadline = time.monotonic() + 6
                    waits = 0
                    while time.monotonic() < deadline:
                        waits = int(root_mysql(
                            "SELECT COUNT(*) FROM performance_schema.data_lock_waits w "
                            "JOIN performance_schema.data_locks l ON "
                            "l.ENGINE_LOCK_ID=w.REQUESTING_ENGINE_LOCK_ID "
                            "WHERE l.OBJECT_NAME='boards';"
                        ))
                        if waits >= 2:
                            break
                        time.sleep(.1)
                    assert waits >= 2, ("both moves did not reach the board lock", waits)
                    responses = [future.result(timeout=20) for future in moves]
                assert all(status == 200 for status, _, _ in responses), responses
                out, error = blocker.communicate(timeout=10)
                assert blocker.returncode == 0, (out, error)
                blocker = None

                status, shown, _ = call(servers[0][0], account, token,
                                        f"/cards/{card['number']}")
                assert status == 200 and shown["board"]["id"] in (
                    boards[1]["id"], boards[2]["id"]), shown
                final_name = names[1] if shown["board"]["id"] == boards[1]["id"] else names[2]
                intermediate_name = names[2] if final_name == names[1] else names[1]
                particulars = [json.loads(value) for value in scalar(
                    "SELECT particulars FROM events WHERE "
                    f"card_id='{card_id}' AND action='card_board_changed' "
                    "ORDER BY created_at,id;"
                ).splitlines()]
                assert len(particulars) == 2, particulars
                assert sorted(value["old_board"] for value in particulars) == sorted(
                    (names[0], intermediate_name)), particulars
                assert sorted(value["new_board"] for value in particulars) == sorted(
                    names[1:]), particulars

                board_ids = [board["id"] for board in boards]
                amounts = [int(scalar(
                    "SELECT CAST(COALESCE(SUM(delta),0) AS SIGNED) FROM storage_entries "
                    f"WHERE board_id='{board_id}';"
                )) for board_id in board_ids]
                expected = [len(IMAGE) if board_id == shown["board"]["id"] else 0
                            for board_id in board_ids]
                assert amounts == expected, (amounts, expected, particulars)
                print("Same-card MySQL moves passed: two blocked writers, accurate event sources and storage ledger")
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
