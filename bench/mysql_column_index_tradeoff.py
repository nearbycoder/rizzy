#!/usr/bin/env python3
"""Compare MySQL column index layouts on matching Rizzy fixtures."""

import argparse
import json
import os
import statistics
import subprocess
import tempfile
import uuid
from pathlib import Path

from mysql_column_delete import fixture, quoted
from sustained_mixed import prepare_rust, start_server, stop_server
from sustained_mixed_mysql import create_databases, drop_databases, mysql_admin, mysql_query, rust_env

CLIENT = "/tmp/rizzy-mysql8/bin/mysql"
SOCKET = "/tmp/rizzy-mysql8/mysql.sock"
INDEXES = {
    "wide": None,
    "wide_forced": None,
    "ordered_compact": "(board_id,column_id,golden,last_active_at,id)",
    "ordered_compact_forced": "(board_id,column_id,golden,last_active_at,id)",
    "filter_compact": "(board_id,column_id,status,closed,postponed)",
}


def sql_time(database, statement, repetitions=40):
    sql = "SET @started=SYSDATE(6); " + statement * repetitions
    sql += " SELECT TIMESTAMPDIFF(MICROSECOND,@started,SYSDATE(6));"
    return int(mysql_query(CLIENT, SOCKET, database, sql).splitlines()[-1])


def sample(size, layout, trial, cpus):
    database = "rzcolindex_" + uuid.uuid4().hex[:12]
    create_databases(CLIENT, SOCKET, [database])
    try:
        with tempfile.TemporaryDirectory(prefix="rizzy-mysql-index-") as temp:
            directory = Path(temp)
            env, credentials = prepare_rust(
                directory, rust_env(directory, "127.0.0.1", 33080, database))
            process, url = start_server("rizzy", directory, env, cpus)
            try:
                target = {"name": "rizzy", "url": url,
                          "account": str(credentials["account"]),
                          "token": credentials["token"], "database": database}
                item = fixture(target, size, trial, CLIENT, SOCKET)
                if index := INDEXES[layout]:
                    mysql_query(CLIENT, SOCKET, database,
                                "DROP INDEX cards_board_column_active ON cards")
                    mysql_query(CLIENT, SOCKET, database,
                                f"CREATE INDEX cards_board_column_active ON cards {index}")
                board = quoted(item["board"]["id"])
                column = quoted(item["delete_column"])
                where = (f"board_id={board} AND status='published' AND closed=0 AND postponed=0")
                reads = {}
                cards_table = ("cards FORCE INDEX(cards_board_column_active)"
                               if layout in ("wide_forced", "ordered_compact_forced") else "cards")
                for name, condition in (("named", f"column_id={column}"),
                                        ("maybe", "column_id IS NULL")):
                    scope = f"{where} AND {condition}"
                    reads[name + "_page_40_us"] = sql_time(database,
                        f"SELECT id FROM {cards_table} WHERE {scope} ORDER BY golden DESC,last_active_at DESC,id DESC LIMIT 15;")
                    reads[name + "_count_40_us"] = sql_time(database,
                        f"SELECT COUNT(*) FROM {cards_table} WHERE {scope};")
                    reads[name + "_explain"] = mysql_query(CLIENT, SOCKET, database,
                        f"EXPLAIN SELECT id FROM {cards_table} WHERE {scope} ORDER BY golden DESC,last_active_at DESC,id DESC LIMIT 15")
                sql = (f"SET @started=SYSDATE(6); START TRANSACTION; "
                       f"SELECT id FROM boards WHERE id={board} FOR UPDATE; "
                       f"UPDATE cards SET column_id=CASE WHEN column_id={column} THEN NULL ELSE column_id END,"
                       f"updated_at='2026-09-26T00:00:00.000Z',bulk_touch_epoch=bulk_touch_epoch+1 "
                       f"WHERE board_id={board}; "
                       f"DELETE FROM `columns` WHERE id={column} AND board_id={board}; "
                       f"UPDATE boards SET updated_at='2026-09-26T00:00:00.000Z' WHERE id={board}; "
                       f"COMMIT; SELECT TIMESTAMPDIFF(MICROSECOND,@started,SYSDATE(6));")
                write_us = int(mysql_query(CLIENT, SOCKET, database, sql).splitlines()[-1])
                row = mysql_query(CLIENT, SOCKET, database,
                    f"SELECT COUNT(*),SUM(updated_at='2026-09-26T00:00:00.000Z'),"
                    f"SUM(last_active_at={quoted(item['marker'])}),COALESCE(SUM(column_id={column}),0) "
                    f"FROM cards WHERE board_id={board}").split("\t")
                assert [int(value) for value in row] == [size, size, size, 0], (layout, row)
                return {"reads": reads, "delete_us": write_us}
            finally:
                stop_server(process)
    finally:
        drop_databases(CLIENT, SOCKET, [database])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=10000)
    parser.add_argument("--trials", type=int, default=3)
    parser.add_argument("--layouts", nargs="+", choices=tuple(INDEXES), default=list(INDEXES))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.cards < 3 or args.cards > 100000 or args.trials < 1:
        parser.error("cards must be 3 to 100000 and trials positive")
    mysql_admin(CLIENT, SOCKET, "SELECT 1")
    cpus = sorted(os.sched_getaffinity(0))[:2]
    result = {"cards": args.cards, "trials": args.trials,
              "rizzy_revision": subprocess.check_output(
                  ["git", "rev-parse", "HEAD"], text=True).strip(),
              "server_cpus": cpus,
              "layouts": {name: [] for name in args.layouts}}
    for trial in range(args.trials):
        order = list(args.layouts)
        if trial % 2:
            order.reverse()
        for layout in order:
            reading = sample(args.cards, layout, trial, cpus)
            result["layouts"][layout].append(reading)
            print(trial, layout, reading["delete_us"], flush=True)
    result["median"] = {name: {
        "delete_us": round(statistics.median(item["delete_us"] for item in readings)),
        **{key: round(statistics.median(item["reads"][key] for item in readings))
           for key in readings[0]["reads"] if key.endswith("_us")},
    } for name, readings in result["layouts"].items()}
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["median"], indent=2))


if __name__ == "__main__":
    main()
