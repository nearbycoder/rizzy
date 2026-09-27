#!/usr/bin/env python3
"""Run paired parity suites on disposable Fizzy and Rizzy MySQL databases."""

import argparse
import os
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from sustained_mixed import (first_difference, prepare_rust, prepare_source,  # noqa: E402
                             start_server, stop_server)
from sustained_mixed_mysql import (create_databases, drop_databases, rust_env,  # noqa: E402
                                   source_env)
from mysql_search_parity import browser_login, run as search_run  # noqa: E402
from parity import run as core_run  # noqa: E402
from board_filter_html_parity import run as board_filter_run  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", choices=("core", "search", "board-filter"))
    args = parser.parse_args()
    client = os.environ.get("RIZZY_TEST_MYSQL_CLIENT", "/tmp/rizzy-mysql8/bin/mysql")
    socket = os.environ.get("RIZZY_TEST_MYSQL_SOCKET", "/tmp/rizzy-mysql8/mysql.sock")
    host = os.environ.get("MYSQL_HOST", "127.0.0.1")
    port = int(os.environ.get("MYSQL_PORT", "33080"))
    suffix = uuid.uuid4().hex[:12]
    source_dbs = {role: f"rzsearch_{suffix}_f{label}" for role, label in
                  (("", ""), ("CABLE_", "_c"), ("CACHE_", "_k"), ("QUEUE_", "_q"))}
    rust_db = f"rzsearch_{suffix}_r"
    databases = [*source_dbs.values(), rust_db]
    processes = []
    created = False
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-search-selfhost-") as temp:
        root = Path(temp)
        try:
            created = True
            create_databases(client, socket, databases)
            fizzy_dir, rizzy_dir = root / "fizzy", root / "rizzy"
            fizzy_dir.mkdir()
            rizzy_dir.mkdir()
            fizzy_env, fizzy_credentials = prepare_source(
                fizzy_dir, 1, source_env(fizzy_dir, 1, host, port, source_dbs))
            local_rust_env = rust_env(rizzy_dir, host, port, rust_db)
            local_rust_env["RIZZY_SEED_EMAIL"] = "mysql-search-owner@example.test"
            local_rust_env["RIZZY_DEV_MAGIC_LINKS"] = "1"
            rizzy_env, rizzy_credentials = prepare_rust(rizzy_dir, local_rust_env)
            cpus = sorted(os.sched_getaffinity(0))[:2]
            fizzy_process, fizzy_url = start_server("fizzy", fizzy_dir, fizzy_env, cpus)
            processes.append(fizzy_process)
            rizzy_process, rizzy_url = start_server("rizzy", rizzy_dir, rizzy_env, cpus)
            processes.append(rizzy_process)
            source = {"name": "fizzy", "url": fizzy_url,
                      "account": str(fizzy_credentials["account"]),
                      "token": fizzy_credentials["token"], "email": "owner@example.com",
                      "mysql_client": client, "mysql_database": source_dbs[""],
                      "mysql_host": host, "mysql_port": port, "mysql_user": "rizzy",
                      "mysql_password": "rizzy"}
            rust = {"name": "rizzy", "url": rizzy_url,
                    "account": str(rizzy_credentials["account"]),
                    "token": rizzy_credentials["token"],
                    "email": local_rust_env["RIZZY_SEED_EMAIL"]}
            if args.suite == "search":
                browser_login(source)
                browser_login(rust)
                source_result = search_run(source, uuid.uuid4().hex[:8])
                rust_result = search_run(rust, uuid.uuid4().hex[:8])
            elif args.suite == "board-filter":
                browser_login(source)
                browser_login(rust)
                source_result = board_filter_run(source, signed_in=True)
                rust_result = board_filter_run(rust, signed_in=True)
            else:
                source_result = core_run(source)
                rust_result = core_run(rust)
            difference = first_difference(source_result, rust_result)
            assert difference is None, difference
            if args.suite == "search":
                print(f"Disposable MySQL search fixture matched pinned Fizzy: "
                      f"{len(source_result['query_probe'])} query cases, "
                      f"{len(source_result['browser'])} rendered search cases")
            elif args.suite == "board-filter":
                print(f"Disposable MySQL board filters matched pinned Fizzy: "
                      f"{len(source_result)} browser workflow comparisons")
            else:
                print(f"Disposable MySQL core workflow matched pinned Fizzy: "
                      f"{len(source_result)} comparisons")
        except Exception:
            for name in ("fizzy", "rizzy"):
                log = root / name / f"{name}.log"
                if log.exists():
                    print(f"{name} log tail:\n{log.read_text()[-4000:]}", file=sys.stderr)
            raise
        finally:
            for process in processes:
                stop_server(process)
            if created:
                drop_databases(client, socket, databases)


if __name__ == "__main__":
    main()
