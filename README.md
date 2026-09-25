# Rizzy

Rizzy is an in-progress Rust implementation of [Fizzy](https://github.com/basecamp/fizzy). The reference checkout in `upstream/` is pinned to commit `477c943e0506f109e5bc83ae9dadbe519732c045` and is ignored by this repository. Fizzy's API docs, routes, schema, and controller tests are the behavioral reference.

**Status: partial implementation.** Rizzy currently has a usable board view and a subset of Fizzy's JSON API. It is not a one-for-one replacement. A parity checked local read workload runs faster, but a product-wide speed or scale advantage has not been established.

## Run

```sh
cargo run --release -- seed
# Save the printed account number and token.
cargo run --release
# Open http://127.0.0.1:3007/ and enter those credentials.
```

The default database is `data/rizzy.sqlite3`. Set `RIZZY_DATABASE_URL` to a different SQLite URL and `RIZZY_BIND` to another bind address. `seed` creates a new account and owner token; its email, name, and account name can be set with `RIZZY_SEED_EMAIL`, `RIZZY_SEED_NAME`, and `RIZZY_SEED_ACCOUNT`.

## Implemented now

- Bearer token authentication with SHA-256 token hashes, account scoping, read versus write permission, token management, identity listing, user listing and editing, and private board access checks.
- Board, column, and card create/read/update/delete JSON routes; account wide card numbering; card close, postpone, gold, triage, move, tag toggle, steps, comments, assignments, watches, pins, and reactions.
- Tag listing, selected card filters, and basic card search.
- Board publishing and unpublishing with unauthenticated public board and card pages.
- A basic browser interface for boards, columns, cards, description editing, and completion.
- SQLite WAL storage with indexes for the board and card lookup paths.

## Parity still required

The reference contains 118 controllers, 164 model files, 384 view files, 70 database tables, and 351 test files. The current build does not yet cover most of that surface. Major missing work includes magic link and passkey sessions, multiuser management, notifications, saved filters and full text search, activity timeline, complete public board views, rich text and attachments, push, webhooks, imports and exports, background jobs, and the complete Hotwire UI. Existing JSON responses also differ from Fizzy in several fields and corner cases. See [PARITY.md](PARITY.md) for a route level checklist.

## Verification and benchmarks

```sh
cargo test
python3 tests/smoke.py
# With both servers and their FIZZY_* / RIZZY_* credentials set:
python3 tests/parity.py
```

For a comparative run, start both applications on the same machine using the same database engine, equivalent seed data, and production builds. Then set `FIZZY_URL`, `FIZZY_ACCOUNT`, `FIZZY_TOKEN`, `RIZZY_URL`, `RIZZY_ACCOUNT`, and `RIZZY_TOKEN` and run `python3 bench/compare.py --cards 100 --seconds 10`. The script creates equal card counts, checks sampled response shapes and first page size, warms both services, and reports throughput, p50, p95, p99, and errors at several concurrency levels. `tests/parity.py` passed 86 comparisons for the implemented workflow. Fizzy was brought up with a local Ruby 3.4.8 installation for the [diagnostic runs](bench/README.md). A valid product wide performance claim still requires full behavioral parity, repeated runs, CPU and memory measurements, and a larger dataset.

## License

Fizzy is released under the O'Saasy License. The required notice is retained in [LICENSE.md](LICENSE.md). In particular, its terms restrict offering a derivative as a directly competing hosted service. Review the license before distribution or hosting.
