#!/usr/bin/env python3
"""Snapshot Rizzy's initialized SQLite schema as portable MySQL DDL.

Run `rizzy seed` against a fresh SQLite database first, then pass that file
here. The output covers tables, indexes, and foreign keys. Runtime SQL and
SQLite trigger behavior must be ported separately before MySQL is supported.
"""

import argparse
import sqlite3
from collections import defaultdict
from pathlib import Path


def quoted(name: str) -> str:
    return "`" + name.replace("`", "``") + "`"


def string_type(name: str, indexed: bool) -> str:
    if not indexed:
        return "LONGTEXT"
    if name == "id" or name.endswith("_id"):
        return "VARCHAR(64)"
    if name.endswith("_at"):
        return "VARCHAR(40)"
    if name in ("status", "role", "kind", "purpose", "permission"):
        return "VARCHAR(32)"
    return "VARCHAR(255)"


def columns_for_index(db: sqlite3.Connection, index: str) -> list[str]:
    columns = [row[2] for row in db.execute(f"PRAGMA index_info({quoted(index)})")]
    if any(column is None for column in columns):
        raise ValueError(f"expression index needs manual conversion: {index}")
    return columns


def generate(db: sqlite3.Connection) -> str:
    tables = [
        row[0]
        for row in db.execute(
            "SELECT name FROM sqlite_master WHERE type='table' "
            "AND name NOT LIKE 'sqlite_%' "
            "AND sql NOT LIKE 'CREATE VIRTUAL TABLE%' "
            "AND name NOT LIKE 'card_search_%' "
            "AND name NOT LIKE 'comment_search_%' ORDER BY rowid"
        )
    ]
    statements = [
        "-- Generated from an initialized Rizzy SQLite database.",
        "-- Search tables, triggers, and application queries require a separate MySQL port.",
        "SET NAMES utf8mb4;",
    ]
    indexes = []
    foreign_keys = []
    for table in tables:
        columns = db.execute(f"PRAGMA table_info({quoted(table)})").fetchall()
        source_sql = db.execute(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?", (table,)
        ).fetchone()[0]
        checks = {}
        for line in source_sql.splitlines():
            if "CHECK(" in line:
                name = line.strip().split(None, 1)[0].strip('"`')
                checks[name] = "CHECK(" + line.split("CHECK(", 1)[1].strip().rstrip(",")
        index_rows = db.execute(f"PRAGMA index_list({quoted(table)})").fetchall()
        fk_rows = db.execute(f"PRAGMA foreign_key_list({quoted(table)})").fetchall()
        indexed = {column[1] for column in columns if column[5]}
        for index in index_rows:
            indexed.update(columns_for_index(db, index[1]))
        indexed.update(row[3] for row in fk_rows)
        definitions = []
        for _, name, kind, required, default, _ in columns:
            sql_type = {
                "INTEGER": "BIGINT",
                "REAL": "DOUBLE",
                "BLOB": "LONGBLOB",
            }.get(kind.upper())
            if sql_type is None:
                if kind.upper() != "TEXT":
                    raise ValueError(f"unsupported SQLite type {table}.{name}: {kind}")
                sql_type = string_type(name, name in indexed)
            definition = f"  {quoted(name)} {sql_type}"
            if required:
                definition += " NOT NULL"
            if default is not None:
                # MySQL needs an expression default for TEXT/BLOB columns.
                definition += f" DEFAULT ({default})" if sql_type in ("LONGTEXT", "LONGBLOB") else f" DEFAULT {default}"
            if name in checks:
                definition += " " + checks[name]
            definitions.append(definition)
        primary = [column[1] for column in sorted(columns, key=lambda row: row[5]) if column[5]]
        if primary:
            definitions.append("  PRIMARY KEY (" + ", ".join(map(quoted, primary)) + ")")
        statements.append(
            f"CREATE TABLE {quoted(table)} (\n" + ",\n".join(definitions)
            + "\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;"
        )
        for index in index_rows:
            _, name, unique, origin, partial = index
            if origin == "pk":
                continue
            if partial:
                if name == "boards_publication_key":
                    partial = 0  # UNIQUE already permits multiple NULLs in MySQL.
                else:
                    continue  # Converted to generated-column indexes below.
            key = name if not name.startswith("sqlite_autoindex_") else f"uniq_{table}_{index[0]}"
            cols = ", ".join(map(quoted, columns_for_index(db, name)))
            prefix = "CREATE UNIQUE INDEX" if unique else "CREATE INDEX"
            indexes.append(f"{prefix} {quoted(key)} ON {quoted(table)} ({cols});")
        groups = defaultdict(list)
        for row in fk_rows:
            groups[row[0]].append(row)
        for number, rows in groups.items():
            rows.sort(key=lambda row: row[1])
            target = rows[0][2]
            source_columns = ", ".join(quoted(row[3]) for row in rows)
            target_columns = ", ".join(quoted(row[4]) for row in rows)
            deletion = rows[0][6]
            suffix = f" ON DELETE {deletion}" if deletion != "NO ACTION" else ""
            foreign_keys.append(
                f"ALTER TABLE {quoted(table)} ADD CONSTRAINT {quoted(f'fk_{table}_{number}')} "
                f"FOREIGN KEY ({source_columns}) REFERENCES {quoted(target)} ({target_columns}){suffix};"
            )
    statements.extend(indexes)
    statements.extend(
        [
            "ALTER TABLE `users` ADD COLUMN `system_account_key` VARCHAR(64) "
            "GENERATED ALWAYS AS (CASE WHEN `role`='system' THEN `account_id` END) STORED;",
            "CREATE UNIQUE INDEX `users_system_account` ON `users` (`system_account_key`);",
            "ALTER TABLE `storage_attachments` "
            "ADD COLUMN `card_image_record_key` VARCHAR(64) GENERATED ALWAYS AS "
            "(CASE WHEN `record_type`='Card' AND `name`='image' THEN `record_id` END) STORED, "
            "ADD COLUMN `card_image_blob_key` VARCHAR(64) GENERATED ALWAYS AS "
            "(CASE WHEN `record_type`='Card' AND `name`='image' THEN `blob_id` END) STORED, "
            "ADD COLUMN `user_avatar_record_key` VARCHAR(64) GENERATED ALWAYS AS "
            "(CASE WHEN `record_type`='User' AND `name`='avatar' THEN `record_id` END) STORED, "
            "ADD COLUMN `user_avatar_blob_key` VARCHAR(64) GENERATED ALWAYS AS "
            "(CASE WHEN `record_type`='User' AND `name`='avatar' THEN `blob_id` END) STORED, "
            "ADD COLUMN `embed_name_key` VARCHAR(191) GENERATED ALWAYS AS "
            "(CASE WHEN `name`='embed' THEN `name` END) STORED;",
            "CREATE UNIQUE INDEX `storage_card_image_unique` ON `storage_attachments` (`card_image_record_key`);",
            "CREATE UNIQUE INDEX `storage_card_image_blob_unique` ON `storage_attachments` (`card_image_blob_key`);",
            "CREATE UNIQUE INDEX `storage_user_avatar_unique` ON `storage_attachments` (`user_avatar_record_key`);",
            "CREATE UNIQUE INDEX `storage_user_avatar_blob_unique` ON `storage_attachments` (`user_avatar_blob_key`);",
            "CREATE UNIQUE INDEX `storage_embed_unique` ON `storage_attachments` "
            "(`record_type`, `record_id`, `embed_name_key`, `blob_id`);",
        ]
    )
    statements.extend(foreign_keys)
    return "\n".join(statements) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("sqlite_database", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    with sqlite3.connect(args.sqlite_database) as db:
        args.output.write_text(generate(db))


if __name__ == "__main__":
    main()
