# MySQL port status

Fizzy supports MySQL. Rizzy's MySQL work is in progress on this branch. The
`mysql:` URL now starts the web server. The repeatable
`tests/mysql_core.py` flow covers account seeding, board and card creation,
columns, triage, steps, tags, account and board entropy, the closing-soon
filter, assignment and activity spikes, watch and pin controls, assigned-card
moves into private boards, saved filters,
event and mention notifications, email and push queue records, notification
settings and unsubscribe tokens, personal timezone, public board publication
and viewing, push-job draining without a VAPID key, passkey registration and
sign-in, comments, JSON and browser search, browser card and menu pages,
closing a card, and API reads. It verifies the
account entropy, user attachable and settings, access record, and board touch
side effects in the database. Most routes have not yet been exercised on
MySQL; some still use SQLite-only SQL, and this is not deployment parity.
The pinned source's `Card::Entropic` suite passed 8 tests and 19 assertions on
the same MySQL 8 server. The closing-soon MySQL query checks both account and
board periods with recent, eligible, and overdue cards; it does not cover
precise threshold boundaries or large indexed scans.
The pinned source's filters controller suite passed 2 tests and 17 assertions
on MySQL 8. Rizzy's core flow now checks repeat saves keep one filter and one
board association, closed-card results, and deletion. Other filter
combinations and browser controls still need comparison.
The pinned source's selected notification, mention, settings, and unsubscribe
suites passed 31 tests and 127 assertions on MySQL 8. Rizzy's MySQL core flow
checks one recipient's event notification, repeated unread count, a card
mention, three queued push records, one pending email bundle, settings record
timestamps and repeat no-ops, token renewal after expiry, and unsubscribe.
Delivery, larger recipient sets, revocation, and full browser parity remain.
`tests/join.py` now runs its invite flow against MySQL, including the
two-process single-slot race and recovery of a missing join code on restart.
`tests/email_change.py` runs validation, expiry, one-use confirmation, session
switch, and local sendmail checks on both databases. A MySQL trigger cannot
update a table being read by the statement that fired it, so the access-grant
paths now read eligible board or user IDs before inserting access rows. The
MySQL core flow also checks a card move that grants a private-board assignee
access, and invokes `deliver-push` to drain three queued jobs without a VAPID
key. These are selected flows; other invitation, email, push, and move cases
still need full cross-app comparison.
The pinned source's selected invite, join, and email-change model and
controller suites passed 51 tests and 232 assertions on the same MySQL 8
server.
`tests/mysql_passkeys.py` runs the passkey registration, management, signed
login, replay, invalid-signature, and deletion flow on MySQL. It also sends
two simultaneous assertions for one challenge and requires exactly one login;
MySQL challenge consumption uses a row lock and transaction in place of
SQLite's `DELETE RETURNING`. Five repeated MySQL runs passed after clearing the
dedicated test database's authentication-attempt history between runs; the
SQLite passkey flow still passes. The pinned
Fizzy MySQL passkey controller and library suites passed 19 tests and 64
assertions. This checks one synthetic authenticator and one local origin; real
browser devices, multiple credentials, and deployment origins remain.
`tests/mysql_account_cancellation.py` checks the single-tenant cancellation
guard, immediate access loss, day-29 retention, day-31 deletion of all
account-scoped MySQL rows and dependent comments, file cleanup, and survival
of another account. MySQL incineration orders account-scoped tables by foreign
key dependency and keeps foreign-key checks enabled; the cancellation row is
locked before deletion. The SQLite cancellation regression also passes. This
fixture does not establish parity for every account lifecycle workflow or
concurrent incineration workers.
`tests/mysql_session_transfer.py` now checks device transfer link creation,
revocation, QR rendering, redemption by PUT and PATCH, replay rejection,
four-hour expiry, and simultaneous redemption with exactly one winner.
MySQL redemption locks the transfer row and deletes it in the same transaction
before creating a session. The matching SQLite link flow was compared with
Fizzy in `tests/session_transfer_parity.py`, and its generation, redemption,
expiry, and race checks passed again after this change. The MySQL test checks
Rizzy only.
`tests/mysql_account_import.py` restores a populated account ZIP on MySQL. Its
default Rizzy-generated fixture checks 43 records across 23 groups and two
original file payloads. A ZIP produced by the pinned Fizzy app's MySQL test
account through `tests/source_mysql_account_export_test.rb` also restored and
re-exported: 94 records across 26 groups, including users, settings,
notifications, filters, webhook deliveries, and a card image. For both fixtures
the test checks account name, card count, re-exported record-group counts, file
bytes, and attachment targets. For the source image, it also fetches the
restored card and serves the image over HTTP with the original bytes. The
Fizzy-produced ZIP also passed the SQLite
import regression. The MySQL test checks that a cross-account
reference leaves no restored records and that a second import of the same ZIP
reports a conflict without adding cards. The import worker now uses MySQL upserts and scalar
`GREATEST` where SQLite used `ON CONFLICT` and scalar `MAX`. A new MySQL board
update trigger supplies the entropy metadata expected during restore, and the
export query quotes MySQL's reserved `key` column. Import and export now
preserve a source system user's settings record and omit generated settings
for an imported user whose archive has none. These fixtures do not cover every
source archive variant or every browser workflow after restoration.
The pinned source's selected webhook model and controller suites passed 71
tests and 334 assertions on MySQL 8 with one test worker. The separate
`tests/mysql_webhooks.py` flow checks Rizzy's webhook create, update,
activation, and deletion; a queued card event; two worker processes claiming
one delivery; private target rejection; and deactivation after the tenth
failure. The claim uses a row lock with `SKIP LOCKED` on MySQL. This verifies
single-delivery correctness for a local blocked target, not successful
internet delivery or maximum worker throughput. The MySQL claim path has not
yet been exercised on MariaDB.

`schema.mysql.sql` is a snapshot of an initialized Rizzy SQLite database. To
regenerate it after a SQLite schema change, seed a new SQLite database and run
`python3 scripts/generate_mysql_schema.py path/to/fresh.sqlite3 schema.mysql.sql`.
The generator preserves the 63 application tables, ordinary indexes, foreign
keys, 15 checks, and the partial uniqueness rules through generated columns.
It deliberately excludes FTS5 shadow tables. MySQL setup creates another 16
account-routed FULLTEXT search tables and backfills published cards and
comments. Twenty-one MySQL triggers in
`schema.mysql.triggers.sql` cover the core flow, with an idempotent backfill
for databases created before the triggers were added. Other SQLite trigger
behavior still needs native MySQL equivalents or equivalent application code.
An existing MySQL database installed the four added user settings triggers on
restart and reported all twenty triggers present at that point. A fresh database with that
twenty-trigger schema passed concurrent initialization and 80 card creations
across two processes. The first fresh run exposed an empty trigger section
header; removing it fixed installation.

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
can leave a partial database if it fails during DDL. Startup now resumes
missing tables, indexes, generated columns, and foreign keys while holding
the schema initialization lock, then checks those objects and the check count
before serving. `tests/mysql_schema_resume.py` passed on two disposable MySQL
8.4.11 databases interrupted after early tables and midway through generated
columns; both retained existing data through recovery, seeding, and a second
restart. The generated-column case started two recovery processes together.
The fresh-schema two-process `tests/mysql_cluster.py` check also
passed after this change. This repairs interrupted installation of the current
snapshot; incompatible object definitions and future schema upgrades still
need versioned migrations and may require manual repair.

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

## Preliminary MySQL throughput comparison

`bench/sustained_mixed_mysql.py` runs the pinned Fizzy source and Rizzy against
separate fresh databases on the same local Oracle MySQL 8.4.11 server. It
compares JSON response shapes and fixture content, then checks card, event, and
comment counts directly in MySQL after each run. One workload unit reads the
board, card, card list, and activity list, then creates, edits, comments on,
closes, and reads a card. The load generator and web servers use separate CPU
affinity sets; Fizzy uses four Puma workers and Rizzy one multithreaded process
on the four server CPUs. The two targets run sequentially, and the second
round reverses their order. MySQL itself is shared and is not included in the
application CPU and memory figures.

The September 26, 2026 runs used 100 initial cards and 15 seconds per target.
All four rounds had matching samples, correct stored counts, and zero HTTP
errors:

| Clients | Round order | Fizzy units/s | Rizzy units/s | Rizzy/Fizzy | Fizzy p95 unit | Rizzy p95 unit |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 8 | Fizzy, Rizzy | 6.74 | 71.09 | 10.55× | 1647 ms | 223 ms |
| 8 | Rizzy, Fizzy | 25.27 | 174.30 | 6.90× | 439 ms | 69 ms |
| 16 | Fizzy, Rizzy | 26.11 | 164.88 | 6.31× | 743 ms | 267 ms |
| 16 | Rizzy, Fizzy | 24.89 | 111.78 | 4.49× | 827 ms | 338 ms |

Raw measurements and revisions are in
`bench/results/mysql_mixed_c8_2026-09-26.json` and
`bench/results/mysql_mixed_c16_2026-09-26.json`. The spread is wide, including
within one target, so these are preliminary local measurements for this one
API workload, not a claim of maximum capacity or full application speed. An
earlier two-client probe saw one Fizzy HTTP 500 on a card update during its
first load phase; Rizzy had not begun its load phase. A later 10-second
two-client run and these four rounds did not reproduce it. That transient
source failure was not diagnosed from that run. The later 500-card failures
below identify a MySQL search-index deadlock at the same endpoint. Larger
datasets, a concurrency sweep, multiple application processes, full background
jobs, and representative browser workflows remain necessary before claiming
greater scale.
After the invite, email, and access changes, a clean-build eight-client
two-round run at commit `73d0e50` completed with matching samples, verified
stored counts, and zero HTTP errors. Fizzy completed 24.37 and 23.43 units/s;
Rizzy completed 169.39 and 62.54 units/s. The corresponding ratios were 6.95×
and 2.67×. The raw artifact is
`bench/results/mysql_mixed_after_join_2026-09-26.json`. The larger spread in
Rizzy throughput needs investigation before drawing a capacity conclusion.

With 500 initial cards, one 20-second eight-client paired round completed:
Fizzy handled 23.25 units/s and Rizzy 156.16 units/s, with matching preflight
samples, zero HTTP errors, and verified stored counts. In the reverse-order
round, Rizzy completed 149.93 units/s with zero errors and verified counts,
but Fizzy returned HTTP 500 during `PATCH /cards/818`, so that round has no
valid paired throughput ratio. A separate 16-client run stopped on Fizzy's
first load phase before Rizzy ran. Both failures were MySQL error 1213
deadlocks in the pinned source's ActiveSearch MySQL shard upsert, reported as
`ActiveSearch::AdapterError` from `CardsController#update`. The benchmark
intentionally stops on the first HTTP error and does not count partial
workflows as completed units. The two failure records are
`bench/results/mysql_mixed_500cards_c8_partial_2026-09-26.json` and
`bench/results/mysql_mixed_500cards_c16_failed_2026-09-26.json`. A repeated,
error-free concurrency sweep and checks with multiple app and database hosts
are still needed to establish a greater-scale claim.
