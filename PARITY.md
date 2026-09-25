# Fizzy parity tracker

Reference revision: `477c943e0506f109e5bc83ae9dadbe519732c045`. Each item is complete only when its route behavior, JSON and HTML response, permissions, side effects, and upstream tests are reproduced. A check means an initial implementation exists; it does not establish one-for-one parity.

| Area | Initial implementation | Full parity |
| --- | --- | --- |
| Account scoped routing, bearer tokens, identity, user listing and settings | ✓ | No |
| Boards CRUD and access control | CRUD, access records, and involvement | No |
| Columns CRUD and card listings | ✓ | No |
| Cards CRUD and card numbering | ✓ | No |
| Closure, triage, postpone, gold, board move | ✓ | No |
| Tags, steps, comments | ✓ | No |
| Full board and card HTML interface | Basic UI | No |
| Sign up, magic links, sessions, passkeys | Magic links, sessions, and initial signup | No |
| Users, roles, invitations, board involvement | User listing/editing, roles, join links, email changes, verification, and name entry | No |
| Mobile client configuration and personal timezone | Android/iOS v1 JSON and timezone updates | No |
| Assignments, reactions, pins, watches | ✓ | No |
| Card filters, tag listing, basic search | FTS5 card and comment index | No |
| Activities and core event side effects | ✓ | No |
| Saved filters and full text search | FTS5 index; saved filters absent | No |
| Board publication and basic public pages | ✓ | No |
| Public board workflow and webhooks | Public pages; webhook CRUD, event queue and JSON delivery | No |
| Notifications, mail, push, jobs | Basic event notifications and read controls | No |
| Attachments and Action Text rich text | — | No |
| Imports, exports, account lifecycle | — | No |
| API response shape and pagination parity | Partial | No |
| SQLite and MySQL deployment parity | SQLite only | No |
| Comparative performance and scale proof | Harness added | No |

The first fidelity gate is to run each upstream API controller test against an equivalent Rust route and compare status, response fields, authorization, and database side effects. The second is visual and interaction testing for each original view. The third is benchmark comparison using parity checked responses and equal data volumes.

[ROUTE_COVERAGE.md](ROUTE_COVERAGE.md) inventories source controller actions and Rizzy method/path declarations. Route presence does not establish functional parity.

The local `tests/parity.py` scenario currently passes 142 comparisons against the running reference server using fresh accounts. It checks status and JSON shape for the core workflow, selected state values, search results, tag, assignment and card state filters, account and board entropy, join-code metadata, board access and involvement, webhook management, basic notification routes and settings, card reading, publication, activity events and descriptions, event deletion, and system comment count. `tests/auth.py`, `tests/join.py`, `tests/email_change.py`, `tests/verification.py`, `tests/webhooks.py`, and `tests/notifications.py` exercise Rust authentication, join, email change, verification, webhook, and notification flows with disposable accounts; they are not yet cross-implementation comparisons. Neither test replaces the full upstream test suite.

Fizzy's production JSON response immediately after changing an empty board's entropy can show its cached old value. Its focused controller test expects the new value and passes. The parity scenario checks the saved value after a board update invalidates that cache; Rizzy currently returns the new value immediately.
