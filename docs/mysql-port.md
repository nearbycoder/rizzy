# MySQL port status

Fizzy supports MySQL. Rizzy's MySQL work is in progress on this branch. The
`mysql:` URL now starts the web server. The repeatable
`tests/mysql_core.py` flow covers account seeding, board and card creation,
columns, triage, steps, tags, account entropy, public board publication and
viewing, comments, JSON and browser search, closing a card, and API reads. It
verifies the account entropy, user attachable and settings, access record,
and board touch side effects in
the database. Most routes have not yet been exercised on MySQL; some still use
SQLite-only SQL, and this is not deployment parity.

`schema.mysql.sql` is a snapshot of an initialized Rizzy SQLite database. To
regenerate it after a SQLite schema change, seed a new SQLite database and run
`python3 scripts/generate_mysql_schema.py path/to/fresh.sqlite3 schema.mysql.sql`.
The generator preserves the 63 application tables, ordinary indexes, foreign
keys, 15 checks, and the partial uniqueness rules through generated columns.
It deliberately excludes FTS5 shadow tables. MySQL setup creates another 16
account-routed FULLTEXT search tables and backfills published cards and
comments. Sixteen MySQL triggers in
`schema.mysql.triggers.sql` cover the core flow, with an idempotent backfill
for databases created before the triggers were added. Other SQLite trigger
behavior still needs native MySQL equivalents or equivalent application code.

The schema and commands were exercised on a disposable local MariaDB 13.0.2
instance on 2026-09-26. The Rust binary created all 63 tables through SQLx,
then seeded one account, two users, one join code, and one access token. The
generated partial unique, foreign key, and check constraints rejected invalid
inserts. A fresh database and an existing database both passed the MySQL core
smoke, and the SQLite unit, join, smoke, and account export checks passed.
`tests/mysql_cluster.py` starts two initializers and two web processes against
one database, then verifies 80 concurrent card creations get unique numbers
and the correct paginated total. The first concurrent initialization probe
found a table-creation race; a MySQL advisory lock now serializes setup, and
the fresh-database rerun passed. This checks correctness at small scale, not
a maximum capacity or a faster-than-Fizzy result.
Card and draft numbering now update the account counter and read it back in
the same transaction on MySQL, avoiding the SQLite-specific `UPDATE RETURNING`
statement. The core and two-process checks passed again after this change.
SQLx 0.8.6's `Any` adapter treats MariaDB's `LONGTEXT` as binary data; the
local `vendor/sqlx-mysql` patch decodes that server type as UTF-8 for Rizzy's
text-only schema.

The same tests passed on a fresh Oracle MySQL 8.4.11 database: 63 core tables,
16 search tables, 16
triggers, 114 foreign-key column mappings, the core API workflow, and 80
concurrent card creations through two processes. The first fresh MySQL schema
attempt exposed a MySQL restriction: a foreign key with `ON DELETE CASCADE`
cannot use the base column of a *stored* generated column. The five generated
uniqueness columns on `storage_attachments` are now virtual; the indexes and
cascading foreign keys install on both MySQL 8.4.11 and MariaDB 13.0.2. See
the [MySQL foreign-key restrictions](https://dev.mysql.com/doc/refman/8.4/en/create-table-foreign-keys.html).
The disposable MySQL server had binary logging enabled, so trigger installation
initially failed with error 1419. The test server was configured with
`log_bin_trust_function_creators=1`; a deployment with binary logging needs
that setting or the necessary trigger-creation privilege. Schema installation
currently leaves a partial database if it fails during DDL. Startup now checks
the expected tables, indexes, foreign keys, generated columns, and check count
and rejects an incomplete schema instead of serving it. A partial database
still needs manual repair or a fresh database; setup must be made resumable
before production use.

The pinned Fizzy MySQL search controller and sharded adapter suites passed
37 tests and 146 assertions on the same Oracle MySQL 8.4.11 server.
`tests/mysql_search_parity.py` compares the two live apps on title,
description, and comment hits; English stemming; card text filters; browser
result titles, highlighted words, and 20-word excerpts; edits; and deletions.
Its selected JSON and browser results matched. A fresh two-process Rizzy run
created 80 cards and 80 search records; an existing database backfilled 63
records for 63 published cards and comments. This does not cover the complete
query grammar, every browser highlight and snippet case, Unicode indexing, or
search throughput.

Remaining database work:

1. Replace the remaining SQLite startup migrations and `PRAGMA` queries with
   versioned MySQL migrations, including upgrades beyond the first backfill.
2. Port the remaining SQLite trigger behavior and observable side effects.
   Check generated columns and uniqueness against the source callbacks.
3. Port runtime statements using `INSERT OR IGNORE`, `ON CONFLICT`,
   `BEGIN IMMEDIATE`, SQLite date functions, and other SQLite-only syntax.
4. Complete MySQL search parity for query grammar, rendering, Unicode, and
   large-scale indexing; adapt cache validation that uses `PRAGMA data_version`.
5. Run the full route, account-transfer, and browser parity suites on MySQL.
   Repeat the comparable load tests with both apps on the same MySQL server,
   then test multiple application processes and larger datasets.

The database-independent SQLx row/pool layer is a foundation for this work.
A short native-SQLite versus `AnyPool` comparison showed lower throughput in
one of two `AnyPool` runs, so the pool choice should be measured again on
longer workloads before merging into the primary checkout.
