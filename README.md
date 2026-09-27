# Rizzy

Rizzy is an in-progress Rust implementation of [Fizzy](https://github.com/basecamp/fizzy). The reference checkout in `upstream/` is pinned to commit `477c943e0506f109e5bc83ae9dadbe519732c045` and is ignored by this repository. Fizzy's API docs, routes, schema, and controller tests are the behavioral reference.

**Status: partial implementation.** Rizzy currently has a usable board view, email-code sign-in, and a subset of Fizzy's JSON API. It is not a one-for-one replacement. A parity checked local read workload runs faster, but a product-wide speed or scale advantage has not been established.

The [auto-postponement comparison](bench/results/auto-postpone-sqlite-5000-2026-09-26.json) processed 5,000 overdue cards among 10,000 cards on four SQLite boards in 0.645–0.689 seconds for Rizzy versus 68.08–68.11 seconds for the pinned Fizzy method across two reversed-order rounds. The timing excludes process startup and setup; per-card Not Now, event, comment, rich-text, and activity-time outcomes were checked. This measures that background sweep, not the whole app.

## Run

```sh
cargo run --release -- seed
# For local development only, show sign-in codes in the browser:
RIZZY_DEV_MAGIC_LINKS=1 cargo run --release
# Open http://localhost:3007/ and sign in with the seeded email.
```

The default database is `data/rizzy.sqlite3`. Set `RIZZY_DATABASE_URL` to a different SQLite URL, `RIZZY_BIND` to another bind address, and `RIZZY_DB_POOL_SIZE` to change the SQLite connection pool (default 12). `seed` creates a new account and owner token; its email, name, and account name can be set with `RIZZY_SEED_EMAIL`, `RIZZY_SEED_NAME`, and `RIZZY_SEED_ACCOUNT`. Card images and user avatar bytes are stored in `data/storage/files` by default; set `RIZZY_STORAGE_DIR` to move them. User and account exports are stored in `data/exports` by default; set `RIZZY_EXPORT_DIR` for another path. Uploaded account import ZIPs are stored in `data/imports` by default; set `RIZZY_IMPORT_DIR` to move them. Export notifications use `RIZZY_SENDMAIL` and `RIZZY_PUBLIC_URL` when configured.

Passkeys use `http://localhost:3007` by default. Set `RIZZY_PASSKEY_ORIGIN` to the exact browser origin for another domain or port, then open that origin in the browser. The server stores one-use registration and sign-in challenges in SQLite.

The server scans for overdue published cards at minute 50 of each UTC hour and moves them to Not Now. Multiple Rizzy processes share an hourly database claim. Run `cargo run --release -- auto-postpone` to process due cards immediately; set `RIZZY_DISABLE_AUTO_POSTPONE=1` to disable the built-in schedule when an external scheduler owns it.

The server deletes unused tags daily after 04:02 UTC, updating or removing saved filters that referenced them. Run `cargo run --release -- delete-unused-tags` to perform cleanup immediately; set `RIZZY_DISABLE_TAG_CLEANUP=1` when an external scheduler owns it.

For email delivery, set `RIZZY_SENDMAIL` to a sendmail-compatible executable and `RIZZY_MAIL_FROM` to its sender address. Notification bundle messages include links based on `RIZZY_PUBLIC_URL`, which should be set to the browser-facing origin. The server checks due bundles at minutes 00 and 30 UTC with a database claim shared across processes; `cargo run --release -- deliver-notifications` drains due bundles immediately. `RIZZY_DEV_MAGIC_LINKS=1` exposes sign-in codes and email-change tokens in response headers for local testing. Without either delivery option, email sign-in returns 503; the seeded API token can still be entered through **Use an API token**. Bearer tokens are accepted only for JSON requests, and read tokens cannot make write requests, matching Fizzy. On a fresh database, the first email-code sign-in or the browser **Sign up** form can create an account with Fizzy's Playground board and 11 starter cards. Set `RIZZY_MULTI_TENANT=1` to allow additional accounts and owner-initiated cancellation. Cancellation closes account access immediately; the hourly maintenance worker deletes its records and stored files after 30 days.

For browser push, set `RIZZY_VAPID_PRIVATE_KEY` (or Fizzy's `VAPID_PRIVATE_KEY`) to a URL-safe, unpadded base64 P-256 private key, `RIZZY_PUBLIC_URL` to the public HTTPS origin, and optionally `RIZZY_VAPID_SUBJECT` to a contact `mailto:` or HTTPS URI. Rizzy derives the browser public key, exposes a device subscription button in notification settings, encrypts push payloads, and checks its durable push queue every five seconds. `cargo run --release -- deliver-push` processes one queue batch. `RIZZY_DISABLE_PUSH_DELIVERY=1` pauses the worker for local tests. A browser must grant notification permission and reach its push provider.

## Implemented now

- Browser signup seeds Fizzy's Playground board with its 11 starter cards, creator watches, publication events, and remote orientation media.
- Bearer token and cookie session authentication with SHA-256 token hashes, single-use email codes, browser email and code entry, initial signup, account scoping, read versus write permission, token management, identity listing, user listing and editing, role changes, and private board access checks.
- Account join links with browser email/code/name flow, usage limits, reset, member name setup, user verification, and atomic redemption across Rust processes. Account settings can promote, demote, and remove people; a removed person can rejoin with a new user record.
- User email changes with validation, a 30-minute one-use confirmation token, session switch, and delivery through the configured sendmail executable.
- Single-use device sign-in links on the user's profile, including a scannable QR code, four-hour expiry, regeneration that revokes older links, and atomic redemption across concurrent requests.
- Passkey registration, naming, deletion, and sign-in with verified WebAuthn assertions and one-use challenges. The profile links to passkey management.
- Board access records and involvement, including grants for new account members, private-board membership changes, and cleanup of revoked members' watches, pins, and assignments.
- Board webhooks with browser management forms, action subscriptions, a durable delivery queue, signed JSON requests, bounded responses, private destination blocking, and deactivation after sustained delivery failures. Set `RIZZY_PUBLIC_URL` to the externally visible origin for payload links; `RIZZY_DISABLE_WEBHOOK_DELIVERY=1` pauses delivery for local testing.
- Special board column JSON lists (Maybe?, Not now, Done) and public pages for those lists and named columns. The anonymous public board has a centered Maybe column, collapsed side columns, rich description, published-card counts, and lazy column previews; public cards show their rich description and read-only steps.
- Initial notifications for board watchers, card assignments, card watchers, and User attachments in card/comment rich text, with per-card unread counts, notification and card read controls, tray JSON, and bundle email frequency settings. `@` User and `#` card picker routes return editor items; User items issue opaque attachment IDs, and writing those attachments creates mentions and in-app notifications. Draft and card pages now use Fizzy's Lexxy 0.9.32 editor. Rails-compatible signed IDs and full push delivery parity remain incomplete.
- Push subscription registration and deletion for the current user, with endpoint allowlisting, DNS resolution, and public IP validation. Notifications queue browser push jobs, encrypt and sign their payloads with VAPID, pin the resolved public push endpoint IP, retry transient failures, and remove expired subscriptions. The notification settings page can register this device's service worker and browser subscription. Live provider delivery and browser permission acceptance remain unverified.
- Account-scoped email unsubscribe links with one-month token expiry and anonymous confirmation. Notification changes are grouped into four-hour, daily, or weekly windows and delivered as multipart text/HTML bundles through sendmail, with board and card grouping, action details, avatars, unsubscribe links, and browser settings links. Some event details, rich-text formatting, and the full settings presentation remain to be reproduced.
- Board, column, and card create/read/update/delete JSON routes, including account level card creation; browser board creation and settings forms for name, access, auto-close period, public link, and deletion; account wide card numbering; card close, postpone, gold, triage, move, tag toggle, steps, comments, assignments, watches, pins, and reactions. The card page links to browser board, assignment, and tag pickers and card/comment reaction forms; moving into a private board grants assignee access only for a board administrator and removes inaccessible card data. Comment authors can open an edit form, save, and delete their comments. Pin and watch endpoints serve conditional HTML frames and Turbo replacements as well as JSON toggles. Card JSON accepts explicit creation and last activity timestamps.
- Rust-rendered board columns with card counts, a centered Maybe first page, collapsed adjacent columns, rich card previews on initial and lazy pages, more-card loading, card drag and drop, a watcher control, board-level text/status/sort filters with paginated results, draft navigation, and column creation; an all-cards browser grid with filters, automatic pagination, and source-guided card layout; and a published card page with the source-style color panel, stage rail, editable steps, background image upload and removal, tag toggling, rich text, comments, edit, watch, pin, Golden Ticket, and completion controls. The board, grid, and card screenshots in [visual baselines](docs/visual-baselines) show current progress against compiled Fizzy CSS. The overall presentation still needs a full Fizzy parity pass.
- Tag listing, card filters for saved views, terms, dates, closures, stalled activity, assignments, boards, columns and tags, and SQLite FTS5 card and comment search over visible plain text with indexed edits and deletes. Browser search shows separate card and comment results with highlighted excerpts and pagination; JSON search returns deduplicated cards.
- Initial saved filter create/delete routes with per-user deduplication and Turbo Stream toggle responses.
- Board publishing and unpublishing with unauthenticated public board and card pages.
- Direct uploads for card images, user avatars, and uploaded image/file embeds in card descriptions and comments, with private/public blob access and cached image previews.
- Account settings and entropy with browser forms, people listing, invite-link and usage-limit pages, board entropy overrides, and an activity feed backed by card and comment events.
- An account home activity timeline with Added, Updated, and Done columns, board and person filters, timezone grouping, day pagination, column detail pages, and a user activity frame.
- Fizzy-format account exports and an account import upload/status flow with transactional restoration of the record groups represented by Rizzy, including original uploaded files, card due dates, blob metadata and variants, mentions, notification bundles, and portable User references in rich text.
- A basic browser interface for boards, columns, cards, user profiles, description editing, and completion.
- An account navigation menu for boards, people, tags, settings, and shortcuts, with conditional ETag responses.
- A dedicated card edit route with title and Lexxy description fields, alongside the card page's inline editor.
- A Fizzy-matching service worker script and local offline bundle, plus a Rizzy web app manifest and icons. Automatic service worker registration across all browser pages is still pending.
- SQLite WAL storage with indexes for the board and card lookup paths.

## Parity still required

The reference contains 118 controllers, 164 model files, 384 view files, 70 database tables, and 351 test files. The current build does not yet cover most of that surface. Major missing work includes complete signup behavior, remaining multiuser flows, complete notification delivery and mention handling, saved filter UI and complete search behavior, the complete activity timeline, complete public board views, general remote and media-rich embeds, live push verification, specialized webhook payloads, full account transfer parity, background jobs, and the complete Hotwire UI. User exports include accessible cards, comments, and their uploaded rich-text attachments. Account exports cover records currently stored by Rizzy and can be imported by Fizzy for the tested workflow. Rizzy imports those represented record groups, but rejects unsupported source records rather than dropping them. Import ZIPs above 512 MiB and unrepresented source features still need work. Existing JSON responses also differ from Fizzy in several fields and corner cases. See [PARITY.md](PARITY.md) for the parity tracker.

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
python3 tests/mentions.py
python3 tests/user_attachables_migration.py
python3 tests/search.py
python3 tests/exports.py
python3 tests/account_transfer.py
python3 tests/account_import.py
python3 tests/notification_source_migration.py
# With both servers and their FIZZY_* / RIZZY_* credentials set:
python3 tests/parity.py
python3 tests/prompt_parity.py
python3 tests/filter_parity.py
python3 tests/saved_filter_parity.py
python3 tests/stalled_parity.py
python3 tests/board_views_parity.py
python3 tests/auth_format_parity.py
python3 tests/request_forgery_parity.py
python3 tests/search_engine_indexing_parity.py
python3 tests/allow_browser_parity.py
python3 tests/platform_html_parity.py
python3 tests/timezone_etag_parity.py
python3 tests/card_image_parity.py
python3 tests/avatar_parity.py
python3 tests/rich_text_parity.py
python3 tests/embed_parity.py
python3 tests/export_attachment_parity.py
python3 tests/account_export_parity.py
python3 tests/step_read_parity.py
python3 tests/events_html_parity.py
python3 tests/home_add_card_parity.py
python3 tests/timeline_event_descriptions_parity.py
python3 tests/timeline_event_media_parity.py
python3 tests/users_events_html_parity.py
python3 tests/user_profile_html_parity.py
python3 tests/session_transfer_parity.py
python3 tests/my_menu_parity.py
python3 tests/webhooks_html_parity.py
python3 tests/card_edit_html_parity.py
python3 tests/card_controls_parity.py
python3 tests/email_change_html_parity.py
python3 tests/email_confirmation_html_parity.py
python3 tests/search_rich_text_parity.py
python3 tests/search_html_parity.py
python3 tests/public_page_html_parity.py
```

For a comparative run, start both applications on the same machine using the same database engine, equivalent seed data, and production builds. Then set `FIZZY_URL`, `FIZZY_ACCOUNT`, `FIZZY_TOKEN`, `RIZZY_URL`, `RIZZY_ACCOUNT`, and `RIZZY_TOKEN` and run `python3 bench/compare.py --cards 100 --seconds 10`. The script creates equal card and activity counts, checks sampled response shapes and first page sizes, warms both services, and reports throughput, p50, p95, p99, and errors at several concurrency levels. `tests/parity.py` passed 142 comparisons for the implemented workflow using fresh accounts. Fizzy was brought up with a local Ruby 3.4.8 installation for the [diagnostic runs](bench/README.md). A valid product wide performance claim still requires full behavioral parity, repeated runs, CPU and memory measurements, and a larger dataset.

`bench/events_html.py` measures the browser activity timeline after creating equal card and event fixtures. It additionally needs `FIZZY_EMAIL`, `FIZZY_DB`, `RIZZY_EMAIL`, and `RIZZY_DB` so it can sign in to each browser session; set `RIZZY_DEV_MAGIC_LINKS=1` on the local Rust server. Results and limits are in [bench/README.md](bench/README.md).

## License

Fizzy is released under the O'Saasy License. The required notice is retained in [LICENSE.md](LICENSE.md). In particular, its terms restrict offering a derivative as a directly competing hosted service. Review the license before distribution or hosting.
