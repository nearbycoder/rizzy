#!/usr/bin/env python3
"""Exercise the full card write workflow through two SQLite-backed processes."""
import concurrent.futures
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from join import call

sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import workflow  # noqa: E402


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "cluster.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}",
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded.stdout).group(1)
        first_port, first = start(env)
        second_port, second = start(env)
        try:
            for round_number in range(3):
                board, _ = call(first_port, f"/{account}/boards", "POST",
                                {"board": {"name": f"Cluster workflow {round_number}"}},
                                bearer=token, status=201)

                def run(index):
                    port = first_port if index % 2 == 0 else second_port
                    target = {"url": f"http://127.0.0.1:{port}",
                              "account": account, "token": token}
                    return workflow(target, board["id"], index)

                with concurrent.futures.ThreadPoolExecutor(max_workers=24) as pool:
                    results = list(pool.map(run, range(200)))
                numbers = [result["number"] for result in results]
                assert len(set(numbers)) == 200
                with sqlite3.connect(database) as db:
                    cards = db.execute("SELECT COUNT(*) FROM cards WHERE board_id=? AND closed=1",
                                       (board["id"],)).fetchone()[0]
                    events = db.execute("SELECT COUNT(*) FROM events WHERE board_id=?",
                                        (board["id"],)).fetchone()[0]
                    comments = db.execute("SELECT COUNT(*) FROM comments WHERE card_id IN "
                                          "(SELECT id FROM cards WHERE board_id=?)",
                                          (board["id"],)).fetchone()[0]
                assert (cards, events, comments) == (200, 800, 600), (cards, events, comments)
        finally:
            stop(second)
            stop(first)
            if sys.exc_info()[0]:
                for name, process in (("first", first), ("second", second)):
                    print(f"{name} server errors: {process.stderr.read().decode()[-3000:]}",
                          file=sys.stderr)
    print("two-process card workflows passed: 600 cards, 2,400 events, 1,800 comments")


if __name__ == "__main__":
    main()
