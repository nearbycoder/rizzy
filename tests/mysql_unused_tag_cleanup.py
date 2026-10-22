#!/usr/bin/env python3
"""Check unused-tag cleanup and saved-filter links on Oracle MySQL."""

import concurrent.futures
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid
from pathlib import Path

from mysql_cluster import BINARY, ROOT, call, free_port, mysql_scalar


def main():
    url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if not url or not os.environ.get("RIZZY_TEST_MYSQL_CLIENT"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL and RIZZY_TEST_MYSQL_CLIENT")
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-unused-tags-") as temporary:
        directory = Path(temporary)
        env = {**os.environ, "RIZZY_DATABASE_URL": url,
               "RIZZY_SEED_EMAIL": "mysql-unused-" + uuid.uuid4().hex + "@example.test",
               "RIZZY_STORAGE_DIR": str(directory / "storage"),
               "RIZZY_DISABLE_TAG_CLEANUP": "1",
               "RIZZY_DISABLE_AUTO_POSTPONE": "1",
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
               "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
        subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env,
                       capture_output=True, check=True)
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True).stdout
        account = re.search(r"Account: (\d+)", seeded).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded).group(1)
        port = free_port()
        env["RIZZY_BIND"] = f"127.0.0.1:{port}"
        log_path = directory / "server.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([BINARY], cwd=ROOT, env=env, stdout=log,
                                       stderr=subprocess.STDOUT, start_new_session=True)
        base = f"http://127.0.0.1:{port}"
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
                raise AssertionError("MySQL Rizzy server did not start")
            status, board, _ = call(base, account, token, "/boards", "POST",
                                    {"board": {"name": "MySQL tag cleanup"}})
            assert status == 201, (status, board)
            status, card, _ = call(base, account, token,
                                   f"/boards/{board['id']}/cards", "POST",
                                   {"card": {"title": "Keep used tag"}})
            assert status == 201, (status, card)
            status, _, _ = call(base, account, token,
                                 f"/cards/{card['number']}/taggings", "POST",
                                 {"tag_title": "mysql-used-tag"})
            assert status == 204, status
        finally:
            process.terminate()
            process.wait(timeout=10)

        account_id, creator_id = mysql_scalar(url,
            "SELECT a.id,u.id FROM accounts a JOIN users u ON u.account_id=a.id "
            f"WHERE a.external_account_id={int(account)} AND u.role='owner'").split("\t")
        used_id = mysql_scalar(url,
            f"SELECT id FROM tags WHERE account_id='{account_id}' AND title='mysql-used-tag'")
        suffix = uuid.uuid4().hex[:12]
        tags = {name: "t" + suffix + name for name in
                ("orphan", "only", "collision", "survivor")}
        filters = {name: "f" + suffix + name for name in
                   ("tag_only", "colliding", "closed", "surviving")}
        for name, tag_id in tags.items():
            mysql_scalar(url,
                "INSERT INTO tags(id,account_id,title,created_at) VALUES"
                f"('{tag_id}','{account_id}','mysql-cleanup-{name}-{suffix}',"
                "'2026-01-01T00:00:00Z')")
        bulk = ["b" + suffix + str(index).zfill(4) for index in range(205)]
        values = ",".join(
            f"('{tag_id}','{account_id}','mysql-cleanup-bulk-{index}-{suffix}',"
            "'2026-01-01T00:00:00Z')"
            for index, tag_id in enumerate(bulk))
        mysql_scalar(url, "INSERT INTO tags(id,account_id,title,created_at) VALUES" + values)

        def save_filter(name, fields, tag=None):
            encoded = json.dumps(fields, sort_keys=True, separators=(",", ":"))
            digest = hashlib.sha256(encoded.encode()).hexdigest()
            filter_id = filters[name]
            mysql_scalar(url,
                "INSERT INTO filters(id,account_id,creator_id,fields,params_digest,created_at,updated_at) VALUES"
                f"('{filter_id}','{account_id}','{creator_id}','{encoded}','{digest}',"
                "'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')")
            if tag:
                mysql_scalar(url, f"INSERT INTO filters_tags VALUES('{filter_id}','{tags[tag]}')")

        save_filter("tag_only", {"tag_ids": [tags["only"]]}, "only")
        save_filter("colliding", {"indexed_by": "closed", "tag_ids": [tags["collision"]]},
                    "collision")
        save_filter("closed", {"indexed_by": "closed"})
        save_filter("surviving", {"sorted_by": "oldest", "tag_ids": [tags["survivor"]]},
                    "survivor")

        def run():
            return subprocess.run([BINARY, "delete-unused-tags"], cwd=ROOT, env=env,
                                  capture_output=True, text=True, timeout=30, check=True).stdout

        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            outputs = list(pool.map(lambda _: run(), range(2)))
        count = sum(int(re.search(r"Deleted (\d+) unused tags", output).group(1))
                    for output in outputs)
        assert count == 4 + len(bulk), outputs
        assert mysql_scalar(url,
            f"SELECT id FROM tags WHERE account_id='{account_id}'") == used_id
        remaining = mysql_scalar(url,
            f"SELECT id FROM filters WHERE account_id='{account_id}' ORDER BY id").splitlines()
        assert remaining == sorted([filters["closed"], filters["surviving"]]), remaining
        fields, digest = mysql_scalar(url,
            f"SELECT fields,params_digest FROM filters WHERE id='{filters['surviving']}'").split("\t")
        assert json.loads(fields) == {"sorted_by": "oldest"}, fields
        assert digest == hashlib.sha256(fields.encode()).hexdigest()
        assert mysql_scalar(url,
            f"SELECT COUNT(*) FROM filters_tags WHERE filter_id LIKE 'f{suffix}%'") == "0"
        assert run().strip() == "Deleted 0 unused tags"
        print("MySQL unused-tag cleanup passed across 209 unused tags, saved filters, and concurrent commands")


if __name__ == "__main__":
    main()
