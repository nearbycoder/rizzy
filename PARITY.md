# Fizzy parity tracker

Reference revision: `477c943e0506f109e5bc83ae9dadbe519732c045`. Each item is complete only when its route behavior, JSON and HTML response, permissions, side effects, and upstream tests are reproduced. A check means an initial implementation exists; it does not establish one-for-one parity.

| Area | Initial implementation | Full parity |
| --- | --- | --- |
| Account scoped routing, bearer tokens, identity, user listing | ✓ | No |
| Boards CRUD and access control | ✓ | No |
| Columns CRUD and card listings | ✓ | No |
| Cards CRUD and card numbering | ✓ | No |
| Closure, triage, postpone, gold, board move | ✓ | No |
| Tags, steps, comments | ✓ | No |
| Full board and card HTML interface | Basic UI | No |
| Sign up, magic links, sessions, passkeys | — | No |
| Users, roles, invitations, board involvement | — | No |
| Assignments, reactions, pins, watches | ✓ | No |
| Card filters, tag listing, basic search | ✓ | No |
| Saved filters, full text search, activities, events | — | No |
| Board publication and basic public pages | ✓ | No |
| Public board workflow and webhooks | — | No |
| Notifications, mail, push, jobs | — | No |
| Attachments and Action Text rich text | — | No |
| Imports, exports, account lifecycle | — | No |
| API response shape and pagination parity | Partial | No |
| SQLite and MySQL deployment parity | SQLite only | No |
| Comparative performance and scale proof | Harness added | No |

The first fidelity gate is to run each upstream API controller test against an equivalent Rust route and compare status, response fields, authorization, and database side effects. The second is visual and interaction testing for each original view. The third is benchmark comparison using parity checked responses and equal data volumes.

The local `tests/parity.py` scenario currently passes 86 comparisons against the running reference server. It checks status and JSON shape for the core workflow, selected state values, search results, tag, assignment and card state filters, publication, and system comment count. It does not replace the full upstream test suite.
