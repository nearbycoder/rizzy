#!/usr/bin/env python3
"""Isolate MySQL card trigger cost during a large column deletion.

Requires the local disposable Oracle MySQL 8 server used by the MySQL bench.
"""

import argparse
import hashlib
import json
import os
import statistics
import subprocess
import tempfile
import uuid
from pathlib import Path

from mysql_column_delete import fixture, quoted
from sustained_mixed import ROOT, RUST, prepare_rust, start_server, stop_server
from sustained_mixed_mysql import (create_databases, drop_databases, mysql_admin,
                                   mysql_query, rust_env)

CLIENT = "/tmp/rizzy-mysql8/bin/mysql"
SOCKET = "/tmp/rizzy-mysql8/mysql.sock"


def create_trigger(database, statement):
    env = {**os.environ, "LD_LIBRARY_PATH": "/tmp/rizzy-mysql8/lib"}
    subprocess.run([CLIENT, "--no-defaults", f"--socket={SOCKET}", "-uroot",
                    f"--database={database}", "--delimiter=//",
                    f"--execute={statement}//"], env=env, check=True,
                   capture_output=True, text=True)


def install_guard_triggers(database):
    for name in ("cards_touch_board_update", "cards_touch_column_update"):
        mysql_query(CLIENT, SOCKET, database, f"DROP TRIGGER {name}")
    create_trigger(database, """
      CREATE TRIGGER cards_touch_board_update AFTER UPDATE ON cards FOR EACH ROW
      BEGIN
        IF NEW.status='published'
           AND NOT EXISTS(SELECT 1 FROM card_bulk_touches WHERE board_id=NEW.board_id) THEN
          UPDATE boards SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z')
          WHERE id=NEW.board_id;
        END IF;
      END
    """)
    create_trigger(database, """
      CREATE TRIGGER cards_touch_column_update AFTER UPDATE ON cards FOR EACH ROW
      BEGIN
        DECLARE stamp VARCHAR(40);
        IF NOT EXISTS(SELECT 1 FROM card_bulk_touches WHERE board_id=NEW.board_id) THEN
          SET stamp=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z');
          IF NEW.column_id IS NOT NULL THEN
            UPDATE columns SET updated_at=stamp WHERE id=NEW.column_id;
          END IF;
          IF OLD.column_id IS NOT NULL AND NOT (OLD.column_id <=> NEW.column_id) THEN
            UPDATE columns SET updated_at=stamp WHERE id=OLD.column_id;
          END IF;
        END IF;
      END
    """)


def sample(size, epoch, trial):
    database = "rzbulktouch_" + uuid.uuid4().hex[:12]
    try:
        create_databases(CLIENT, SOCKET, [database])
        with tempfile.TemporaryDirectory(prefix="rizzy-mysql-bulk-probe-") as temp:
            directory = Path(temp)
            env, credentials = prepare_rust(directory,
                rust_env(directory, "127.0.0.1", 33080, database))
            if not epoch:
                install_guard_triggers(database)
            process, url = start_server("rizzy", directory, env,
                                        sorted(os.sched_getaffinity(0))[:2])
            try:
                target = {"name": "rizzy", "url": url, "account": str(credentials["account"]),
                          "token": credentials["token"], "database": database}
                item = fixture(target, size, trial, CLIENT, SOCKET)
                board = quoted(item["board"]["id"])
                column = quoted(item["delete_column"])
                changes = ",bulk_touch_epoch=bulk_touch_epoch+1" if epoch else ""
                statements = [
                    ("lock", f"SELECT id FROM boards WHERE id={board} FOR UPDATE"),
                ]
                if not epoch:
                    statements.append(("guard", f"INSERT INTO card_bulk_touches(board_id) VALUES({board})"))
                statements.extend([
                    ("nullify", f"UPDATE cards SET column_id=NULL{changes} WHERE board_id={board} AND column_id={column}"),
                    ("touch_cards", f"UPDATE cards SET updated_at='2026-09-26T00:00:00.000Z'{changes} WHERE board_id={board}"),
                    ("delete_column", f"DELETE FROM `columns` WHERE id={column} AND board_id={board}"),
                ])
                if not epoch:
                    statements.append(("unguard", f"DELETE FROM card_bulk_touches WHERE board_id={board}"))
                statements.append(("touch_board", f"UPDATE boards SET updated_at='2026-09-26T00:00:00.000Z' WHERE id={board}"))
                sql = "SET @total=SYSDATE(6); START TRANSACTION; "
                for name, statement in statements:
                    sql += (f"SET @started=SYSDATE(6); {statement}; "
                            f"SELECT {quoted(name)},TIMESTAMPDIFF(MICROSECOND,@started,SYSDATE(6)); ")
                sql += "COMMIT; SELECT 'total',TIMESTAMPDIFF(MICROSECOND,@total,SYSDATE(6));"
                lines = mysql_query(CLIENT, SOCKET, database, sql).splitlines()
                result = {parts[0]: int(parts[1]) for parts in
                          (line.split("\t") for line in lines) if len(parts) == 2}
                assert result["total"] > 0, result
                assert int(mysql_query(CLIENT, SOCKET, database,
                    f"SELECT COUNT(*) FROM cards WHERE board_id={board} AND column_id={column}")) == 0
                return result
            finally:
                stop_server(process)
    finally:
        drop_databases(CLIENT, SOCKET, [database])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=10000)
    parser.add_argument("--trials", type=int, default=3)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.cards < 3 or args.cards > 100000 or args.trials < 1:
        parser.error("cards must be 3 to 100000 and trials positive")
    mysql_admin(CLIENT, SOCKET, "SELECT 1")
    with RUST.open("rb") as binary:
        binary_sha256 = hashlib.file_digest(binary, "sha256").hexdigest()
    result = {"cards": args.cards, "trials": args.trials,
              "rizzy_revision": subprocess.check_output(
                  ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "rizzy_binary_sha256": binary_sha256,
              "variants": {"guard_query": [], "row_epoch": []}}
    for trial in range(args.trials):
        order = (("guard_query", False), ("row_epoch", True)) if trial % 2 == 0 else (
            ("row_epoch", True), ("guard_query", False))
        for name, epoch in order:
            reading = sample(args.cards, epoch, trial)
            result["variants"][name].append(reading)
            print(trial, name, reading, flush=True)
    result["median_us"] = {name: {step: round(statistics.median(
        entry[step] for entry in readings)) for step in readings[0]}
        for name, readings in result["variants"].items()}
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
