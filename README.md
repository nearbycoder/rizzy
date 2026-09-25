# Rizzy

Rizzy is an in-progress Rust implementation of [Fizzy](https://github.com/basecamp/fizzy). The reference checkout in `upstream/` is pinned to commit `477c943e0506f109e5bc83ae9dadbe519732c045` and is ignored by this repository. Fizzy's API docs, routes, schema, and controller tests are the behavioral reference.

**Status: partial implementation.** Rizzy currently has a usable board view, email-code sign-in, and a subset of Fizzy's JSON API. It is not a one-for-one replacement. A parity checked local read workload runs faster, but a product-wide speed or scale advantage has not been established.

## Run

```sh
cargo run --release -- seed
# For local development only, show sign-in codes in the browser:
RIZZY_DEV_MAGIC_LINKS=1 cargo run --release
# Open http://127.0.0.1:3007/ and sign in with the seeded email.
```

The default database is `data/rizzy.sqlite3`. Set `RIZZY_DATABASE_URL` to a different SQLite URL, `RIZZY_BIND` to another bind address, and `RIZZY_DB_POOL_SIZE` to change the SQLite connection pool (default 12). `seed` creates a new account and owner token; its email, name, and account name can be set with `RIZZY_SEED_EMAIL`, `RIZZY_SEED_NAME`, and `RIZZY_SEED_ACCOUNT`. Card image bytes are stored in `data/storage/files` by default; set `RIZZY_STORAGE_DIR` to move them. User data exports are stored in `data/exports` by default; set `RIZZY_EXPORT_DIR` for another path. Export notifications use `RIZZY_SENDMAIL` and `RIZZY_PUBLIC_URL` when configured.

For email delivery, set `RIZZY_SENDMAIL` to a sendmail-compatible executable and `RIZZY_MAIL_FROM` to its sender address. `RIZZY_DEV_MAGIC_LINKS=1` exposes sign-in codes and email-change tokens in response headers for local testing. Without either delivery option, email sign-in returns 503; the seeded API token can still be entered through **Use an API token**. Bearer tokens are accepted only for JSON requests, and read tokens cannot make write requests, matching Fizzy. On a fresh database, the first email-code sign-in can create an account. Set `RIZZY_MULTI_TENANT=1` to allow additional accounts.

## Implemented now

- Bearer token and cookie session authentication with SHA-256 token hashes, single-use email codes, initial signup, account scoping, read versus write permission, token management, identity listing, user listing and editing, role changes, and private board access checks.
- Account join links with usage limits, reset, member name setup, user verification, and atomic redemption across Rust processes.
- User email changes with validation, a 30-minute one-use confirmation token, session switch, and delivery through the configured sendmail executable.
- Board access records and involvement, including grants for new account members, private-board membership changes, and cleanup of revoked members' watches, pins, and assignments.
- Board webhooks with action subscriptions, a durable delivery queue, signed JSON requests, bounded responses, private destination blocking, and deactivation after sustained delivery failures. Set `RIZZY_PUBLIC_URL` to the externally visible origin for payload links; `RIZZY_DISABLE_WEBHOOK_DELIVERY=1` pauses delivery for local testing.
- Special board column JSON lists (Maybe?, Not now, Done) and public pages for those lists and named columns. The public board page links to each view.
- Initial notifications for board watchers, card assignments, and card watchers, with per-card unread counts, notification and card read controls, tray JSON, and bundle email frequency settings.
- Board, column, and card create/read/update/delete JSON routes; account wide card numbering; card close, postpone, gold, triage, move, tag toggle, steps, comments, assignments, watches, pins, and reactions. Card JSON accepts explicit creation and last activity timestamps.
- Tag listing, card filters for saved views, terms, dates, closures, stalled activity, assignments, boards, columns and tags, and SQLite FTS5 card and comment search with indexed edits and deletes.
- Initial saved filter create/delete routes with per-user deduplication and Turbo Stream toggle responses.
- Board publishing and unpublishing with unauthenticated public board and card pages.
- Account settings and entropy, board entropy overrides, and an activity feed backed by card and comment events.
- A basic browser interface for boards, columns, cards, description editing, and completion.
- SQLite WAL storage with indexes for the board and card lookup paths.

## Parity still required

The reference contains 118 controllers, 164 model files, 384 view files, 70 database tables, and 351 test files. The current build does not yet cover most of that surface. Major missing work includes passkeys and complete signup behavior, remaining multiuser flows, complete notification delivery and mention handling, saved filter UI and complete search behavior, the complete activity timeline, complete public board views, rich text and attachments, push, specialized webhook payloads, account imports and exports, background jobs, and the complete Hotwire UI. User exports currently include accessible cards and comments but cannot include attachments until attachment storage is built. Existing JSON responses also differ from Fizzy in several fields and corner cases. See [PARITY.md](PARITY.md) for the parity tracker.

## Verification and benchmarks

```sh
cargo test
python3 tests/smoke.py
python3 tests/auth.py
python3 tests/join.py
python3 tests/verification.py
python3 tests/filters.py
python3 tests/filter_results.py
python3 tests/stalled.py
python3 tests/card_timestamps.py
python3 tests/email_change.py
python3 tests/client_settings.py
python3 tests/cluster_writes.py
python3 tests/webhooks.py
python3 tests/notifications.py
python3 tests/search.py
python3 tests/exports.py
# With both servers and their FIZZY_* / RIZZY_* credentials set:
python3 tests/parity.py
python3 tests/filter_parity.py
python3 tests/saved_filter_parity.py
python3 tests/stalled_parity.py
python3 tests/board_views_parity.py
python3 tests/auth_format_parity.py
```

For a comparative run, start both applications on the same machine using the same database engine, equivalent seed data, and production builds. Then set `FIZZY_URL`, `FIZZY_ACCOUNT`, `FIZZY_TOKEN`, `RIZZY_URL`, `RIZZY_ACCOUNT`, and `RIZZY_TOKEN` and run `python3 bench/compare.py --cards 100 --seconds 10`. The script creates equal card and activity counts, checks sampled response shapes and first page sizes, warms both services, and reports throughput, p50, p95, p99, and errors at several concurrency levels. `tests/parity.py` passed 142 comparisons for the implemented workflow using fresh accounts. Fizzy was brought up with a local Ruby 3.4.8 installation for the [diagnostic runs](bench/README.md). A valid product wide performance claim still requires full behavioral parity, repeated runs, CPU and memory measurements, and a larger dataset.

## License

Fizzy is released under the O'Saasy License. The required notice is retained in [LICENSE.md](LICENSE.md). In particular, its terms restrict offering a derivative as a directly competing hosted service. Review the license before distribution or hosting.
