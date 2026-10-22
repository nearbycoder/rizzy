#!/usr/bin/env python3
"""Confirm signed email links across two MySQL-backed Rizzy processes."""

import sys
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from email_change_cluster import check  # noqa: E402
from sustained_mixed import prepare_rust  # noqa: E402
from sustained_mixed_mysql import (create_databases, drop_databases,  # noqa: E402
                                   mysql_admin, rust_env)

CLIENT = "/tmp/rizzy-mysql8/bin/mysql"
SOCKET = "/tmp/rizzy-mysql8/mysql.sock"
HOST = "127.0.0.1"
PORT = 33080


def main():
    version = mysql_admin(CLIENT, SOCKET, "SELECT VERSION()").strip()
    original_trust = mysql_admin(
        CLIENT, SOCKET, "SELECT @@GLOBAL.log_bin_trust_function_creators").strip()
    database = "rztest_email_" + uuid.uuid4().hex[:12]
    with tempfile.TemporaryDirectory(prefix="rizzy-email-mysql-") as temporary:
        directory = Path(temporary)
        mysql_admin(CLIENT, SOCKET, "SET GLOBAL log_bin_trust_function_creators=1")
        try:
            create_databases(CLIENT, SOCKET, [database])
            env = rust_env(directory, HOST, PORT, database)
            env.pop("RIZZY_SECRET_KEY_BASE", None)
            env["RIZZY_DEV_MAGIC_LINKS"] = "1"
            env, credentials = prepare_rust(directory, env)
            check(directory, env, credentials)
            print(f"PASS MySQL {version} email link across two Rizzy processes")
        finally:
            try:
                drop_databases(CLIENT, SOCKET, [database])
            finally:
                mysql_admin(CLIENT, SOCKET,
                            f"SET GLOBAL log_bin_trust_function_creators={original_trust}")


if __name__ == "__main__":
    main()
