#!/usr/bin/env python3
"""Check account card numbering and write durability across two Rizzy processes."""
import concurrent.futures
import os
import re
import sqlite3
import subprocess
import tempfile
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from join import call


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "cluster.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}", "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
        seed = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seed.stdout).group(1)
        token = re.search(r"Bearer token: (\w+)", seed.stdout).group(1)
        first_port, first = start(env)
        second_port, second = start(env)
        try:
            board, _ = call(first_port, f"/{account}/boards", "POST", {"board": {"name": "Cluster writes"}}, bearer=token, status=201)
            path = f"/{account}/boards/{board['id']}/cards"

            def create_card(index):
                port = first_port if index % 2 == 0 else second_port
                result, _ = call(port, path, "POST", {"card": {"title": f"Concurrent {index}"}}, bearer=token, status=201)
                return result["number"]

            with concurrent.futures.ThreadPoolExecutor(max_workers=24) as pool:
                numbers = list(pool.map(create_card, range(100)))
            assert len(set(numbers)) == 100, "duplicate card number"
            assert numbers and sorted(numbers) == list(range(min(numbers), min(numbers) + 100))
            with sqlite3.connect(database) as db:
                count, distinct_count = db.execute("SELECT COUNT(*),COUNT(DISTINCT number) FROM cards WHERE board_id=?", (board["id"],)).fetchone()
                assert (count, distinct_count) == (100, 100)
                account_count = db.execute("SELECT cards_count FROM accounts WHERE external_account_id=?", (account,)).fetchone()[0]
                assert account_count == max(numbers)
        finally:
            stop(second)
            stop(first)
    print("two-process concurrent writes passed: 100 cards, unique contiguous numbers")


if __name__ == "__main__":
    main()
