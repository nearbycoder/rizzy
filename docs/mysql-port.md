# MySQL port status

Fizzy supports MySQL. Rizzy's MySQL work is in progress on this branch. The
`mysql:` URL now starts the web server. The repeatable
`tests/mysql_core.py` flow covers account seeding, board and card creation,
comments, closing a card, and API reads. It verifies the account entropy,
user attachable and settings, access record, and board touch side effects in
the database. Most routes have not yet been exercised on MySQL; some still use
SQLite-only SQL, and this is not deployment parity.

`schema.mysql.sql` is a snapshot of an initialized Rizzy SQLite database. To
regenerate it after a SQLite schema change, seed a new SQLite database and run
`python3 scripts/generate_mysql_schema.py path/to/fresh.sqlite3 schema.mysql.sql`.
The generator preserves the 63 application tables, ordinary indexes, foreign
keys, 15 checks, and the partial uniqueness rules through generated columns.
It deliberately excludes FTS5 shadow tables. Sixteen MySQL triggers in
`schema.mysql.triggers.sql` cover the core flow, with an idempotent backfill
for databases created before the triggers were added. Other SQLite trigger
behavior still needs native MySQL equivalents or equivalent application code.

The schema and commands were exercised on a disposable local MariaDB 13.0.2
instance on 2026-09-26. The Rust binary created all 63 tables through SQLx,
then seeded one account, two users, one join code, and one access token. The
generated partial unique, foreign key, and check constraints rejected invalid
inserts. A fresh database and an existing database both passed the MySQL core
smoke, and the SQLite unit, join, smoke, and account export checks passed.
SQLx 0.8.6's `Any` adapter treats MariaDB's `LONGTEXT` as binary data; the
local `vendor/sqlx-mysql` patch decodes that server type as UTF-8 for Rizzy's
text-only schema. This patch and the schema need a direct MySQL 8.x run;
MariaDB has dialect and behavior differences.

Remaining database work:

1. Replace the remaining SQLite startup migrations and `PRAGMA` queries with
   versioned MySQL migrations, including upgrades beyond the first backfill.
2. Port the remaining SQLite trigger behavior and observable side effects.
   Check generated columns and uniqueness against the source callbacks and
   MySQL 8.x.
3. Port runtime statements using `INSERT OR IGNORE`, `ON CONFLICT`,
   `BEGIN IMMEDIATE`, SQLite date functions, and other SQLite-only syntax.
4. Implement MySQL full-text search with the same user-visible results as the
   pinned Fizzy source, and adapt cache validation that uses `PRAGMA data_version`.
5. Run the full route, account-transfer, and browser parity suites on MySQL.
   Repeat the comparable load tests with both apps on the same MySQL server,
   then test multiple application processes and larger datasets.

The database-independent SQLx row/pool layer is a foundation for this work.
A short native-SQLite versus `AnyPool` comparison showed lower throughput in
one of two `AnyPool` runs, so the pool choice should be measured again on
longer workloads before merging into the primary checkout.
