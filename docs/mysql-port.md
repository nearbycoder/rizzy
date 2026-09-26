# MySQL port status

Fizzy supports MySQL. Rizzy's MySQL work is in progress on this branch. The
`mysql:` URL currently accepts only `init-db` and `seed`; starting the web
server with MySQL returns an explicit error because its queries, triggers, and
search implementation still use SQLite syntax.

`schema.mysql.sql` is a snapshot of an initialized Rizzy SQLite database. To
regenerate it after a SQLite schema change, seed a new SQLite database and run
`python3 scripts/generate_mysql_schema.py path/to/fresh.sqlite3 schema.mysql.sql`.
The generator preserves the 63 application tables, ordinary indexes, foreign
keys, 15 checks, and the partial uniqueness rules through generated columns.
It deliberately excludes FTS5 shadow tables and SQLite triggers; those need
native MySQL implementations.

The schema and commands were exercised on a disposable local MariaDB 13.0.2
instance on 2026-09-26. The Rust binary created all 63 tables through SQLx,
then seeded one account, two users, one join code, and one access token. The
generated partial unique, foreign key, and check constraints rejected invalid
inserts. The existing SQLite unit, join, and smoke checks passed after this change. MySQL
8.x compatibility still needs a direct run; MariaDB is a useful development
server but has dialect and behavior differences.

Remaining database work:

1. Replace SQLite startup migrations and `PRAGMA` queries with versioned MySQL
   migrations, including upgrades of existing MySQL databases.
2. Port 32 SQLite triggers and their observable side effects. Check generated
   columns and uniqueness against the source callbacks and MySQL 8.x.
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
