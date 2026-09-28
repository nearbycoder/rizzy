#!/usr/bin/env python3
"""Check MySQL due-card processing and concurrent repeat commands."""

import concurrent.futures
import os
import re
import signal
import subprocess
import tempfile
import time
import urllib.request
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

from mysql_cluster import BINARY, ROOT, call, free_port, mysql_scalar


def main():
    url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if not url or not os.environ.get("RIZZY_TEST_MYSQL_CLIENT"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL and RIZZY_TEST_MYSQL_CLIENT")
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-auto-postpone-") as temp:
        env = {**os.environ, "RIZZY_DATABASE_URL": url,
               "RIZZY_SEED_EMAIL": "mysql-auto-" + uuid.uuid4().hex + "@example.test",
               "RIZZY_STORAGE_DIR": str(Path(temp) / "storage"),
               "RIZZY_DISABLE_AUTO_POSTPONE": "1",
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
               "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
        subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env, check=True, capture_output=True)
        seed = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                              check=True, capture_output=True, text=True).stdout
        account = re.search(r"Account: (\d+)", seed).group(1)
        token = re.search(r"Bearer token: (\w+)", seed).group(1)
        port = free_port()
        env["RIZZY_BIND"] = f"127.0.0.1:{port}"
        log = (Path(temp) / "server.log").open("w")
        process = subprocess.Popen([BINARY], cwd=ROOT, env=env, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        base = f"http://127.0.0.1:{port}"
        try:
            for _ in range(100):
                if process.poll() is not None:
                    raise AssertionError(Path(log.name).read_text()[-1200:])
                try:
                    with urllib.request.urlopen(base + "/up", timeout=.2) as response:
                        if response.status == 200:
                            break
                except OSError:
                    time.sleep(.1)
            else:
                raise AssertionError("MySQL Rizzy server did not start")
            status, board, _ = call(base, account, token, "/boards", "POST",
                                    {"board": {"name": "Automatic postponement"}})
            assert status == 201
            due_count = 205
            due_ids = []
            for index in range(due_count):
                status, due, _ = call(base, account, token,
                                      f"/boards/{board['id']}/cards", "POST",
                                      {"card": {"title": f"Due {index}"}})
                assert status == 201
                due_ids.append(due["id"])
            status, recent, _ = call(base, account, token, f"/boards/{board['id']}/cards", "POST",
                                     {"card": {"title": "Recent"}})
            assert status == 201
            status, stale_flag, _ = call(base, account, token,
                                         f"/boards/{board['id']}/cards", "POST",
                                         {"card": {"title": "Stale flag without record"}})
            assert status == 201
            status, stale_record, _ = call(base, account, token,
                                           f"/boards/{board['id']}/cards", "POST",
                                           {"card": {"title": "Stale record without flag"}})
            assert status == 201
            status, _, _ = call(base, account, token,
                                f"/cards/{stale_record['number']}/not_now", "POST")
            assert status == 204
        finally:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=10)
            log.close()
        stamp = (datetime.now(timezone.utc) - timedelta(days=31)).isoformat(
            timespec="milliseconds").replace("+00:00", "Z")
        mysql_scalar(url, f"UPDATE cards SET last_active_at='{stamp}' "
                          f"WHERE board_id='{board['id']}' AND id<>'{recent['id']}'")
        mysql_scalar(url, f"UPDATE cards SET postponed=1 WHERE id='{stale_flag['id']}'")
        mysql_scalar(url, f"UPDATE cards SET postponed=0 WHERE id='{stale_record['id']}'")
        plan = mysql_scalar(url,
            f"EXPLAIN SELECT c.id FROM cards c WHERE c.board_id='{board['id']}' "
            "AND c.status='published' AND c.closed=0 AND c.postponed=0 "
            f"AND c.last_active_at<='{stamp}' AND NOT EXISTS(SELECT 1 FROM card_not_nows n "
            "WHERE n.card_id=c.id) ORDER BY c.last_active_at,c.id LIMIT 100")
        assert "cards_board_active" in plan and "Not exists" in plan, plan

        def run():
            return subprocess.run([BINARY, "auto-postpone"], cwd=ROOT, env=env,
                                  capture_output=True, text=True, check=True,
                                  timeout=120).stdout.strip()

        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            outputs = list(pool.map(lambda _: run(), range(2)))
        counts = [int(re.search(r"Auto-postponed (\d+) cards", output).group(1))
                  for output in outputs]
        assert sum(counts) == due_count + 1, outputs
        assert run() == "Auto-postponed 0 cards"
        assert mysql_scalar(url,
            f"SELECT COUNT(*) FROM card_not_nows n JOIN cards c ON c.id=n.card_id "
            f"WHERE c.board_id='{board['id']}'") == str(due_count + 2)
        assert mysql_scalar(url,
            f"SELECT COUNT(*) FROM card_not_nows WHERE card_id='{recent['id']}'") == "0"
        assert mysql_scalar(url,
            f"SELECT u.role FROM card_not_nows n JOIN users u ON u.id=n.user_id "
            f"WHERE n.card_id='{stale_flag['id']}'") == "system"
        assert mysql_scalar(url,
            f"SELECT u.role FROM card_not_nows n JOIN users u ON u.id=n.user_id "
            f"WHERE n.card_id='{stale_record['id']}'") == "owner"
        assert mysql_scalar(url,
            f"SELECT COUNT(*) FROM events WHERE board_id='{board['id']}' "
            f"AND action='card_auto_postponed'") == str(due_count + 1)
        assert mysql_scalar(url,
            f"SELECT COUNT(*) FROM comments m JOIN cards c ON c.id=m.card_id "
            f"WHERE c.board_id='{board['id']}' "
            f"AND m.body LIKE '%due to inactivity%'") == str(due_count + 1)
        assert len(set(due_ids)) == due_count
        print(f"MySQL auto-postponement passed across {due_count} ordinary due cards, both stale flag cases, and concurrent commands")


if __name__ == "__main__":
    main()
