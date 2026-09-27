#!/usr/bin/env python3
"""Recover two interrupted MySQL schema installs in disposable databases.

Requires RIZZY_TEST_MYSQL_URL for a local account with database grants and a
socket-authenticated root MySQL client. The test creates and drops its own
rzresume_ schemas; it never modifies the database named in the URL.
"""

import os
import re
import subprocess
import uuid
from pathlib import Path
from urllib.parse import unquote, urlparse, urlunparse


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"
SCHEMA = ROOT / "schema.mysql.sql"
CLIENT = Path(os.environ.get("RIZZY_TEST_MYSQL_CLIENT", "/tmp/rizzy-mysql8/bin/mysql"))
SOCKET = os.environ.get("RIZZY_TEST_MYSQL_ROOT_SOCKET", "/tmp/rizzy-mysql8/mysql.sock")


def mysql(statement, database=None):
    env = {**os.environ, "LD_LIBRARY_PATH": str(CLIENT.resolve().parents[1] / "lib")}
    command = [str(CLIENT), "--no-defaults", "--batch", "--skip-column-names",
               f"--socket={SOCKET}", "-uroot"]
    if database:
        command.append(database)
    result = subprocess.run(command, input=statement, env=env, text=True,
                            capture_output=True, check=True)
    return result.stdout.strip()


def statements():
    clean = "\n".join(line for line in SCHEMA.read_text().splitlines()
                      if not line.lstrip().startswith("--"))
    return [part.strip() for part in clean.split(";") if part.strip()]


def preload(database, stage, ddl):
    if stage == "tables":
        selected = [ddl[0], *[item for item in ddl if item.startswith("CREATE TABLE ")][:12]]
        mysql(";\n".join(selected) + ";", database)
        mysql("INSERT INTO accounts(id,external_account_id,name,created_at) "
              "VALUES('retained-account',42,'Retained','2026-01-01T00:00:00.000Z');", database)
        return
    assert stage == "generated"
    generated_at = next(index for index, item in enumerate(ddl)
                        if item.startswith("ALTER TABLE `storage_attachments` ADD COLUMN "))
    mysql(";\n".join(ddl[:generated_at]) + ";", database)
    definitions = ddl[generated_at].split(" ADD COLUMN ", 1)[1].split(", ADD COLUMN ")
    mysql(";\n".join("ALTER TABLE `storage_attachments` ADD COLUMN " + definition
                      for definition in definitions[:2]) + ";", database)


def main():
    parsed = urlparse(os.environ.get("RIZZY_TEST_MYSQL_URL", ""))
    if parsed.scheme != "mysql" or not parsed.username or not parsed.path.strip("/"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL to a dedicated MySQL URL")
    username = unquote(parsed.username)
    if not re.fullmatch(r"[A-Za-z0-9_]+", username):
        raise SystemExit("MySQL test username must contain only letters, digits, or underscore")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    ddl = statements()
    for stage in ("tables", "generated"):
        database = "rzresume_" + uuid.uuid4().hex[:12]
        mysql(f"CREATE DATABASE `{database}` CHARACTER SET utf8mb4 COLLATE utf8mb4_bin; "
              f"GRANT ALL PRIVILEGES ON `{database}`.* TO '{username}'@'127.0.0.1';")
        try:
            preload(database, stage, ddl)
            env = {**os.environ,
                   "RIZZY_DATABASE_URL": urlunparse(parsed._replace(path="/" + database)),
                   "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
                   "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
            if stage == "generated":
                initializers = [subprocess.Popen([BINARY, "init-db"], cwd=ROOT, env=env,
                                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                                 text=True) for _ in range(2)]
                for process in initializers:
                    _, error = process.communicate(timeout=60)
                    assert process.returncode == 0, (stage, "concurrent init-db", error[-3000:])
                commands = ("seed", "init-db")
            else:
                commands = ("init-db", "seed", "init-db")
            for command in commands:
                result = subprocess.run([BINARY, command], cwd=ROOT, env=env,
                                        capture_output=True, text=True, timeout=60)
                assert result.returncode == 0, (stage, command, result.stderr[-3000:])
            tables = int(mysql("SELECT COUNT(*) FROM information_schema.tables "
                               "WHERE table_schema=DATABASE();", database))
            expected_tables = sum(item.startswith("CREATE TABLE ") for item in ddl) + 16
            assert tables == expected_tables, (stage, tables)
            triggers = int(mysql("SELECT COUNT(*) FROM information_schema.triggers "
                                 "WHERE trigger_schema=DATABASE();", database))
            assert triggers == 24, (stage, triggers)
            accounts = int(mysql("SELECT COUNT(*) FROM accounts;", database))
            assert accounts == (2 if stage == "tables" else 1), (stage, accounts)
            if stage == "tables":
                retained = mysql("SELECT name FROM accounts WHERE id='retained-account';", database)
                assert retained == "Retained", retained
            print(f"MySQL schema resumed after {stage}: {tables} tables, {accounts} accounts")
        finally:
            mysql(f"DROP DATABASE IF EXISTS `{database}`;")


if __name__ == "__main__":
    main()
