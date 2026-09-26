#!/usr/bin/env python3
"""Compare Rizzy board conditional reads before and after the data-version fast path."""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests"))
from board_html_cached_etag import port, request, start
from events_html import browser_cookie, html_response, run


def sample(binary, label, cards, seconds, concurrencies):
    with tempfile.TemporaryDirectory(prefix=f"rizzy-board-etag-{label}-") as temp:
        database = Path(temp) / "rizzy.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}",
               "RIZZY_STORAGE_DIR": str(Path(temp) / "storage"),
               "RIZZY_DEV_MAGIC_LINKS": "1",
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1"}
        seeded = subprocess.run([binary, "seed"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=True).stdout
        account = re.search(r"Account: (\d+)", seeded).group(1)
        token = re.search(r"Bearer token: (\w+)", seeded).group(1)
        process, url = start(env, f"127.0.0.1:{port()}", Path(temp) / "server.log", binary)
        try:
            status, _, raw = request(url, token, f"/{account}/boards", "POST",
                                     {"board": {"name": "Conditional benchmark"}})
            assert status == 201, (status, raw)
            board = json.loads(raw)["id"]
            for index in range(cards):
                status, _, raw = request(url, token, f"/{account}/boards/{board}/cards", "POST",
                                         {"card": {"title": f"Benchmark card {index}"}})
                assert status == 201, (status, raw)
            target = {"name": "rizzy", "url": url, "account": account,
                      "email": "owner@example.com", "db": str(database)}
            target["cookie"] = browser_cookie(target)
            path = f"/boards/{board}"
            status, headers, body = html_response(target, path)
            assert status == 200 and body
            etag = headers["ETag"]
            assert len(re.findall(rb'id="article_card_[^"]+"', body)) == min(cards, 15)
            status, _, body_304 = html_response(target, path, etag)
            assert status == 304 and not body_304
            measurements = {}
            for concurrency in concurrencies:
                measurements[str(concurrency)] = {
                    "full": run(target, path, concurrency, seconds),
                    "conditional": run(target, path, concurrency, seconds, etag),
                }
            return {"binary": str(binary), "cards": cards,
                    "html_bytes": len(body), "measurements": measurements}
        finally:
            process.terminate()
            process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--before", required=True, type=Path)
    parser.add_argument("--after", required=True, type=Path)
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=3)
    parser.add_argument("--concurrency", default="8,32")
    args = parser.parse_args()
    concurrencies = [int(value) for value in args.concurrency.split(",")]
    results = {label: sample(binary.resolve(), label, args.cards, args.seconds, concurrencies)
               for label, binary in (("before", args.before), ("after", args.after))}
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
