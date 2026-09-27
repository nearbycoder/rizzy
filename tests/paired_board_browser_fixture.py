#!/usr/bin/env python3
"""Keep equivalent board fixtures alive for paired browser inspection."""

import json
import os
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from card_workflow import expect  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-browser-board-") as temporary:
        processes = []
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env, credentials = prepare_source(directory, 2)
                else:
                    env = rust_env(directory)
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                    env, credentials = prepare_rust(directory, env)
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url,
                          "account": str(credentials["account"]),
                          "token": credentials["token"], "email": "owner@example.com",
                          "db": str(directory / ("db.sqlite3" if name == "fizzy"
                                                 else "rizzy.sqlite3"))}
                board, _ = expect(target, "/boards", "POST",
                                  {"board": {"name": "Paired board", "all_access": True}}, 201)
                for index in range(3):
                    expect(target, f"/boards/{board['id']}/cards", "POST",
                           {"card": {"title": f"Idea {index + 1}"}}, 201)
                print(json.dumps({"name": name,
                                  "board_url": f"{url}/{target['account']}/boards/{board['id']}",
                                  "cards_url": f"{url}/{target['account']}/cards",
                                  "database": target["db"]}), flush=True)
            print("Sign in through each browser's email form, then inspect the URLs above.", flush=True)
            try:
                while True:
                    time.sleep(1)
            except KeyboardInterrupt:
                pass
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
