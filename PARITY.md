# Fizzy parity tracker

Reference revision: `477c943e0506f109e5bc83ae9dadbe519732c045`. Each item is complete only when its route behavior, JSON and HTML response, permissions, side effects, and upstream tests are reproduced. A check means an initial implementation exists; it does not establish one-for-one parity.

| Area | Initial implementation | Full parity |
| --- | --- | --- |
| Account scoped routing, bearer tokens, identity, user listing and settings | ✓ | No |
| Boards CRUD and access control | CRUD, access records, and involvement | No |
| Columns CRUD and card listings | Named and special board column listings, with public pages | No |
| Cards CRUD and card numbering | JSON CRUD, explicit timestamps, and account numbering | No |
| Closure, triage, postpone, gold, board move | ✓ | No |
| Tags, steps, comments | ✓ | No |
| Full board and card HTML interface | Basic UI | No |
| Sign up, magic links, sessions, passkeys | Magic links, sessions, and initial signup | No |
| Users, roles, invitations, board involvement | User listing/editing, roles, join links, email changes, verification, and name entry | No |
| Mobile client configuration and personal timezone | Android/iOS v1 JSON and timezone updates | No |
| Assignments, reactions, pins, watches | ✓ | No |
| Card filters, tag listing, basic search | Saved IDs, terms, dates, closures, activity spikes, and FTS5 index | No |
| Activities and core event side effects | ✓ | No |
| Saved filters and full text search | FTS5 index; saved filter persistence and Turbo toggles | No |
| Board publication and basic public pages | ✓ | No |
| Public board workflow and webhooks | Public pages; webhook CRUD, event queue and JSON delivery | No |
| Notifications, mail, push, jobs | Basic event notifications and read controls | No |
| Attachments and Action Text rich text | — | No |
| Imports, exports, account lifecycle | Asynchronous user card ZIP exports, authenticated download, and expiry | No |
| API response shape and pagination parity | Partial | No |
| SQLite and MySQL deployment parity | SQLite only | No |
| Comparative performance and scale proof | Harness added | No |

The first fidelity gate is to run each upstream API controller test against an equivalent Rust route and compare status, response fields, authorization, and database side effects. The second is visual and interaction testing for each original view. The third is benchmark comparison using parity checked responses and equal data volumes.

[ROUTE_COVERAGE.md](ROUTE_COVERAGE.md) inventories source controller actions and Rizzy method/path declarations. Route presence does not establish functional parity.

The local `tests/parity.py` scenario currently passes 142 comparisons against the running reference server using fresh accounts. It checks status and JSON shape for the core workflow, selected state values, search results, tag, assignment and card state filters, account and board entropy, join-code metadata, board access and involvement, webhook management, basic notification routes and settings, card reading, publication, activity events and descriptions, event deletion, and system comment count. `tests/auth.py`, `tests/join.py`, `tests/email_change.py`, `tests/verification.py`, `tests/webhooks.py`, and `tests/notifications.py` exercise Rust authentication, join, email change, verification, webhook, and notification flows with disposable accounts; they are not yet cross-implementation comparisons. Neither test replaces the full upstream test suite.

`tests/filter_parity.py` passes 13 additional cross-app card-result comparisons, including terms, closure dates, closers, a backdated card, and a card approaching auto-postponement. `tests/saved_filter_parity.py` checks that a saved filter selects the same closed card on both apps; the Rails filter is created through its model layer because its Turbo controller requires a browser session. This does not cover the full filter UI or every time zone.
`tests/stalled_parity.py` additionally checks the stalled index before and after an old spike, a fresh comment, and postponement. Local `tests/stalled.py` checks spike creation from assignment, reopening, and multi-person comments. These checks do not cover all background-job timing or all search syntax.

`tests/exports.py` checks user export ZIP content, board access scope, download permissions, asynchronous completion, and the 24-hour request limit. A live Fizzy export confirmed the same sample card's board, title, status, escaped description, and comment body format. User ZIPs currently omit attachments because Rizzy does not yet store them; account ZIP export and import are still missing. The background worker retries processing exports after a five-minute stale lease and removes completed exports after 24 hours.

`tests/board_views_parity.py` compares private special-column JSON status, response shape, card order, and pagination, plus public column membership and pagination, against live Fizzy. The Rust public pages are still simplified and need a full visual and interaction pass. Fizzy rejects bearer-token requests for these HTML views while Rizzy currently accepts them; unauthenticated JSON also follows different sign-in behavior. The test checks that private JSON is protected without treating those differing status codes as parity.

Fizzy's production JSON response immediately after changing an empty board's entropy can show its cached old value. Its focused controller test expects the new value and passes. The parity scenario checks the saved value after a board update invalidates that cache; Rizzy currently returns the new value immediately.
