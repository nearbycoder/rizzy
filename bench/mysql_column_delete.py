#!/usr/bin/env python3
"""Compare matched authenticated column deletes on disposable MySQL accounts."""

import argparse
import hashlib
import json
import os
import statistics
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

from card_workflow import expect
from sustained_mixed import ROOT, RUST, SOURCE, prepare_rust, prepare_source, start_server, stop_server
from sustained_mixed_mysql import (create_databases, drop_databases, mysql_admin,
                                   mysql_query, rust_env, source_env)


def quoted(value):
    return "'" + value.replace("'", "''") + "'"


def sequence_sql():
    digit = "(SELECT 0 n UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9)"
    terms = [f"{10 ** index} * d{index}.n" for index in range(5)]
    joins = " CROSS JOIN ".join(f"{digit} d{index}" for index in range(5))
    return f"(SELECT {' + '.join(terms)} AS n FROM {joins}) seq"


def fixture(target, size, trial, client, socket):
    board_name = f"MySQL column delete {size} trial {trial}"
    board, _ = expect(target, "/boards", "POST", {"board": {"name": board_name}}, 201)
    path = f"/boards/{board['id']}"
    columns = []
    for name in ("Delete", "Keep"):
        column, _ = expect(target, path + "/columns", "POST",
                           {"column": {"name": name}}, 201)
        columns.append(column)
    cards = []
    for name in ("Delete card", "Keep card", "Maybe card"):
        card, _ = expect(target, path + "/cards", "POST",
                         {"card": {"title": name}}, 201)
        cards.append(card)
    for card, column in zip(cards[:2], columns):
        expect(target, f"/cards/{card['number']}/triage", "POST",
               {"column_id": column["id"]}, 204)

    database = target["database"]
    marker = ("2020-01-01 00:00:00.000000" if target["name"] == "fizzy"
              else "2020-01-01T00:00:00.000Z")
    board_filter = f"b.name={quoted(board_name)}"
    first_number = int(mysql_query(client, socket, database,
        f"SELECT MAX(c.number)+1 FROM cards c JOIN boards b ON b.id=c.board_id WHERE {board_filter}"))
    id_fields = ("HEX(b.id),HEX(b.account_id),HEX(b.creator_id),HEX(d.id),HEX(k.id)"
                 if target["name"] == "fizzy" else
                 "b.id,b.account_id,b.creator_id,d.id,k.id")
    ids = mysql_query(client, socket, database,
        f"SELECT {id_fields} FROM boards b JOIN `columns` d ON d.board_id=b.id AND d.name='Delete' "
        f"JOIN `columns` k ON k.board_id=b.id AND k.name='Keep' WHERE {board_filter}").split("\t")
    assert len(ids) == 5, (target["name"], ids)
    ids = [f"UNHEX({quoted(value)})" for value in ids] if target["name"] == "fizzy" else [quoted(value) for value in ids]
    board_id, account_id, creator_id, delete_id, keep_id = ids
    id_expression = ("UUID_TO_BIN(UUID())" if target["name"] == "fizzy"
                     else "REPLACE(UUID(),'-','')")
    sequence = sequence_sql()
    columns_sql = "id,account_id,board_id,column_id,creator_id,number,title,status,created_at,updated_at,last_active_at"
    statement = f"""
      INSERT INTO cards({columns_sql})
      SELECT {id_expression}, {account_id}, {board_id},
             CASE MOD(seq.n,3) WHEN 0 THEN {delete_id} WHEN 1 THEN {keep_id} ELSE NULL END,
             {creator_id}, {first_number}+seq.n,
             CONCAT('Fixture card ',seq.n), 'published',
             {quoted(marker)}, {quoted(marker)}, {quoted(marker)}
      FROM {sequence}
      WHERE seq.n<{size - 3}
    """
    mysql_query(client, socket, database, statement)
    if target["name"] == "rizzy":
        # The production bulk-touch guard suppresses per-card timestamp
        # callbacks while setting a common measurement marker.
        mysql_query(client, socket, database,
            f"INSERT INTO card_bulk_touches(board_id) VALUES({board_id})")
    mysql_query(client, socket, database,
        f"UPDATE cards SET updated_at={quoted(marker)},last_active_at={quoted(marker)} WHERE board_id={board_id}")
    if target["name"] == "rizzy":
        mysql_query(client, socket, database,
            f"DELETE FROM card_bulk_touches WHERE board_id={board_id}")
    count = int(mysql_query(client, socket, database,
        f"SELECT COUNT(*) FROM cards c JOIN boards b ON b.id=c.board_id WHERE {board_filter}"))
    assert count == size, (target["name"], "fixture count", count, size)
    keep_stamp = mysql_query(client, socket, database,
        f"SELECT k.updated_at FROM `columns` k JOIN boards b ON b.id=k.board_id WHERE {board_filter} AND k.name='Keep'")
    return {"board": board, "delete_column": columns[0]["id"],
            "board_name": board_name, "marker": marker, "keep_stamp": keep_stamp,
            "size": size}


def measure(target, item, client, socket):
    started = time.perf_counter()
    expect(target, f"/boards/{item['board']['id']}/columns/{item['delete_column']}",
           "DELETE", None, 204)
    elapsed_ms = (time.perf_counter() - started) * 1000
    name = quoted(item["board_name"])
    marker = quoted(item["marker"])
    values = mysql_query(client, socket, target["database"], f"""
      SELECT COUNT(*),
             SUM(c.updated_at={marker}),
             SUM(c.last_active_at={marker}),
             SUM(c.column_id IS NULL),
             SUM(c.column_id=k.id)
      FROM cards c JOIN boards b ON b.id=c.board_id
      JOIN `columns` k ON k.board_id=b.id AND k.name='Keep'
      WHERE b.name={name}
    """).split("\t")
    counts = [int(value) for value in values]
    clone_count = item["size"] - 3
    expected_keep = 1 + sum(index % 3 == 1 for index in range(clone_count))
    expected_null = 2 + sum(index % 3 == 0 or index % 3 == 2
                            for index in range(clone_count))
    assert counts == [item["size"], 0, item["size"], expected_null, expected_keep], (
        target["name"], "delete effects", counts, expected_null, expected_keep)
    keep_stamp = mysql_query(client, socket, target["database"],
        f"SELECT k.updated_at FROM `columns` k JOIN boards b ON b.id=k.board_id WHERE b.name={name} AND k.name='Keep'")
    assert keep_stamp == item["keep_stamp"], (target["name"], "untouched column", keep_stamp)
    return round(elapsed_ms, 3)


def run_trial(args, size, trial, cpus):
    prefix = "rzdelete_" + uuid.uuid4().hex[:12]
    source_dbs = {role: prefix + "_f" + suffix for role, suffix in
                  (("", ""), ("CABLE_", "_c"), ("CACHE_", "_k"), ("QUEUE_", "_q"))}
    rust_db = prefix + "_r"
    databases = [*source_dbs.values(), rust_db]
    processes = []
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-column-delete-") as temp:
        root = Path(temp)
        try:
            create_databases(args.mysql_client, args.mysql_socket, databases)
            targets = {}
            for name in ("fizzy", "rizzy"):
                directory = root / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, args.source_workers,
                    source_env(directory, args.source_workers, args.mysql_host,
                               args.mysql_port, source_dbs)) if name == "fizzy" else
                    prepare_rust(directory, rust_env(directory, args.mysql_host,
                                                     args.mysql_port, rust_db)))
                process, url = start_server(name, directory, env, cpus)
                processes.append(process)
                targets[name] = {"name": name, "url": url,
                                 "account": str(credentials["account"]),
                                 "token": credentials["token"],
                                 "database": source_dbs[""] if name == "fizzy" else rust_db}
            fixtures = {name: fixture(targets[name], size, trial,
                                      args.mysql_client, args.mysql_socket)
                        for name in ("fizzy", "rizzy")}
            order = ("fizzy", "rizzy") if trial % 2 == 0 else ("rizzy", "fizzy")
            times = {name: measure(targets[name], fixtures[name],
                                   args.mysql_client, args.mysql_socket) for name in order}
            return times
        finally:
            for process in processes:
                stop_server(process)
            drop_databases(args.mysql_client, args.mysql_socket, databases)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--sizes", nargs="+", type=int, default=[1000, 10000])
    parser.add_argument("--trials", type=int, default=3)
    parser.add_argument("--source-workers", type=int, default=2)
    parser.add_argument("--mysql-client", default="/tmp/rizzy-mysql8/bin/mysql")
    parser.add_argument("--mysql-socket", default="/tmp/rizzy-mysql8/mysql.sock")
    parser.add_argument("--mysql-host", default="127.0.0.1")
    parser.add_argument("--mysql-port", type=int, default=33080)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if min(*args.sizes, args.trials, args.source_workers) < 1 or max(args.sizes) > 100000:
        parser.error("sizes must be 1 to 100000; trials and workers must be positive")
    if min(args.sizes) < 3:
        parser.error("each fixture needs at least three cards")
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    mysql_admin(args.mysql_client, args.mysql_socket, "SELECT 1")
    cpus = sorted(os.sched_getaffinity(0))[:2]
    with RUST.open("rb") as binary:
        binary_sha256 = hashlib.file_digest(binary, "sha256").hexdigest()
    result = {"database": "Oracle MySQL 8", "source_revision": subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=SOURCE, text=True).strip(),
        "source_dirty": bool(subprocess.check_output(
            ["git", "status", "--porcelain"], cwd=SOURCE)),
        "rizzy_revision": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "rizzy_dirty": bool(subprocess.check_output(
            ["git", "status", "--porcelain"], cwd=ROOT)),
        "rizzy_binary_sha256": binary_sha256,
        "mysql_version": mysql_admin(args.mysql_client, args.mysql_socket,
                                     "SELECT VERSION()").strip(),
        "fixture": "Three API cards, then SQL-cloned cards and timestamp markers outside the timed DELETE; all-card timestamp, activity, placement, and other-column state verified after each request",
        "server_cpus": cpus, "source_workers": args.source_workers,
        "rust_processes": 1, "sizes": {}}
    for size in args.sizes:
        samples = {"fizzy": [], "rizzy": []}
        for trial in range(args.trials):
            try:
                times = run_trial(args, size, trial, cpus)
            except subprocess.CalledProcessError as error:
                print(error.stderr, flush=True)
                raise
            for name, elapsed in times.items():
                samples[name].append(elapsed)
            print(size, trial, times, flush=True)
        result["sizes"][str(size)] = {"samples_ms": samples,
            "median_ms": {name: round(statistics.median(values), 3)
                          for name, values in samples.items()}}
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
