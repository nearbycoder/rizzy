# MySQL port status

Fizzy supports MySQL. Rizzy's MySQL work is in progress on this branch. The
`mysql:` URL now starts the web server. The repeatable
`tests/mysql_core.py` flow covers account seeding, board and card creation,
columns, triage, steps, tags, account and board entropy, the closing-soon
filter, assignment and activity spikes, watch and pin controls, assigned-card
moves into private boards, saved filters,
event and mention notifications, email and push queue records, notification
settings and unsubscribe tokens, personal timezone, local creation and closure
date filters around Los Angeles midnight, public board publication and viewing,
push-job draining without a VAPID key, passkey registration and
sign-in, comments, card and comment reaction timestamp side effects, JSON and browser search, browser card and menu pages,
closing a card, and API reads. It verifies the
account entropy, user attachable and settings, access record, and board touch
side effects in the database. Most routes have not yet been exercised on
MySQL; some still use SQLite-only SQL, and this is not deployment parity.
Account entropy's first and repeated saves now touch cards while preserving
board, column, and card activity times. A migration replaces the older MySQL
account trigger on existing databases; `tests/mysql_account_entropy_trigger_migration.py`
checks that replacement.
The core flow also checks that a changed comment edit touches its card and
board, an identical edit leaves timestamps unchanged, and comment deletion
touches the card and board without advancing card activity time.
`tests/mysql_auto_postpone.py` checks the due-card sweep with two concurrent
command processes. All 205 ordinary overdue cards and a flag-only overdue card
gain one system-owned Not Now record, event, and comment; a recent card stays
active and a card with an existing Not Now record stays postponed even when its
old flag says otherwise. The two processes create only one set of side effects
across the 100-card batch boundary. Both query plans use `cards_board_active`
and an indexed Not Now lookup. The hourly scheduler's database claim and
larger MySQL account sweeps still need load testing.
`tests/mysql_unused_tag_cleanup.py` checks the daily unused-tag job's manual
command on MySQL 8.4.11. Two concurrent commands deleted 209 unused tags across
the 100-tag batch boundary, retained a used tag, updated or removed affected
saved filters, and left no work for a repeat command. The timed schedule and a
tag insertion racing with cleanup remain unverified.
It checks a named-column card JSON ETag, a conditional 304 response, and
invalidation after a step is added to that column's card.
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
assertions. Fresh paired `tests/mysql_pair_selfhost.py passkeys-flow` runs now
exercise registration, naming, signed login, replay, bad signatures, and
deletion on both apps through account-scoped management routes. The paired
`passkeys-html` suite checks those route links, unscoped redirect, and
missing-passkey responses. This checks one synthetic authenticator and one
local origin; real browser devices, multiple credentials, and deployment
origins remain.
`tests/mysql_draft_cluster.py` runs the existing browser draft parity flow on
MySQL, then creates a draft on one Rizzy process, edits it on a second, publishes
it on the first, and verifies the card and publication event from the second.
It also publishes 32 drafts concurrently across both processes on distinct
boards and verifies each published card. Four fresh-database runs passed. The
pinned Fizzy draft and publish controller suites passed 5 tests and 36
assertions on SQLite. This verifies
cross-process state sharing and distinct-board concurrent publication on the
tested MySQL setup. The same-board Rust cluster race is checked separately
below; paired source behavior for concurrent publication and the complete
browser interface remain open.
`tests/mysql_pair_selfhost.py draft` now boots disposable Fizzy and Rizzy MySQL
apps and matches the 11-summary browser draft workflow, including magic-link
sign-in, draft resumption, publication, search and activity side effects, and
add-another redirects. The same fixture still passes against both SQLite apps.
This paired run checks one owner and board per app, not concurrent publication
or full browser presentation.
`tests/mysql_pair_selfhost.py card-image` and `embeds` also pass on disposable
Fizzy and Rizzy MySQL databases. They compare direct uploads, image assignment
and removal, blob access, byte retrieval, preview content types, image and file
embeds, and normalized rendered Action Text. The image test checks that removal
changes card and board update times but preserves card last-activity time in
both databases. The embed comparison skips its SQLite-only direct SQL and
filesystem cleanup assertions on MySQL; selected Rust-only MySQL cleanup is
covered by the storage materialization test below.
`tests/mysql_pair_selfhost.py card-controls`, `card-board`, and `card-edit`
pass on fresh paired MySQL databases. They compare pin and watch HTML frames,
ETags, Turbo streams and JSON toggles; the board picker and browser/JSON moves;
and the card edit form, save, and reload. The fixtures check selected behavior
only; exact presentation, all permissions, and concurrent edits remain open.
Fresh paired MySQL `assignment` and `reaction` runs passed picker frames,
cache responses, Turbo changes, self-assignment, card and comment reaction
forms, and selected JSON mutations. The `column-turbo` run passed named-column
form creation, position swaps, rename, deletion redirect, and unsupported
format effects. It compares stream action order after excluding generated
column IDs; each app's fixture still checks its stream targets. The SQLite
column Turbo fixture passed after the harness refactor. Other multiuser and
concurrent column operations remain to be compared.
Fresh paired `card-previews` and `account-card` runs matched pinned Fizzy's
preview pagination after card insertion and JSON/browser account card creation.
`tests/mysql_rich_text_backfill.py` now verifies a one-time MySQL startup
backfill for older board, card, and comment text lacking Action Text record
identities; a second initialization keeps exactly those three rows. The
backfill uses set-based inserts and a schema-initialization marker. The paired
MySQL rich-text embed flow passed after this migration change. Other upgrade
states and Action Text transitions remain open.
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
`tests/mysql_delete_cleanup.py` checks API deletion of comments, cards, and
boards against direct MySQL records. It covers Action Text record rows, card
and comment mentions and reactions, notifications sourced by deleted events
or mentions, and a board event whose card survives on another board. The
first run showed an orphaned comment rich-text record. Deletion now removes
these polymorphic dependents in the same transaction before foreign-key
cascades remove the owning rows. The matching SQLite rich-text lifecycle and
the MySQL core flow passed after this change. The pinned source uses dependent
model associations for these records; other destruction callbacks and large
board deletion costs still need comparison.
The notification source lookup now has a composite index in both schemas,
matching Fizzy's source index. The MySQL index uses prefixes covering current
Event/Mention kinds and UUID-style IDs, so startup could install it on existing
databases where those columns are `LONGTEXT`. On that database, `EXPLAIN` chose
`notifications_source` for an equality lookup. The regenerated fresh schema
and interrupted-schema recovery test also passed.
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

`tests/mysql_webhook_cleanup.py` passed on a disposable MySQL 8.4.11 database:
503 eight-day-old delivery rows were removed in 500-row batches while three
recent rows remained, `EXPLAIN` chose the new `(created_at,id)` stale index,
and a second initialization retained that index. The matching disposable
Fizzy/Rizzy SQLite test passed. The quarter-hour worker uses a database claim
to avoid duplicate scheduled cleanup across app processes; its live timing
and contention with delivery workers have not been load tested.

`schema.mysql.sql` is a snapshot of an initialized Rizzy SQLite database. To
regenerate it after a SQLite schema change, seed a new SQLite database and run
`python3 scripts/generate_mysql_schema.py path/to/fresh.sqlite3 schema.mysql.sql`.
Storage entry request IDs use `VARCHAR(255)` to match the source's request ID
limit. Startup widens that column on databases created from the older 64-character
snapshot.
`tests/mysql_schema_snapshot.py` now regenerates the DDL from a fresh SQLite
database and compares it byte for byte. The snapshot includes the board
foreign key on `card_bulk_touches`. `tests/mysql_schema_upgrade.py` passed on
a disposable MySQL 8.4.11 server after shrinking the request ID column to 64
characters and dropping that foreign key; restart restored both, preserved the
account, and accepted a 255-character storage entry ID.
The generator preserves the current 67 application tables, ordinary indexes,
foreign keys, 15 checks, and the partial uniqueness rules through generated
columns. It deliberately excludes FTS5 shadow tables. MySQL setup creates
another 16 account-routed FULLTEXT search tables and backfills published cards
and comments.
Webhook delinquency state now lives in its own account-scoped table. Startup
backfills existing webhook counters into tracker rows, and new webhooks create
the row transactionally. Checks for SQLite upgrade, source versus Rust failure
and deactivation timestamps, and export/import of a zero-failure tracker pass.
The pinned Fizzy import probe restored that tracker from a Rizzy archive.
On a disposable MySQL 8.4.11 server, `tests/mysql_schema_resume.py` recovered
two partial schemas with 67 application tables and 16 search shards;
`tests/mysql_webhooks.py` passed two-worker blocked delivery;
`tests/mysql_account_import.py` restored the tracker; and
`tests/mysql_webhook_tracker_migration.py` backfilled a legacy nine-failure
webhook and survived a second restart. These are single-host local checks.
Earlier Rizzy builds advanced the webhook's own update time on each failure;
the original pre-failure value cannot be reconstructed during backfill.
Twenty-four MySQL triggers in
`schema.mysql.triggers.sql` cover the core flow, with an idempotent backfill
for databases created before the triggers were added. Other SQLite trigger
behavior still needs native MySQL equivalents or equivalent application code.
An existing MySQL database installed the four added user settings triggers on
restart and reported all twenty triggers present at that point. A fresh database with that
twenty-trigger schema passed concurrent initialization and 80 card creations
across two processes. The first fresh run exposed an empty trigger section
header; removing it fixed installation.

Card insert and update triggers now advance a named column's timestamp when
its cards change or move, matching the checked Fizzy callbacks. A changed
column name also touches its cards and board; repeating the same name is a
no-op. The MySQL core flow checks triage and rename timestamps, and both
interrupted-schema recovery cases passed with all triggers installed. In the
four-process cluster test, a move/title race exposed a stale board snapshot
in card edits. Card edits now lock and recheck the card's current board before
updating it. Three fresh-database cluster flows passed after the fix, each including 200
move/title pairs and opposite-direction moves. This checks those races on one
MySQL host, not all concurrent card writes or multi-host deployment.

Column creation now locks its board while allocating the next position and
touches the board. Deletion locks the board, clears the deleted column from
its cards, and touches all board cards without changing their activity times.
A transaction-scoped `card_bulk_touches` row suppresses per-card board and
column trigger updates during the bulk touch, then is removed before commit.
The core flow checks these timestamp effects; an existing-schema recovery
test recreates the guard table and replaces old MySQL triggers. The four-process
cluster flow also creates 20 columns on
one board concurrently and checks that their positions are exactly 1 through
20. Column position swaps now take the same database board lock, and the
cluster flow concurrently swaps ten disjoint pairs right and then left across
four processes. Column updates now read the current row under that lock; the
cluster test checks 30 simultaneous name/color edit pairs on one column and
retains both fields after every pair. A matched SQLite benchmark found Rizzy faster on a 1,000-card
delete but slower on 10,000- and 30,000-card deletes. MySQL large-board
delete throughput and cross-host behavior remain unmeasured.

The schema and commands were exercised on a disposable local MariaDB 13.0.2
instance on 2026-09-26. The Rust binary created all 63 tables present at that
revision through SQLx,
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

The same tests passed on a fresh Oracle MySQL 8.4.11 database at that revision: 63 core tables,
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
Its selected JSON and browser results matched. `tests/mysql_pair_selfhost.py search`
now creates disposable Fizzy and Rizzy databases, boots both apps, and compares
86 JSON query cases and 10 rendered search cases, including accented text,
punctuation, quotes, shared card/comment title highlights, and last-activity
ordering. A fresh two-process Rizzy run created 80 cards and 80 search records;
an existing database backfilled 63 records for 63 published cards and comments.
These fixtures do not cover the complete query grammar, every browser snippet,
Unicode indexing, or search throughput.

`tests/mysql_pair_selfhost.py core` now runs the existing 142-comparison JSON
workflow against fresh Fizzy and Rizzy MySQL databases. Its first run matched
all comparisons, including account settings, boards, cards, search, filters,
comments, activities, notifications, and webhooks. This checks one workflow
with one account per app; it does not cover every registered route or browser
interaction.

`tests/mysql_pair_selfhost.py board-filter` runs the signed-in board-filter
browser fixture on both MySQL apps. It initially found a 500 from card-preview
step metadata: MySQL returned `SUM(completed)` as a decimal that SQLx `Any`
could not map. Rizzy now casts that aggregate to a signed integer. The fresh
paired MySQL run and a paired SQLite regression passed, including a two-step
card with one completed step and the filtered second-page preview fragment.
Other browser routes and full HTML equivalence remain unverified on MySQL.

The same disposable paired harness now passes `all-cards`, `named-column`,
`column-cache`, and `column-show-cache` against Fizzy on MySQL. The all-cards
fixture compares two-page previews, state and people filters, board-choice
ordering, and JSON pagination; the named-column fixtures compare empty and
two-page HTML plus conditional JSON and HTML validators after edits. The
source MySQL adapter matches more cards for the fixture's multiword term than
the source SQLite adapter, and Rizzy's MySQL results match that source set.
The existing SQLite fixtures also passed after reuse in the MySQL harness.
These are selected paths, not full browser or cache parity.

Five more disposable paired MySQL workflows now pass: `board-views` checks
private and public special/named columns and pagination; `board-settings`
checks browser create, edit, entropy choices, and delete; `webhooks-html`
checks browser validation and CRUD; `step-read` checks JSON and browser step
detail/edit; and `public-page` checks anonymous board/card HTML, rich text,
steps, image bytes, social metadata, and unpublishing. The public card fixture
checks each step's completion state by content: Fizzy's MySQL association has
no explicit step ordering, and its two newly created steps can appear in either
order. These comparisons expand route coverage but do not establish full
MySQL browser, deployment, or HTML parity.

Remaining database work:

The storage entry and total tables now have a live MySQL 8.4.11 regression in
`tests/mysql_storage_materialization.py`. A fresh database checks card image,
card embed, and comment embed recording; Card and Comment transfer entries;
two concurrent materialization commands and no-op replay; a deleted ledger
entry repaired by reconciliation; account and board totals; and attachment
file cleanup. Eight clients also attach 16 card embeds concurrently on one
board before two workers materialize its total. The first run exposed MySQL's
decimal result type for `SUM(byte_size)` in the card-transfer path, which SQLx
`Any` could not map; Rizzy now casts that sum to a signed integer on MySQL.
The existing SQLite source-versus-Rust ledger comparison and SQLite
materialization suite still pass. Larger attachment datasets, sustained
concurrent writes, scheduled-worker behavior, and MySQL upgrade paths remain
unverified.

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
affinity sets; the initial comparisons used four Fizzy Puma workers and one
multithreaded Rizzy process on the same four server CPUs. The two targets run
sequentially, and the second round reverses their order. MySQL itself is
shared and is not included in the application CPU and memory figures.

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
`ActiveSearch::AdapterError` from `CardsController#update`. Those runs used a
benchmark that stopped on the first HTTP error and did not count partial
workflows as completed units. The two failure records are
`bench/results/mysql_mixed_500cards_c8_partial_2026-09-26.json` and
`bench/results/mysql_mixed_500cards_c16_failed_2026-09-26.json`. A repeated,
error-free concurrency sweep and checks with multiple app and database hosts
are still needed to establish a greater-scale claim.

The MySQL mixed benchmark now finishes the other target's load when one target
returns an HTTP error. It writes an artifact with the failed target's observed
database counts, error, and log excerpts, marks the round unsuitable for a
paired throughput ratio, and exits nonzero after writing. A 100-card,
eight-client probe reached a Fizzy MySQL search-index deadlock after 0.5
seconds and seven completed units; Rizzy then completed its ten-second phase
at 198.65 units/s with zero errors and verified counts. That result is in
`bench/results/mysql_mixed_100cards_c8_failure_2026-09-26.json` and is
reliability evidence, not a speed ratio. A separate 500-card, 32-client,
20-second pair completed with no errors or count mismatches: Fizzy reached
26.63 units/s and Rizzy 162.21 units/s, a 6.09× ratio for that round. Its
artifact is `bench/results/mysql_mixed_500cards_c32_20s_2026-09-26.json`.
Both runs used the pinned Fizzy source and the release binary hash recorded
in each artifact. The source deadlock is intermittent, so repeated rounds are
needed before making a stable throughput or maximum-scale claim.

At 500 initial cards and 64 clients, two reversed-order 20-second rounds from
clean commit `89afdd6` completed Rizzy's full load at 168.92 and 146.98
units/s, with zero HTTP errors and verified card, event, and comment counts.
Fizzy returned one `PATCH /cards` HTTP 500 in each round after 9.54 and 9.34
seconds. Both source logs report MySQL error 1213 in the ActiveSearch shard
upsert. The source rates before failure, 26.72 and 27.31 units/s, are **not**
paired comparisons with the full Rizzy runs; partially completed source
workflows also make their stored counts differ from a completed-unit formula.
The raw result is
`bench/results/mysql_mixed_500cards_c64_20s_2026-09-26.json`. This shows
Rizzy sustained this particular local workload at 64 clients while the
source encountered an error under the same setup. It does not establish a
maximum client count, multi-host scale, or whole-application speed.

`bench/sustained_mixed_mysql.py --rizzy-processes 4` now routes workers across
four independent Rust servers that share one MySQL database and the same four
server CPUs as Fizzy's four Puma workers. The first eight-client run exposed
MySQL error 1213 in Rizzy: card and event writes held shared foreign-key locks
on the board, then the card update trigger tried to upgrade that board lock to
exclusive. The affected card creation, update, closure, event, and reading
transactions now lock the parent board before their child writes. A ten-second
eight-client rerun completed 2,161 Rizzy workflows with zero errors and verified
counts, distributed 538–542 across the four processes; Fizzy completed 261.

In the [four-process 500-card run](../bench/results/mysql_mixed_500cards_c32_4process_20s_2026-09-26.json),
Rizzy completed 3,866 and 3,908 workflows in two reversed-order 20-second
rounds at 32 clients, or 192.03 and 194.50 workflows/s. Both had zero HTTP
errors, verified card/event/comment counts, and near-even work distribution
across all four Rust processes. Fizzy hit its previously observed ActiveSearch
deadlock in the first round. Its second round completed 505 workflows at
24.32/s without error, giving one valid paired ratio of 8.00×. All four app
processes shared the same four assigned CPUs; MySQL was not CPU pinned and was
excluded from the application process resource figures.
The run tests one board and one API workflow on one host. Other routes and
boards, retries for unrelated deadlocks, and multi-host deployment still need
verification before claiming more general scale.

The [direct one-versus-four Rizzy comparison](../bench/results/rizzy_mysql_1v4_500cards_c32_20s_2026-09-26.json)
uses the same binary, MySQL server, four assigned app CPUs, 500-card fixture,
32 clients, and two reversed-order 20-second rounds. On this single-board
workload, one process completed 164.70 and 165.74 workflows/s, while four
completed 184.74 and 191.65 workflows/s: a 12.2–15.6% gain with no HTTP
errors and verified database counts. The four-process distribution was within
seven completed workflows among workers in each round. Summed app-process PSS
was about 46 MiB at the end of each four-process phase versus 13–15 MiB for
one process. The board-touch trigger serializes writes to this board, so this
fixture may understate scale across independent boards.

The [four-board direct comparison](../bench/results/rizzy_mysql_1v4_4boards_500cards_c32_20s_2026-09-26.json)
kept 500 initial cards, 32 clients, the same MySQL server and four app CPUs,
and two reversed-order 20-second rounds. Work was distributed evenly across
four boards and, in the cluster phases, four app processes. One process handled
199.99 and 198.12 workflows/s; four handled 285.37 and 279.88, gains of 42.7%
and 41.3%. All phases had zero HTTP errors and exact card, event, and comment
counts. End-of-phase app PSS was 13–15 MiB for one process and 47 MiB for four.
This supports higher throughput through application-process scaling on this
local, four-board API workload. The shared account card counter and database
remain contention points; other routes, larger datasets, durability under
failure, and multiple hosts still need direct tests.

The [1,000-card four-board direct comparison](../bench/results/mysql-cluster-1000-4boards-c32-30s-2026-09-26.json)
extends that workload to two reversed-order 30-second rounds at 32 clients on
the same four app CPUs. One process completed 192.62 and 190.88 workflows/s;
four processes completed 255.00 and 241.17, gains of 32.4% and 26.3%. All
four phases had zero HTTP errors and the expected card, event, and comment
counts. The measured gain is smaller than on the 500-card fixture and remains
one-host, four-board evidence.

The [1,000-card Fizzy/Rizzy MySQL comparison](../bench/results/mysql-mixed-1000-c32-30s-2026-09-26.json)
used four Fizzy Puma workers or one Rizzy process on those four CPUs. In its
clean reversed-order pair, Rizzy completed 135.26 workflows/s versus Fizzy's
21.11 with zero HTTP errors and verified counts. The other Fizzy phase failed
one PATCH with an ActiveSearch shard deadlock, so the two-round comparison is
not fully clean and its 27.19 workflows/s cannot be used as a paired
throughput result. The Rust binary hash and source revisions are in the
artifacts; the source and Rust apps still have incomplete feature parity.
A [16-client repeat](../bench/results/mysql-mixed-1000-c16-30s-2026-09-26.json)
likewise produced one clean pair, 138.86 Rizzy versus 24.37 Fizzy
workflows/s, and one Fizzy ActiveSearch deadlock phase. Lower concurrency did
not remove this source failure from the 1,000-card fixture.
An [eight-client repeat](../bench/results/mysql-mixed-1000-c8-30s-2026-09-26.json)
had the same outcome: one clean pair at 139.10 versus 24.08 workflows/s and
one Fizzy ActiveSearch deadlock. A clean two-round comparison of this larger
fixture remains unverified.

The paired MySQL harness now supports `--boards 4` and verifies aggregate
card, event, and comment counts across the boards. The [current 16-client
comparison](../bench/results/mysql-mixed-1000-4boards-4process-c16-30s-2026-09-27.json)
used 1,000 initial cards, four Fizzy workers or four Rizzy processes, four app
CPUs, the same MySQL server, and two reversed-order 30-second rounds. All four
phases passed sampled-response and final-count gates with zero HTTP errors.
Rizzy handled 251.94 and 225.40 workflow units/s versus Fizzy's 23.07 and
24.49. The [32-client extension](../bench/results/mysql-mixed-1000-4boards-4process-c32-30s-2026-09-27.json)
had one clean pair at 252.31 versus 26.87; Rizzy's other phase completed at
251.47 without errors, but Fizzy's card PATCH returned 500 from an
ActiveSearch MySQL deadlock. The failed phase cannot support a throughput
ratio. These results measure one local mixed API workflow, not full product
parity, multi-host scaling, or maximum sustainable capacity.

The [current one-versus-four Rizzy MySQL comparison](../bench/results/mysql-rizzy-1v4-1000-4boards-c32-30s-2026-09-27.json)
held the same release binary, 1,000-card four-board fixture, four app CPUs,
32 clients, and two reversed-order 30-second rounds. Four app processes
completed 250.72 and 248.86 workflow units/s versus 172.28 and 174.04 for
one, gains of 45.5% and 43.0%, with zero HTTP errors and exact record counts.
Load was near even across boards and processes. End-of-phase app PSS rose
from 14–15 MiB to about 50 MiB, and average app CPU use from 0.70–0.71 to
1.63–1.65 cores. The gain is evidence of one-host process scaling on this
workload, not a cross-host or maximum-capacity result.

The [matched-fixture MySQL JSON read comparison](../bench/results/mysql-read-matched-fixtures-1000-8-32-128-15s-2026-09-26.json)
avoids that write-path failure. Fresh 1,000-card databases, the same four
available app CPUs, two reversed-order rounds, and 15-second phases at 8,
32, and 128 clients yielded zero HTTP errors and unchanged record counts.
The harness checked the full decoded sampled JSON values, including cross-route
ID relationships, after masking generated IDs, timestamps, and server addresses;
it also checked pagination on board, card, scoped-card, and scoped-activity GET
routes. The account number and owner details matched across targets. Rizzy
served 3,251–4,117 requests/s versus Fizzy's 320–350, or 10.2–12.1× per
paired level. Its serialized bodies were about 1% smaller. Unsampled responses,
other routes, writes, and multi-host capacity remain unproven.

The [four-process repeat](../bench/results/mysql-read-matched-fixtures-4process-1000-8-32-128-15s-2026-09-26.json) routed each JSON path evenly over four Rizzy processes and compared them with four Fizzy Puma workers on the same four app CPUs. A [one-process control](../bench/results/mysql-read-matched-fixtures-1process-1000-8-32-128-15s-2026-09-26.json) used the identical Rust binary. Both fresh-fixture runs completed two reversed-order 15-second rounds at 8, 32, and 128 clients with equal sampled decoded JSON, zero HTTP errors, and unchanged card/event/comment counts. Four-process Rizzy served 3,372–4,605 requests/s versus Fizzy's 290–312; compared with one Rizzy process, its mean throughput was 2%, 24%, and 30% higher at the three client levels. At 128 clients its p95 latency rose from 45–54 ms for one process to 70–85 ms for four, with greater CPU and memory use. These measurements support local read throughput scaling on the four routes, not cross-host or whole-application scale. Runtime and connection-pool configurations still differ between targets.

The write-path audit found routes that updated cards without locking their
parent board first. `tests/mysql_cluster.py` now checks 400 distinct cards
through step create, update, and delete phases and then runs 200 same-card
pairs per phase: step/title, unwatch/title, triage/title, and reaction/title.
Each pair uses different app processes; the 64-client phases share one board
across four processes. Before the fixes, each new mixed phase produced HTTP
500 database errors. Step CRUD, watches, triage/return-to-stream, and reaction
creation now lock the board before child writes; reaction creation also commits
its insert and card touch together. The complete cluster test passed four fresh
account runs after these changes, and the MySQL core flow plus SQLite step and
watch regressions passed. This covers those specific races, not every lock
cycle. That initial check had not exercised draft publication or card moves.

Reaction removal now uses the same board-first card lock as creation and
commits the delete with the card, board, and optional comment timestamp touches.
The MySQL core flow verified Fizzy's four creation/removal timestamp patterns,
and one fresh four-process cluster run passed 200 reaction deletions racing
title edits on the same cards. This adds a specific delete-path check; other
concurrent reaction and comment operations remain to be audited.

The watch mutation now uses the card-and-board lock helper too. A live SQLite
comparison checked Fizzy's watch on/off and repeat no-op timestamp effects.
The expanded cluster test ran 200 watch mutations concurrently with moves of
the same cards across four processes; with `RIZZY_TEST_MYSQL_CLIENT` configured,
it checked representative persisted watch states after the race. Other
cross-board write paths still need the same stale-board audit.

Card read and unread mutations now also recheck the card's current board under
the board-first lock before touching that board's access timestamp. A fresh
four-process MySQL cluster run passed 200 card-reading requests racing with
moves of those cards to another board, alongside the existing mixed phases.
The check found no HTTP failures and verified representative destination
boards; it does not cover every notification or access transition.

Assignments now have a stored `updated_at` on both databases. MySQL startup
adds and backfills that column on existing databases before installing the
legacy-insert trigger; a four-row upgrade, fresh schema, direct legacy insert,
interrupted-backfill recovery, and the core flow passed. A migration marker
keeps later process starts from scanning all assignment rows. A new cluster
phase runs two assignment toggles on
each of 200 cards across four processes. Before serializing the membership
decision under the card's board lock, one run returned success for both
requests but left a sampled card unassigned. Four runs after the fix checked
all 200 final assignment rows and passed. Associated event, comment, and
notification writes are still separate from the membership transaction.

A new move/title phase in that test reproduced MySQL 1213 deadlocks when a
card moved between boards while another process edited its title. The move
transaction now locks both boards in ID order before changing the card and
related rows. The title-change event could still read the old board after the
move; event creation now locks the board, locks and rechecks the card, and
retries under its current board if it moved. Six fresh-account cluster runs
then passed 200 move/title pairs and 200 simultaneous moves in opposite
directions, alongside the earlier mixed phases. `tests/mysql_core.py`, the
SQLite card-update touch check, and 600 two-process SQLite card workflows also
passed. These runs check one host and these paths; other write combinations
remain outside the result.

The draft publication path also reproduced MySQL 1213 deadlocks when 32 draft
cards on one board were published concurrently through two app processes.
Publication now locks the board before updating a draft into a published card.
`tests/mysql_draft_cluster.py` keeps its browser draft and cross-process flow,
and optionally uses a MySQL CLI to seed 32 same-board drafts for this race.
Six fresh-account runs passed after the fix, with all 32 cards published and
exactly 32 publication events in the final run. This checks one board on one
host, not every draft callback, notification delivery, or multi-host case.

Assignment/title contention later exposed another MySQL 1213 deadlock. An
activity-spike callback held the card while its update trigger waited for the
board, while a title edit held the board and waited for that card. Event and
activity-spike writes now share a board-first transaction helper that locks
and rechecks the card's current board after moves. The expanded four-process
cluster test passed eleven fresh-account runs with 200 assignment/title pairs;
six runs also passed 200 tag/title pairs using one shared tag. The test
samples returned assignments, tags, and final titles. The MySQL core flow and
600 two-process SQLite card workflows passed after this change. These checks
do not cover every callback or prove multi-host capacity.

Tag toggles had a separate parity and contention gap: unlike Fizzy's Tagging
model, Rizzy did not touch the card or board, and its membership check and
write were separate transactions. A live source/Rust probe now matches the
card and board update times for both tag-on and tag-off while leaving
`last_active_at` unchanged. The Rust toggle now holds the board and card lock
through the membership decision, mutation, and card touch. Five fresh-account
MySQL cluster runs sent two concurrent toggles per card across four processes
for 200 cards; every request succeeded and every card retained its tag after
the pair. The MySQL core flow also passed. This is a same-host correctness
check, not a tag throughput or maximum-scale result.

Triage and send-back-to-triage now lock and recheck the card's current board
before writing. Triage verifies the chosen column inside that transaction;
if a move won the race, the old board's column returns 404 instead of being
attached to a card on another board. Send-back checks access to the current
board and records its event there. A fresh four-process cluster run raced 200
triages and 200 send-backs with card moves and found no HTTP 500s or orphaned
cross-board column links. The paired SQLite `tests/triage_board_parity.py`
matched Fizzy's sequential move, invalid old-column, new-column, and
untriage behavior, and `tests/column_touch_parity.py` still matched the
timestamp side effects. Concurrent semantics on other mutation paths remain
open.

Close, reopen, postpone, gild, and ungild now decide from card state read
under the current board and card locks. This prevents a stale pre-lock state
from inserting a second closure or goldness row, and rechecks access after a
board move. A fresh four-process cluster run raced 200 close, postpone, or
gold actions with moves, then sent two simultaneous close or gild requests to
each of another 200 cards. It checked all 200 moved cards' final flags, exact
closure/not-now/goldness row counts, and exactly 100 closure events for the
duplicate-close group. The paired SQLite closure, goldness, and Not Now
lifecycle checks still passed against pinned Fizzy. This is a same-host
correctness check for those operations; other races and sustained write scale
remain unverified.

The card-board transfer now locks both boards in ID order, locks and rechecks
the card's actual source board, and retries when another move made the first
snapshot stale. It rechecks source and destination access under those locks,
uses the verified source for attachment ledger transfers, and treats a card
already on the destination as a no-op. `tests/mysql_same_card_moves.py` held
the source board lock until two separate app processes were waiting, then
sent the image-bearing card to two different destinations. Three fresh
MySQL 8.4.11 runs passed with correct source names in both move events and
all image bytes charged only to the final board. The expanded four-process
cluster test and MySQL storage materialization test also passed. The paired
SQLite move-access and storage-ledger tests still matched Fizzy, as did a
new chained/repeated move test. This covers those same-host moves, not every
concurrent access change or distributed deployment.

Pin and unpin now hold the card's current board lock through the access
check and mutation. This orders a member's pin against a move to a private
board: either the pin commits first and move cleanup removes it, or the move
commits first and pin returns 404. `tests/mysql_card_move_access.py` forced both
requests to wait on the source board in separate app processes; three fresh
MySQL 8.4.11 runs left no inaccessible pin. The expanded four-process cluster
test also passed 200 owner pin/move pairs and checked all 200 retained pins
when access persisted. Paired Fizzy/Rizzy pin controls, tray, and access-loss
checks passed on SQLite. Other access changes and cross-host races remain
outside these results.

Watch and unwatch now also recheck access after locking the card's current
board. The same private-board race fixture creates a second card and forces a
member watch and owner move to wait on that board lock. Three fresh MySQL
8.4.11 runs left neither a pin nor a watch for the member after the moves;
the full four-process cluster still passed its owner watch/move phase. Paired
SQLite checks matched Fizzy's watch timestamp effects and both apps returned
404 when the former member tried to watch after losing access. Other
notification and broadcast effects under contention remain open.

The card edit, tag, step, comment, assignment, and reaction paths now check
the actor's access after locking the card's current board. The tag insertion
joins the card write transaction. The private-board fixture queues an owner
move ahead of member reaction, tag, assignment, and step requests behind a
MySQL board lock. Two fresh runs returned 404 for each later request and
found no new row for any of those four writes. Paired SQLite tests match
Fizzy's post-move 404 responses for these routes, and existing tag, reaction,
and comment timestamp comparisons still pass. The fresh MySQL core and
four-process cluster passed; other mutation routes and access-revocation
interleavings remain to be audited.

Card publication, deletion, and reading also use the current-board access
transaction. The private-board race fixture queues an owner move before a
member creator's delete and reading requests, then repeats with a real draft
created through a session. The later member requests returned 404, the
deleted card remained on the destination, and the draft kept its drafted
status and publication-event count. The paired access-loss check matched
Fizzy's ordinary 404 responses, while the 142-comparison API workflow,
two-process draft flow with 32 concurrent publishes, MySQL core, and fresh
four-process cluster passed. Other concurrent card lifecycle combinations
and access revocations remain open.

Card and draft creation now lock the destination board and check the creator's
current access before reserving a card number. In two fresh MySQL 8.4.11 runs,
`tests/mysql_card_creation_access.py` queued an owner board update that revoked
a member ahead of the member's JSON create, then repeated the ordering for a
session-authenticated browser draft create. Both late requests returned 404;
the board card count and account number counter stayed unchanged. The paired
SQLite fixture matched Fizzy's 404 response for board-scoped and account-
scoped creation after access loss. The existing two-server draft and fresh
four-process cluster suites passed. Other revocation paths and wider
concurrent creation behavior remain open.

`tests/mysql_pair_selfhost.py route-get` passed a 92-path paired GET census on
fresh MySQL databases. It compares JSON-request status and content type with
missing resource identifiers, including account-scope redirects, lookup order,
and missing-template errors. `route-get-contract` also compares redirect
destinations, JSON error bodies, and exact non-JSON error body digests for all
92 paths. `route-get-html` passes the same contract for signed-in HTML requests,
including static 404 pages and empty 406 responses. The paired `join-code` suite passed invite-link
browser behavior plus invalid QR signatures and absent-account prefixes for
JSON and SVG requests. Successful response bodies, valid-resource routes, and
most write paths are outside these censuses. The separate `route-get-valid`
suite uses equivalent persisted board, column, webhook, card, step, comment,
and reaction fixtures to match status, content type, and JSON shape for 18
populated GET paths. It exposed a user-activity JSON request that returned
HTML 200 in Rizzy; the handler now matches Fizzy's JSON 406. Full body value
and browser HTML parity for these paths remain open.
The `route-get-valid-html` suite matches status, content type, and resource
text on 30 populated signed-in HTML GET paths, plus one conditional steps
request; non-200 bodies are compared by length and digest. It exposed browser
HTML requests to the card-steps index, board-access listing, and named-column
card list returning JSON 200 in Rust; Rizzy now matches Fizzy's HTML 406.
The paired `step-read`, populated JSON GET, private/public board-view, and
named-column browser suites passed after the change. Full markup and
interaction parity remain unverified by this probe.
The disposable paired `auth-format` suite also passed bearer-only format,
read-token write denial, JSON suffix, and unauthenticated redirect checks on
fresh MySQL databases.
