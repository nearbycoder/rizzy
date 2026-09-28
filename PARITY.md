# Fizzy parity tracker

Reference revision: `477c943e0506f109e5bc83ae9dadbe519732c045`. Each item is complete only when its route behavior, JSON and HTML response, permissions, side effects, and upstream tests are reproduced. A check means an initial implementation exists; it does not establish one-for-one parity.

The notification settings browser page now uses the shared signed-in shell and
Fizzy's header, two settings panels, board involvement frames and forms, push
status and help disclosure, and email frequency form structure. A disposable
paired `tests/notification_settings_paired.py` run checks panel and frame
structure, alphabetized boards, HTML and JSON involvement updates, method
override forms, email frequency saves, and the absent-method 404 against the
pinned source. `tests/notification_settings_html.py` checks the Rust page's
browser controls and unsubscribe-token creation; `tests/pwa_registration.py`
also passes for the page. The paired check now compares push-help headings,
steps, paragraphs, and section counts across Chrome on Linux and Android,
Firefox on Windows and Android, Safari on iOS and macOS, Chrome on iOS, Edge on
Windows, and an unknown user agent. Chromium opened the Chrome/Linux help
disclosure without page errors. Native-device settings, exact styling, and
live push delivery remain open. Paired Chromium
screenshots at 1280 and 390 pixels record the [Fizzy desktop](docs/visual-baselines/fizzy-notification-settings-1280.png),
[Rizzy desktop](docs/visual-baselines/rizzy-notification-settings-1280.png),
[Fizzy mobile](docs/visual-baselines/fizzy-notification-settings-390.png), and
[Rizzy mobile](docs/visual-baselines/rizzy-notification-settings-390.png)
pages with one board and the default email setting. Rizzy toggled board
involvement in place and auto-saved the email choice in Chromium, with no page
errors or horizontal overflow at 390 pixels. The pinned source's live board
watch form returned HTTP 406 in this local production setup, so its click flow
was not established as a passing browser comparison. Its request contracts
remain covered by the paired test.

`python3 tests/route_coverage.py --strict` now finds all 211 application routes, three Turbo Native routes, nine Active Storage routes, and 14 Action Mailbox routes registered. `tests/action_mailbox_parity.py` compares the pinned production source's default disabled-ingress and gated-conductor responses with Rizzy across all 14 methods, three JSON suffixes, HTML and JSON Accept headers, and anonymous and bearer-token requests. Configured mailbox ingress and development conductor behavior remain open, as does functional parity for the rest of the route inventory.

`tests/http_methods_parity.py` compares 493 paired HEAD and unsupported-method contracts on disposable source and Rust servers. All 92 application GET paths match for authenticated JSON HEAD; their OPTIONS responses match under HTML and JSON Accept headers, with and without a bearer token. Six `.json` paths exercise conflicting Accept headers, and six other unsupported verbs match under authenticated JSON. Two unsupported `.json` writes verify that the error keeps the original HTML Accept preference. An anonymous GET to the POST-only passkey challenge also returns the same 404. Fizzy returns a negotiated 404 for these method misses; Rizzy's router now does the same without Axum's `Allow` header. `tests/shadowed_routes_parity.py` compares another 152 requests across six literal paths that overlap dynamic resource routes, plus the unscoped access-token path. It covers every dynamic-only GET, HEAD, PATCH, PUT, and DELETE method on those paths, with JSON suffixes, HTML and JSON Accept headers, and anonymous and bearer-token requests. These error and authentication contracts match; valid-resource behavior, side effects, and other route families still require parity checks.

`tests/route_get_status_parity.py` now starts disposable paired apps and compares 92 registered GET paths under authenticated JSON and anonymous HTML, 184 cases total. Status, content type, redirect destination, and error body match the pinned source. The comparison exposed and fixed sign-in redirect content type, authentication before scoped public-route rejection, card-preview authentication before format rejection, and a static 500 page that had gained a CSP nonce. This is a missing-resource and unauthenticated route census, not verification of successful page content or interaction.

`tests/route_get_valid_paired.py` repeats the populated-route comparison on disposable SQLite apps. Its 36 JSON contracts match response shapes, stable field values, pagination, and selected cache/write headers. Its 59 browser contracts match status, content type, selected content markers, layout flags, and conditional response behavior. The initial fixture showed large HTML differences: Fizzy's webhook creation page was 51,748 bytes versus Rizzy's 5,203, and the card edit page was 45,595 versus 3,695. Both pages have since received closer source-shaped rendering, but the source's shared navigation, form details, and much of the presentation remain to be ported and compared structurally.

`tests/html_form_inventory.py` extracts form actions, methods, and named controls
from seven populated, signed-in pages on disposable paired apps. The board
settings page now has six forms on each app: the main board, auto-close,
publication, and deletion forms, plus shared-shell search and notification
forms. Rizzy places public description in a separate form shown when published.
`tests/board_settings_html_paired.py` checks fresh Fizzy and Rizzy board
creation, updates, deletion, and Turbo Stream replacements for auto-close and
publication. It also checks that ordinary HTML form submissions mutate state
but return the source's empty 406 response, while JSON form submissions return
the source's success statuses. Rizzy's settings page now uses the signed-in
shell, two source-shaped panels, a radial auto-close selector, a public-link
switch bar, and a delete confirmation dialog. Signed-in Chromium changed the
Rust auto-close choice, published and unpublished in place, and opened and
canceled deletion. The [Fizzy desktop](docs/visual-baselines/fizzy-board-settings-1280.png),
[Rizzy desktop](docs/visual-baselines/rizzy-board-settings-1280.png),
[Fizzy mobile](docs/visual-baselines/fizzy-board-settings-390.png), and
[Rizzy mobile](docs/visual-baselines/rizzy-board-settings-390.png) screenshots
record the remaining visual differences. Fizzy's draggable knob behavior,
published rich-text editor, avatars and icons, and exact presentation still
differ. On the populated card page, Fizzy has 14 forms and Rizzy 13. Pin and
watch use POST forms with delete method overrides, and
`tests/card_controls_paired.py` compares their card-page presence, cached
frames, Turbo replacements, and JSON actions against fresh pinned Fizzy.
Golden Ticket and completion now use POST forms with delete overrides.
`tests/card_flag_forms_paired.py` compares those forms, the four create/destroy
Turbo target sets, and JSON actions on fresh paired apps. Chromium toggled all
four card controls; the Golden Ticket and completion checks also passed at
390 pixels. Card and comment reactions now render POST delete-override forms;
fresh paired HTTP and browser checks cover their deletion. The remaining card
form is inside Fizzy's embedded closure-restoration Turbo Stream, which Rizzy
does not yet render. The direct step edit form now includes the source's hidden
unchecked value; `tests/step_edit_completed_paired.py` checks that editing a
completed step submits it and reopens the step. The inventory remains diagnostic for those interactions and
the remaining markup differences.

The comment edit page now renders inside Rizzy's application shell, and the in-card editor uses the same shared controls and styling. `tests/comment_edit_html_parity.py` passed against disposable Fizzy and Rizzy servers, including page-shell markers, Turbo create/edit/delete, JSON updates, and comment side effects. A paired Chromium check confirmed that opening and saving the inline editor keeps each app on its card URL, closes the editor, and shows the saved text. The [direct Fizzy](docs/visual-baselines/fizzy-comment-edit-direct-1280.png), [direct Rizzy](docs/visual-baselines/rizzy-comment-edit-direct-1280.png), [inline Fizzy](docs/visual-baselines/fizzy-comment-edit-inline-1280.png), and [inline Rizzy](docs/visual-baselines/rizzy-comment-edit-inline-1280.png) screenshots record the current visual comparison. This verifies one comment flow, not complete card-page visual or interaction parity.

The direct card edit page now uses Fizzy's full-width, borderless title and Lexxy layout instead of a separate brown panel. The [Fizzy](docs/visual-baselines/fizzy-card-edit-direct-1280.png) and [Rizzy](docs/visual-baselines/rizzy-card-edit-direct-1280.png) screenshots use the same title and description. `tests/card_edit_html_paired.py` passed against fresh SQLite apps after the change. A signed-in Chromium check also saved a title with Enter and canceled with Escape in each app; both persisted the save or discarded the edit, replaced the editor with read-only content, and kept the `/edit` URL. The paired test checks request and response contracts, while the browser check covers this direct-page interaction; broader card editing controls and visual parity remain open.

The webhook index and creation pages now use a centered source-shaped header and form layout, including switch controls and the empty-list add button. The creation page's back link now targets the webhook list, the list's back link names its board, and the edit page uses its webhook name in the document title and source placeholder. `tests/webhooks_html_paired.py` passed the creation, validation, update, deletion, and navigation checks against fresh apps. Paired Chromium clicks changed all 11 event switches on and off in both apps and followed the creation page's back link to the list. [Fizzy new](docs/visual-baselines/fizzy-webhook-new-1280.png), [Rizzy new](docs/visual-baselines/rizzy-webhook-new-1280.png), [Fizzy index](docs/visual-baselines/fizzy-webhook-index-1280.png), and [Rizzy index](docs/visual-baselines/rizzy-webhook-index-1280.png) record the visual comparison. The populated detail page now places its Edit control in the header and presents the secret, subscriptions, and deliveries in source order and spacing. The paired regression checks its edit link and full signature guidance; Chromium followed the detail/edit back links and showed both subscribed actions selected. [Fizzy detail](docs/visual-baselines/fizzy-webhook-show-1280.png), [Rizzy detail](docs/visual-baselines/rizzy-webhook-show-1280.png), [Fizzy edit](docs/visual-baselines/fizzy-webhook-edit-1280.png), and [Rizzy edit](docs/visual-baselines/rizzy-webhook-edit-1280.png) record the new comparison. Populated delivery controls and the rest of the shared shell still need fuller parity checks.

The populated webhook index now renders a compact name, dashed separator, and circular minus control like Fizzy. The [Fizzy](docs/visual-baselines/fizzy-webhook-index-populated-1280.png) and [Rizzy](docs/visual-baselines/rizzy-webhook-index-populated-1280.png) screenshots record the comparison. A live browser check caught an actual confirmation bug: Rizzy's old inline click handler let a canceled removal submit. Confirmation now runs from a form submit listener. With `window.confirm` returning false, both apps retained the webhook; returning true removed it and left each database with zero webhooks. `tests/webhooks_html_paired.py` still passes CRUD and now asserts the row's separator, accessible Remove label, and source confirmation text.

`tests/webhook_delivery_html_paired.py` now creates a real delivery on each fresh app and compares the populated webhook page through pending, completed with HTTP 200, completed with HTTP 500, and errored states. Rizzy renders the source's delivery DOM ID, success class, relative time label, and `<time datetime>` value; the paired test and the existing webhook browser CRUD test pass. Other relative-time boundaries, the delivery JSON page under varied event types, and browser presentation of longer delivery lists remain open.

The same paired delivery fixture now checks JSON behavior after storing a request with a signature header and body: both apps expose only the non-signature headers. It also dispatches a comment-created event and verifies that the JSON eventable URL points to the comment anchor on its card. The prior Rust response exposed all stored request fields and linked comments to a different path. Other event types, JSON cache timing, and larger delivery pages remain open.

The paired generic webhook wire test uncovered an activity renderer precision mismatch when a card carried a supplied microsecond timestamp: Fizzy's nested card JSON used milliseconds, while Rizzy sent microseconds. The shared activity renderer now serializes card, comment, and event timestamps at JSON millisecond precision. `tests/timestamp_precision_parity.py` checks the nested activity values on both apps; `tests/webhook_wire_payload_parity.py` again matches card and comment wire bodies, including the explicit card timestamp. Untested event variants and full delivery behavior remain open.

The delivery fixture now expands to 24 real events. Both apps return 15 rows and a next link on page one, nine rows without a next link on page two, distinct IDs in descending delivery time, and 20 rows on the webhook HTML page. It also compares older delivery labels at 50, 400, 600, and 700 days. The 50-day check exposed a Rust label of “about 1 month” where Rails rendered “about 2 months”; Rizzy now follows the pinned Rails month and year ranges. Exact boundary seconds and all subscribed event kinds still need verification.

The [200-delivery JSON read benchmark](bench/results/webhook-deliveries-read-200-2026-09-28.json) created the same 200 cards, events, and pending deliveries on each fresh SQLite app. Its 100-row fourth page matched decoded content and compact serialized size (43,603 bytes), after masking generated IDs, times, and hosts. On the same four assigned server CPUs, four Fizzy Puma workers and one Rizzy process completed two reversed-order ten-second rounds at 8, 32, and 64 clients, all with zero HTTP errors and unchanged record counts. Rizzy served 1,484–1,724 requests/s versus Fizzy's 202–229 (7.0–7.8×); p95 was 6.7–42.3 ms versus 56–400 ms. Both approached a local read-throughput plateau by 32–64 clients. This verifies faster local delivery-history reads on this fixture; successful outbound delivery throughput, multi-host scaling, other event types, and whole-app parity remain open.

The real-worker card publication benchmark initially exposed a Rust delivery bottleneck: [before the worker change](bench/results/webhook-publish-delivery-200-c8-2026-09-28.json), Rizzy completed a 200-card/200-delivery batch at 34.9–38.2 deliveries/s while Fizzy completed 41.8–44.0. Rizzy had been claiming at most eight pending deliveries per 250 ms polling tick. The worker now refills a free slot immediately, retaining idle polling and stale-delivery recovery. On clean-tree [8-client](bench/results/webhook-publish-delivery-refill-200-c8-2026-09-28.json), [32-client](bench/results/webhook-publish-delivery-refill-200-c32-2026-09-28.json), and [64-client](bench/results/webhook-publish-delivery-refill-200-c64-2026-09-28.json) repeats, Rizzy completed 681–742 deliveries/s versus Fizzy's 42–44, across two reversed-order rounds per level. Every run had zero HTTP errors, 200 unique card numbers, equal final card/event/completed-delivery counts, and sampled JSON states, actions, response summaries, and shapes matching the source. The apps shared four server CPU affinity slots; Fizzy used three Puma workers and one Solid Queue worker, while Rizzy used one process with an integrated eight-slot delivery worker. The destination was a blocked loopback URI, so these results test queue dispatch, payload preparation, and private-target rejection. Successful outbound delivery, retries, maximum backlog size, multi-host capacity, and whole-app parity remain unverified.

Larger clean-tree [1,000-card at 64 clients](bench/results/webhook-publish-delivery-1000-c64-2026-09-28.json) and [2,000-card at 128 clients](bench/results/webhook-publish-delivery-2000-c128-2026-09-28.json) batches also passed two reversed-order rounds with zero HTTP errors, unique card numbers, exact final card/event/completed-delivery counts, and sampled completed `private_uri` responses. Rizzy cleared these fixed batches at 994–1,451 deliveries/s versus Fizzy's 53–57, a matched-round 18.6–26.9× difference. Card POST p95 ranged from 32–48 ms for Rizzy and 793–1,163 ms for Fizzy. All phases used four server CPUs, one Rizzy process with its integrated worker, and three Fizzy Puma workers plus one Solid Queue worker. These figures include card creation and queue drain against a blocked destination; they are not successful outbound throughput or maximum sustainable load.

`tests/passkey_csrf_parity.py` compares 20 live challenge requests, scoped and unscoped, with missing, wrong, and valid CSRF tokens sent by header or form field under HTML and JSON Accept. Status, content type, redirect, and rejection body match the pinned source; success checks the presence of a challenge without equating its bytes. Rizzy now places a signed CSRF token on its sign-in and passkey management pages, includes it in browser challenge fetches, and checks it before the scoped redirect. `tests/passkey_flow_paired.py` runs both apps through registration, sign-in, replay rejection, bad signature rejection, update, and deletion on disposable databases. Fizzy's signed stateless WebAuthn challenge format and Rizzy's stored challenge format still differ, and full Rails CSRF behavior outside this route remains unverified.

The paired request-forgery regression now again matches the source's exact static 422 HTML: Rizzy had inserted a random CSP nonce meta tag into that error page. The mounted Action Mailbox test caught the same error-page mutation in its gated conductor POST and passes again after static policy handling.

| Area | Initial implementation | Full parity |
| --- | --- | --- |
| Account scoped routing, bearer tokens, identity, user listing and settings | ✓ | No |
| Boards CRUD and access control | JSON CRUD, browser new/edit forms, access records, and involvement | No |
| Columns CRUD and card listings | Named and special board column listings, position swaps, and public pages | No |
| Cards CRUD and card numbering | JSON CRUD, browser draft creation/resumption and publication, explicit timestamps, and account numbering | No |
| Closure, triage, postpone, gold, board move | ✓ | No |
| Tags, steps, comments | ✓ | No |
| Full board and card HTML interface | Rust-rendered board columns with immediate Maybe cards and lazy loading for other columns, board-level filtered card grid and pagination, all-cards browser grid with automatic preview pages, drag and drop, draft navigation, and column creation; published card page with rich description, steps, comments, edit, comment, watch, pin, column selection, board picker, and completion controls; Lexxy draft and card editors | No |
| Sign up, magic links, sessions, passkeys | Magic links, sessions, browser email/code entry, signup/completion forms, device transfer links with QR codes, passkey registration/management/sign-in, and initial Playground data | No |
| Users, roles, invitations, board involvement | JSON users, browser profile and name edit, roles, join links, email changes, verification, and name entry | No |
| Mobile client configuration and personal timezone | Parameterized Android/iOS JSON route and timezone updates | No |
| Assignments, reactions, pins, watches | Assignee and self-assignment toggles, reactions, pin/watch JSON mutations, a source-shaped pinned-card HTML page and Home tray, cacheable HTML frames, and Turbo replacement streams | No |
| Card filters, tag listing, basic search | Saved IDs, terms, dates, closures, activity spikes, FTS5 index, and selected MySQL FULLTEXT queries on this branch | No |
| Activities, core event side effects, and browser day timeline | Activity JSON, account home, filtered Added/Updated/Done days, timezone grouping, day pagination, column pages, Home add-card board selection, inline board/person filter controls, action-specific event sentences, common event media previews, and footer Search/Pins/Notifications controls | No |
| Saved filters and full text search | Plain-text FTS5 index for cards/comments; 16 MySQL search shards with selected cross-app checks on this branch; saved filter persistence, Turbo toggles, and settings refresh | No |
| Board publication and basic public pages | ✓ | No |
| Public board workflow and webhooks | Public pages; webhook browser management, CRUD, event queue, and generic, Slack, Campfire, and Basecamp delivery formats | No |
| Notifications, mail, push, jobs | Event and User attachment mention notifications, read controls and Home notification tray, access cleanup, VAPID push queue/delivery, timed multipart text/HTML email bundles, and expiring unsubscribe links | No |
| Attachments and Action Text rich text | Card images, avatars, image and file embeds in card/comment text, common formatted text, signed limit/fill image representation routes, and a storage entry/total ledger | No |
| Imports, exports, account lifecycle | Asynchronous user and account ZIPs; account import upload/status, transactional record restoration, file bytes, and expiry for represented records; owner cancellation and 30-day incineration | No |
| API response shape and pagination parity | Partial | No |
| SQLite and MySQL deployment parity | SQLite runtime; MySQL 8.4.11 core, search, notification, webhook, account import/cancellation, device transfer, deletion cleanup, partial schema recovery, and two-process write and worker checks on this branch | No |
| Comparative performance and scale proof | Scoped reads plus 30- and 60-second mixed API runs through 256 clients | No |

`tests/account_export_field_inventory.py --strict` builds a fresh board/card
fixture with a text-file embed on both SQLite apps and compares exported record
groups, field names, counts, blob metadata, and attachment ownership. Those
match for the fixture, including the User `attachable_sgid`
field. The test also passes each Rizzy User SGID to Fizzy's signed GlobalID
parser with a shared signing secret and checks the decoded User ID. Rizzy's
mention picker now emits this format; `tests/mentions.py` checks that the signed
IDs render and create mentions and notifications. The export fixture writes a
signed User attachment to each card, explicitly runs Fizzy's queued mention
job, and checks that both archives contain its portable User GID and one
mention. New direct, multipart, and imported blobs use Rails-compatible signed
IDs. `tests/blob_signed_id_reference.py` verifies a direct upload's blob ID and
Action Text SGID with Fizzy's verifiers, then embeds the file in Rizzy. Legacy
opaque blob IDs, import round trips with signed User mentions, and unrepresented
record groups remain open. `tests/account_export_field_inventory.py
--image-variant --strict` and its `--image-format` option now compare JPEG,
PNG, animated GIF, and animated WebP embeds. On these four fixtures, both
archives contain the same blob and attachment counts, preview bytes and
metadata, and Rails variation digests. Rizzy persists the derived blobs and
removes them with their original. It uses libvips `thumbnail_image` for these
formats when available, including every GIF and WebP frame. Other image
formats, broader image dimensions, and preview processing throughput have not
been compared.
The strict export fixture now also covers SVG and BMP embeds. Both apps exported
the same blob metadata and attachment records for each, with no derived
variants. The paired MySQL embed flow compares their file-attachment markup,
downloaded bytes, and MIME types: SVG is served as
`application/octet-stream`, and BMP as `image/bmp`. TIFF, AVIF, HEIC, HEIF,
and other media remain unverified.

The [JPEG embed write run](bench/results/image-embed-write-jpeg-40-2026-09-27.json)
timed 40 alternating, single-client card updates after preparing distinct
uploaded images and cards on fresh paired SQLite apps. Both ended with 86
variant records across the 43 writes, including warmups. Rizzy's median was
44.1 ms and p95 48.1 ms versus Fizzy's 52.2 ms and 69.6 ms on the same four
server CPUs. This measures synchronous JPEG embed writes only; concurrent
uploads, the other formats, and maximum sustainable throughput remain open.
`tests/storage_variant_concurrency.py` passed ten fresh runs with eight
simultaneous attachments and deletions of one image embed routed across two
Rizzy processes. Each run kept one pair of derived variants, queued one purge
after the last deletion, and removed original and variant bytes with two
worker processes available. The equivalent MySQL test passed eight runs.
These verify the shared-blob lifecycle under local concurrent requests; they
are not throughput or multi-host capacity measurements.

The [MySQL port status](docs/mysql-port.md) records the tested core workflow,
the still-missing MySQL runtime work, and the database comparison needed
before deployment parity or a greater-scale claim can be made.
The disposable paired MySQL harness also passed board views, board settings,
webhook management, step detail/edit, and anonymous public board/card workflows
against pinned Fizzy. Each covers its named fixture; the full route inventory
and exact browser rendering remain unverified.

`tests/home_add_card_parity.py` compares the Home add-card action, its selected
or recently accessed board, draft redirect, conditional refresh after board
creation, and single/multiple-board filter headings against the pinned Fizzy
app on disposable SQLite databases. Browser checks also exercised immediate
filter submission, option search, Escape, and the mobile popover.
`tests/timeline_event_descriptions_parity.py` compares 14 rendered Home event
sentences across the source's timeline action set, including assignments,
board and collection moves, triage, comment, and automatic postponement.
`tests/timeline_event_media_parity.py` compares card background images, local
image and file embeds, and remote image/video previews in the Home event cards;
it also follows the local image and background URLs to verify the media bytes.
The Home ETag now includes the compiled event renderer and its embedded assets,
so a browser revalidates after a CSS or template change; this was verified by
reloading the same session across a rebuilt Rizzy process. The complete source
event layout and other media formats remain to be matched.
The [paired Home media load run](bench/results/home-media-html-98-2026-09-26.json)
checked 100 visible events and matching media counts on fresh fixtures, then
measured two reversed-order five-second rounds at eight and 32 clients.
Rizzy handled 6,375–7,786 requests/s versus Fizzy's 315–392, with zero
errors, but its warm-cached HTML was 87,740 bytes versus 216,774 bytes.
The differing markup and incomplete interactions preclude a one-for-one
performance claim.
The event renderer now follows Fizzy's event wrapper classes, related-card
hover attributes, action-specific icon mapping, nested avatar and description
row, and excerpt-only-for-publish-or-comment layout. The source event SVG icons
are served as local assets. The paired media test checks the markup and icon
bytes against the pinned source; 14 event sentences and the Home add-card flow
still pass. Chromium showed populated Added and Updated columns, the comment
icon and media previews, and no page errors. The
[post-layout Home run](bench/results/home-media-html-layout-98-2026-09-26.json)
found 145,888-byte Rizzy HTML versus 216,774-byte Fizzy HTML and 6,894–7,842
versus 316–411 requests/s over the two short concurrency levels. Other source
layout details, media forms, and browser interactions remain incomplete.
Chromium comparison of fresh media boards exposed a day-boundary gap: Fizzy's
public sign-in page sets the browser timezone cookie, while Rizzy had waited
for one to exist. Rizzy's full-page layout now sets the same cookie; a new
browser session reported `America/Chicago` and rendered `ADDED TODAY` instead
of the next UTC date. Home now uses Fizzy's shared-hour CSS grid and column
dividers, source-shaped day headings and grid maximize icon, and a centered
two-row header. In Chromium, Added and Updated time blocks occupied the same
hour row and the page had no console errors. The
[post-grid Home run](bench/results/home-media-html-day-grid-98-2026-09-26.json)
showed 148,727-byte Rust versus 216,774-byte source HTML and 6,500–7,679
versus 308–403 requests/s across 8 and 32 clients, with zero HTTP errors.
Remaining header/menu behavior and many visual details still differ. Browser timezone preference persistence
and changes during a live session remain to be compared end to end.

A fresh three-card 1280 × 800 Chromium comparison captured
[Fizzy Home](docs/visual-baselines/fizzy-home-1280.png) and
[Rizzy Home](docs/visual-baselines/rizzy-home-1280.png). Rizzy now uses the
source desktop font scale, event spacing and text dimensions, centered time
label and empty state, and source-sized header and footer controls. Measured
grid heights were 477.8 px in Fizzy and 478.9 px in Rizzy; the first card was
98.86 and 98.89 px tall. The browser rendered a local event time on the first
visit despite the initial UTC response and reported no page errors. Colors,
avatar treatment, exact event icon artwork, and other screen states
still need parity work. The existing paired Home media and add-card checks pass.

The Home account trigger now opens a nonmodal menu over the page. A fresh
1280 × 800 Chromium comparison captured [Fizzy's menu](docs/visual-baselines/fizzy-home-menu-1280.png)
and [Rizzy's menu](docs/visual-baselines/rizzy-home-menu-1280.png). The dialog
has the source dimensions, search field, quick links, grouped navigation,
source SVG icons, close control, keyboard jump, and sign-out action. Browser
checks covered filtering, arrow navigation, `J`, Escape, outside click, direct
menu close, and direct menu sign-out without page errors. The paired
`tests/my_menu_parity.py` checks content, conditional caching, board and saved
view invalidation, and JSON rejection; `tests/home_add_card_parity.py` and
`tests/csp_configuration_parity.py` still pass. A `304 Not Modified` response
now leaves the cached page's Content Security Policy nonce intact, which keeps
its scripts running after revalidation. The menu's colophon, exact vertical
spacing, and all conditional sections and navigation states remain incomplete.

The card Back link now preserves the exact filtered board, all-cards, or Home
URL used to open it, including query parameters and a slashless account Home.
It uses the source page's label for the return text and falls back to the
card's board after a direct visit from an unrelated page. Escape and Left Arrow
follow the same link. In disposable browsers, Fizzy returned to Home, a
filtered board, and a filtered one-board all-cards view; Rizzy matched those
links and labels. A direct Rust card visit after account settings kept the
board fallback despite an older saved filtered URL. `tests/home_add_card_parity.py`
now also checks that both `/1` and `/1/` render Home in both apps. Board
columns can now receive focus, select their first card, move selection with
Up and Down, and open the selected card with Enter. A browser check of the
three cases in Fizzy's `board_keyboard_focus_test.rb` confirmed that Escape
returns focus to the opened card in named and Maybe columns; moving that card
to a collapsed column while its detail page is open leaves the previously
expanded column alone and selects no card on return. A fresh paired
`tests/board_filter_html_parity.py` run still passed. Navigation through Turbo
transitions and other board keyboard shortcuts remain incomplete.

The Home footer now opens source-shaped Pins and Notifications trays and an
inline Search bar. The Pins HTML response contains rich card previews and
preserves the separate JSON limit; `tests/pins_html_parity.py` compares its
items, ETag revalidation, card-edit invalidation, and unpin result against the
pinned source. Chromium exercised a pin, unpin, three search hits, and an empty
notification tray without console errors. Search also follows a numeric card
query to the card. The
[footer Home repeat](bench/results/home-media-html-footer-98-2026-09-26.json)
checked the same 100 events and selected media counts on fresh fixtures, with
zero HTTP errors in two reversed-order five-second rounds. Rizzy served
5,626–7,460 requests/s versus Fizzy's 319–414 at 8 and 32 clients, while
rendering 160,643 versus 216,774 HTML bytes. Live Turbo updates, complete
keyboard behavior, and exact tray/card styling still need cross-app verification.
This read-only run cannot establish
whole-app speed or greater maximum scale.

`tests/notification_tray_html_parity.py` now creates the same watched-board
notification on both apps, compares populated tray structure, and follows
individual read, include-read, unread, and bulk-read transitions. The shared
Rust notification card now renders the source's metadata groups, avatar,
body, and mention title treatment in the Home tray and full notifications
page. Chromium found the populated card and successfully used Mark as read and
Clear all. The Home footer fetches notification state when it loads and on
window focus, marks unread status on its toggle, and closes after Clear all.
Source Action Cable broadcasts and the remaining visual and keyboard details
are still open.
The [latest Home read](bench/results/home-media-html-notification-tray-98-2026-09-26.json)
and [same-binary repeat](bench/results/home-media-html-notification-tray-repeat-98-2026-09-26.json)
still favored Rizzy in all 16 error-free phases, but both targets slowed
substantially between runs. The measured page sizes were 162,819 and 216,774
bytes. These short local reads do not establish whole-app speed or maximum
capacity; the host's rate drift makes absolute throughput comparisons less
reliable.

`tests/storage_ledger_parity.py` compares Fizzy and Rizzy ledger operations for
card images, board/card/comment embeds, a board transfer, and card deletion.
`tests/storage_materialization.py` checks concurrent materialization, exact
account and board totals, idempotency, board cleanup, and reconciliation of an
attachment that predates the ledger. `reconcile-storage` scans existing
attachments and adds only the missing byte delta; `materialize-storage` drains
pending totals. Account import now writes per-board reconciliation entries from
its restored attachments and materializes the account total before completion;
`tests/account_import.py` checks those values against the ZIP.
`tests/mysql_storage_materialization.py` now checks the same storage ledger on
a fresh MySQL 8.4.11 database, including Card and Comment transfer entries,
two concurrent totals workers, missing-entry reconciliation, 16 concurrent
embed writes, and file cleanup. A MySQL `SUM(byte_size)` decimal mapping caused
the initial image-bearing board move to return 500; casting it to a signed
integer fixed the route. A fresh paired SQLite ledger run still matched Fizzy.
`tests/storage_embed_update_parity.py` compares both apps through empty, null,
plain-text, and replacement updates of card and board embeds; card detail,
list, activity, and ledger states match in that fixture. Fizzy retains an embed
when rich text becomes empty or null and detaches it on a nonempty edit without
the embed. `tests/embed_parity.py` also matches rendered Action Text after
Rizzy began using blob analysis metadata, rather than locally detected image
dimensions, for the immediate HTML response. Periodic reconciliation, other
Action Text transitions, and the remaining MySQL rich-text paths remain open.
`tests/storage_request_id_parity.py` now checks both
apps' generated, sanitized, and truncated `X-Request-Id` response headers,
including a 404, and matches request IDs on attachment, transfer, and deletion
ledger entries. Background-job request context and untested request paths
remain open. The MySQL schema now permits the source's 255-character request
IDs and startup widens older columns. The paired check also covers a stored
255-character ID, replacement entries, and board deletion. A fresh schema
regeneration matches `schema.mysql.sql` exactly. On disposable MySQL 8.4.11,
`tests/mysql_schema_upgrade.py` verified both the older request ID column
upgrade and restoration of the `card_bulk_touches` foreign key.

The [post-storage-ledger scoped read run](bench/results/embedded-ledger-read-100-4workers-8-32-5s-2026-09-26.json)
measured the new attachment lookup on four JSON read routes using fresh
100-card SQLite fixtures, four assigned app CPUs, four Fizzy workers, one
Rizzy process, and two reversed-order five-second rounds at 8 and 32 clients.
Sampled fields and shapes matched, the first card page was 23,974 bytes in
both apps, and all phases had zero errors. Rizzy served 8,746–8,876 requests/s
versus Fizzy's 371–440 on that fixture. This is a scoped local read result;
the fixture contains no embeds and cannot establish whole-app speed or
maximum scale.

On the MySQL branch, `tests/mysql_search_parity.py` compared the pinned Fizzy
MySQL adapter with Rizzy on a shared Oracle MySQL 8.4.11 server. Selected JSON
search results, card text filters, browser hit counts, stemming, edits, and
deletions matched. The source MySQL search suites passed 37 tests and 146
assertions. A fresh `tests/mysql_pair_selfhost.py search` run used disposable
databases for both apps and matched 86 JSON query cases and 10 rendered search
cases. Its fixture adds accented text, punctuation and quote combinations, and
a card and comment that both highlight the same title. This is evidence for
those fixtures, not full ActiveSearch parity; all browser snippets, Unicode
indexing, and search scale remain open.

The source JSON search applies `Card.latest`, which sorts by last activity. A paired SQLite test now creates two matching cards, comments on the older card, and confirms both apps move it ahead in JSON search. Rizzy's MySQL JSON query previously sorted by creation time; it now uses the same last-activity order, and `tests/mysql_search_parity.py` includes the matching two-card case. The fresh disposable MySQL comparison ran this case and matched both apps.

`tests/mysql_pair_selfhost.py core` also ran the 142-comparison core JSON
workflow on fresh Fizzy and Rizzy MySQL databases with zero differences. Its
scope is the workflow described below; it does not prove parity for every route
or browser page.

`tests/mysql_pair_selfhost.py board-filter` found that MySQL card-preview
fragments returned 500 when a step aggregate was decoded as a decimal by SQLx.
Rizzy now casts the aggregate to an integer on MySQL. Fresh paired MySQL and
SQLite runs passed the board-filter browser fixture, including a two-step card
with one completed step, filtered pagination, people filters, and empty results.
The fixture compares seven workflow summaries and additional per-page
assertions; exact board markup and other browser flows remain open.

The disposable MySQL harness also passed `all-cards`, `named-column`,
`column-cache`, and `column-show-cache` against pinned Fizzy. These fixtures
exercise all-cards HTML filtering and two-page previews, the named-column
empty and two-page HTML states, JSON page validators after card edits, and
index/show validators after card triage and column rename. Fizzy's MySQL
multiword term filter matched more cards than its SQLite counterpart, so the
all-cards fixture compares the actual source and Rust result sets on each
backend. The existing SQLite fixtures passed after the harness changes.
These cases do not establish exact browser markup, interaction, or all-route
cache invalidation parity.

The same disposable MySQL harness passed `card-controls`, `card-board`, and
`card-edit` against pinned Fizzy. The fixtures check pin and watch frames,
conditional ETags, Turbo replacement streams and JSON toggles; the board
picker, cached response, browser and JSON moves; and the edit form, Lexxy
editor, JSON rejection, save and reload. They cover selected transitions, not
the complete card UI, exact markup, or every permission and side effect.
Paired MySQL `assignment` and `reaction` runs also passed the browser pickers,
conditional responses, card metadata and Turbo changes, self-assignment,
card/comment reaction forms, and selected JSON mutations. The `column-turbo`
run passed form creation, left/right reordering, rename, deletion redirect,
and unsupported-format side effects; stream action sequences matched after
per-app generated IDs were excluded. The SQLite column fixture still passes.
These are selected paths, not complete interaction or permission parity.
The paired `card-previews` run matched the 15-card first and two-card second
Turbo preview pages after new cards arrived; `account-card` matched JSON
creation and the browser draft redirect. A legacy MySQL database with board,
card, and comment text but no Action Text identity rows now backfills those
three rows on startup. `tests/mysql_rich_text_backfill.py` checks the result and
repeat initialization, and the paired MySQL embed flow still passes. This is
upgrade coverage for those record types, not full archive or attachment parity.

`tests/join.py` and `tests/email_change.py` now run their full existing Rust
flows on MySQL as well as SQLite. The MySQL invite test includes a two-process
redemption race, member access changes, reset, and restart after a deleted join
code. The MySQL core flow also checks an assigned card move into a private
board and draining queued push jobs without a VAPID key. The local MySQL mixed
API benchmark is recorded in [MySQL port status](docs/mysql-port.md); it covers
one workflow and does not prove full application speed or maximum scale.
`tests/mysql_passkeys.py` also checks a complete passkey registration and
authentication flow on MySQL, including two simultaneous submissions of one
challenge with exactly one successful login. The pinned source's selected
MySQL passkey suites passed 19 tests and 64 assertions. Browser authenticators
and alternate deployment origins are still unverified.

The four-process MySQL mixed-workload probe exposed a Rizzy board-lock upgrade
deadlock on concurrent card updates. Card, event, closure, and reading writes
now lock the parent board before child rows that reach the board-touch trigger.
The 500-card, 32-client rerun completed two 20-second Rust phases without HTTP
errors or count mismatches at 192.03 and 194.50 workflows/s, split nearly
evenly across four processes. Fizzy completed one comparable phase at 24.32/s;
its other phase failed in its ActiveSearch shard. The raw result and limits are
in [MySQL port status](docs/mysql-port.md). This is evidence for one local API
workload, not complete MySQL parity or a multi-host scale result.

A direct one-versus-four Rizzy MySQL comparison used the same release binary,
four app CPUs, and two reversed-order 20-second rounds at 32 clients. With 500
cards on one board, four processes handled 184.74–191.65 workflows/s versus
164.70–165.74 for one, a 12–16% gain. With the same 500 cards spread evenly
over four boards, four handled 279.88–285.37 versus 198.12–199.99, a 41–43%
gain. Every phase had zero HTTP errors and exact stored counts; work was near
even across boards and processes. This verifies one-host process scaling for
this API workload. Multi-host operation, other routes, and full source parity
remain open.

The [1,000-card four-board MySQL rerun](bench/results/mysql-cluster-1000-4boards-c32-30s-2026-09-26.json)
completed two reversed-order 30-second rounds at 32 clients. Four Rizzy
processes handled 241–255 workflows/s versus 191–193 for one, a 26–32% gain,
with zero HTTP errors and exact stored counts. In the corresponding
[Fizzy/Rizzy run](bench/results/mysql-mixed-1000-c32-30s-2026-09-26.json),
one clean pair measured 135.26 Rizzy workflows/s versus Fizzy's 21.11; the
other Fizzy phase failed once in its ActiveSearch shard and cannot support a
clean paired ratio. These are scoped local API measurements, not maximum
capacity or whole-app results.
A [16-client repeat](bench/results/mysql-mixed-1000-c16-30s-2026-09-26.json)
also had one clean pair, 138.86 Rizzy versus 24.37 Fizzy workflows/s, and one
Fizzy ActiveSearch deadlock phase.
The [eight-client repeat](bench/results/mysql-mixed-1000-c8-30s-2026-09-26.json)
again had one clean pair, 139.10 versus 24.08 workflows/s, and one source
ActiveSearch deadlock; the larger fixture still lacks a clean two-round pair.
The new four-board option for `bench/sustained_mixed_mysql.py` distributes the
same 1,000 cards and mixed JSON workflow over four boards. A [current
16-client comparison](bench/results/mysql-mixed-1000-4boards-4process-c16-30s-2026-09-27.json)
used four Fizzy Puma workers or four Rizzy processes on the same four app CPUs
and MySQL server, in two reversed-order 30-second rounds. Sampled responses
matched before load; all four phases had zero HTTP errors and exact final
card, event, and comment counts. Rizzy completed 251.94 and 225.40 workflow
units/s versus Fizzy's 23.07 and 24.49, paired gains of 10.9 and 9.2 times.
Work was near evenly distributed across boards and Rizzy processes. The
[32-client extension](bench/results/mysql-mixed-1000-4boards-4process-c32-30s-2026-09-27.json)
had one clean pair at 252.31 versus 26.87 units/s; Rizzy completed its other
phase at 251.47 without errors, while Fizzy failed a card PATCH with an
ActiveSearch MySQL deadlock, so that second pair is invalid for throughput
comparison. This is a local API workload with partial feature parity, not a
whole-app, multi-host, or maximum-capacity result.
The [64-client four-board extension](bench/results/mysql-mixed-1000-4boards-4process-c64-20s-2026-09-27.json)
used the same pinned source and committed Rizzy binary in two reversed-order
20-second rounds. Rizzy completed 261.16 and 267.47 workflow units/s with zero
HTTP errors and verified counts. Fizzy returned one card-PATCH HTTP 500 in
each round; both logs identify an ActiveSearch MySQL deadlock. Its interrupted
phases cannot be used for a paired speed ratio. They show a reliability limit
for this source fixture, while Rizzy completed the tested load.
The [current one-versus-four Rizzy MySQL run](bench/results/mysql-rizzy-1v4-1000-4boards-c32-30s-2026-09-27.json)
used the same committed release binary, four app CPUs, 32 clients, 1,000
cards on four boards, and two reversed-order 30-second rounds. Four processes
completed 250.72 and 248.86 workflow units/s versus 172.28 and 174.04 for
one, gains of 45.5% and 43.0%. All phases had zero HTTP errors and exact
stored counts, with work near even across boards and processes. Four processes
used about 50 MiB end-of-phase app PSS versus 14–15 MiB for one, and averaged
1.63–1.65 app CPU cores versus 0.70–0.71. This establishes process scaling
for this local API workload; it does not establish a maximum capacity or
multi-host operation.
At [64 clients](bench/results/mysql-rizzy-1v4-1000-4boards-c64-20s-2026-09-27.json),
the same release binary and four app CPUs completed two reversed-order
20-second rounds. Four processes handled 262.36 and 257.73 units/s versus
175.46 and 166.43 for one, gains of 49.5% and 54.9%. All four phases had
zero HTTP errors and exact card, event, and comment counts. This extends
one-host process-scaling evidence to the tested 64-client workload; it does
not establish maximum sustainable concurrency or multi-host scale.
The [matched-fixture read-only MySQL comparison](bench/results/mysql-read-matched-fixtures-1000-8-32-128-15s-2026-09-26.json)
did complete two reversed-order rounds at 8, 32, and 128 clients with zero
errors, unchanged counts, and matched complete sampled JSON values after
masking generated IDs, timestamps, and server addresses. Rizzy served
3,251–4,117 requests/s versus Fizzy's 320–350 on board, card, scoped-card,
and scoped-activity GETs. Both used the same account number and owner details;
Rizzy's serialized bodies were about 1% smaller. The result covers only those
four read routes on one host.

A same-binary [four-process MySQL read run](bench/results/mysql-read-matched-fixtures-4process-1000-8-32-128-15s-2026-09-26.json) and [one-process control](bench/results/mysql-read-matched-fixtures-1process-1000-8-32-128-15s-2026-09-26.json) used 1,000-card fresh fixtures, the same four server CPUs, and two reversed-order 15-second rounds per client level. The load client distributed every route evenly over four Rust processes, checked their sampled JSON against Fizzy, and found zero HTTP errors and unchanged counts in all phases. Four-process Rizzy served 3,372–4,605 requests/s versus four-worker Fizzy's 290–312, or 11.4–15.6× in paired levels. Against the one-process Rizzy control with the identical binary, four processes averaged 2% more at 8 clients, 24% more at 32, and 30% more at 128. At 128 clients four processes had higher p95 latency (70–85 ms versus 45–54 ms) and used more CPU and memory. This verifies local process throughput scaling for four JSON reads; it does not verify multi-host deployment, write scaling, untested routes, or whole-app parity.

The expanded `tests/mysql_cluster.py` reproduced HTTP 500 database errors when
step creation, unwatching, triage, or reaction creation raced a title edit on
the same board through different MySQL-backed processes. Those mutations now
lock the board before touching child rows; reaction creation also commits its
insert and card touch together. Four full reruns passed the 400-card step CRUD
phases and 200 paired requests in each of four mixed phases across four app
processes. The MySQL core flow and SQLite step and watch regressions also pass.
Other card mutations and multi-host writes still need the same audit.

The expanded MySQL cluster test also found InnoDB 1213 deadlocks on card
move/title races. Moves now lock both boards in ID order. Event creation
rechecks a locked card's board and retries when it changed after the caller's
read, so title events use the destination board after a move. Six fresh-account
runs passed 200 move/title pairs and 200 opposite-direction moves across four
processes; the MySQL core and SQLite card workflow regressions passed. Same-board
draft publication separately reproduced MySQL 1213 deadlocks across two
processes. Publication now locks its board first; six 32-draft runs passed,
including a check for one publication event per card. Other callbacks and
multi-host writes remain unverified.

The assignment/title MySQL race then reproduced a 1213 deadlock between an
activity-spike card update and a title edit. Event and activity-spike writes
now share a board-first card lock with a board recheck after moves. Eleven
fresh-account cluster runs passed 200 assignment/title pairs; six additional
runs passed 200 tag/title pairs against one shared tag, with sampled card state
checks. MySQL core and the two-process SQLite card workflow passed. Other
callbacks and multi-host scale remain open.

`tests/tag_touch_parity.py` compares live Fizzy and Rizzy tag-on/tag-off
requests on disposable SQLite databases. Fizzy advances the card and board
update times on both toggles while leaving `last_active_at` unchanged; Rizzy
now matches. Tag membership and the card touch are committed together.
`tests/mysql_cluster.py` also ran 200 pairs of simultaneous toggles on already
tagged cards through four processes and verified every tag remained present
after two toggles. Five fresh-account runs and the MySQL core flow passed.
This covers the tested toggle and timestamp behavior, not every tag UI or
concurrent source behavior.

The [60-second mixed API benchmark](bench/results/sustained-mixed-100-2026-09-26.json) uses fresh source and Rust databases, 100 initial cards, 32 clients, the same eight logical server CPUs, and two reversed-order rounds. Each completed unit makes five JSON reads and four card writes. With board JSON and account ETag state included, Rizzy completed 90.99–91.10 units/s versus Fizzy's 25.61–28.03, with no HTTP errors and matching final card, event, and comment formulas. The artifact records the pinned Fizzy revision, Rizzy commit, and release binary hash. It compared sampled response shapes, content, counts, and pagination headers before timing. This verifies a faster result for this workload, not one-for-one application speed or maximum scale: the apps have different process models and unimplemented browser and background behaviors, and SQLite writes serialize. `tests/board_show_pagination_parity.py` checks signed-in board pages one and two, their separate ETags, and 304 responses against disposable Fizzy and Rizzy databases.

The [1,000-card, 64-client](bench/results/sustained-mixed-1000-64-2026-09-26.json), [128-client](bench/results/sustained-mixed-1000-128-2026-09-26.json), and [256-client](bench/results/sustained-mixed-1000-256-2026-09-26.json) mixed API runs used the same eight assigned server CPUs and two reversed-order 30-second rounds each. Across these six pairs, Rizzy completed 118.51–126.62 units/s versus Fizzy's 22.83–27.08, with zero HTTP errors and correct final record formulas in every run. At 256 clients both setups were near a local throughput plateau while p95 unit latency rose to 3.68–3.71 seconds for Rizzy and 10.43–12.59 for Fizzy. This is a larger-fixture, higher-concurrency comparison for one API workflow; it does not establish multi-host scale, a hard capacity limit, or complete application parity.

The [four-core repeat](bench/results/sustained-mixed-1000-64-4cpus-2026-09-27.json) ran that 1,000-card, 64-client mixed JSON workflow for two reversed-order 30-second rounds with both servers restricted to the same four logical CPUs and the load generator on other CPUs. Rizzy completed 140.99–141.13 units/s versus Fizzy's 20.26–20.50, a 6.88–6.97× throughput ratio, with no HTTP errors and correct final card, event, and comment counts. P95 unit latency was 805–812 ms for Rizzy versus 3,969–4,920 ms for Fizzy. Rizzy averaged 3.62–3.65 CPU cores while Fizzy averaged 3.01–3.05; end-of-run proportional memory was 85–86 MiB for one Rizzy process versus 1,730–1,764 MiB for the four-worker Fizzy process tree. The harness checked response shapes and selected content before timing, plus status and basic type during reads, rather than byte-equivalent output on every timed request. This strengthens the local result under equal CPU affinity but does not establish full application or multi-host scale parity.

The [100-card rich-preview HTML run](bench/results/cards-html-rich-preview-media-100-2026-09-26.json) exercises the current avatar/background-capable preview build with 15 visible cards, each with a column, tag, completed step, comment, reaction, and assignee. Fresh Fizzy and Rizzy databases and the same eight assigned server CPUs produced zero HTTP errors in five-second 1-, 8-, and 32-client runs. Rizzy served 1,864–2,486 requests/s versus Fizzy's 79–192. The checked samples matched visible card count and selected rich-preview data, but differed greatly in response size (48,482 versus 161,442 bytes) and still lack full HTML/interaction parity. This measures the current browser pages and cannot establish one-for-one application speed or greater multi-host scale.

`tests/board_show_pagination_parity.py` compares board detail JSON across two pages and three filter cases on fresh databases. Fizzy's filtered board response counts the matching cards even though its JSON body describes the board; Rizzy applies the same filter to `X-Total-Count` and its next-page link. The source's unfiltered board ETag changes by page, after a card leaves the board, and after an account rename; the Rust response now matches those decisions. A repeated account name leaves the validator stable in both apps. The test covers ordinary, Done, and text filters, not every saved filter, card state, or concurrent write race.

Accounts now retain an update time. `tests/account_timestamp_migration.py` checks backfill from creation time on an old database, a changed-name touch, unchanged-name, counter, and entropy no-ops, insertion, and restart. A disposable Fizzy probe confirmed that card numbering does not touch its account while a changed name does. `tests/account_transfer.py` checks that the saved timestamp appears in the account ZIP, and `tests/account_import.py` checks a fresh imported timestamp and re-export, including a source account named `Import` whose card count changes. Historical update times missing from old Rizzy databases cannot be recovered, so migration uses creation time. Other account callbacks and validator inputs still need comparison.

`tests/activity_event_board_parity.py` checks the activities API with an event whose board differs from its card's board and a second event with an unsupported action. Fizzy scopes access, board filters, and the top-level board field to the event board while the nested card retains its own board; it excludes the unsupported action. A member with access only to the event board sees the activity, while access only to the card board does not expose it. Rizzy now matches those results. The fixture changes event and access rows directly on disposable databases to expose the distinction; ordinary board moves and all action descriptions still need broader comparison.

`tests/activity_pagination_parity.py` inserts 197 timestamped events on equivalent disposable boards and compares every JSON page on both apps: 15, 30, 50, 100, 2, and 0 records, all 197 IDs unique and reverse chronological, matching total-count and next-page headers. Fizzy's GearedPagination sends a next link again on the out-of-range empty sixth page; Rizzy's shared JSON pagination helper now reproduces that behavior. This verifies one board-filtered traversal, not every query parameter or route that uses the helper.

`tests/activity_particulars_parity.py` compares JSON descriptions and normalized particulars for all 12 allowed activity actions plus self-assignment and self-unassignment. It inserts source-style nested particulars and two historical assignees on disposable boards, including names, card titles, and action fields with HTML-sensitive characters. Rizzy now returns Fizzy's action-specific particulars, loads assignees named by old events even when they are no longer assigned to the card, and matches the source descriptions for these 14 cases. This direct-row fixture verifies serialization, not every callback or notification produced by the underlying actions.

`tests/activity_filters_parity.py` creates two boards with owner and second-creator events in each app, then compares single and multiple board filters, single and multiple creator filters, combined filters, and scalar forms of both query parameters. The scalar cases exposed that Rizzy ignored `board_ids` and `creator_ids` without brackets; it now accepts those Fizzy-supported forms. This covers the tested query values and visible records, not all malformed parameter shapes or permissions.

`tests/activity_access_matrix_parity.py` compares the activities API for an owner and a member across two private boards, with owner- and member-created events, board and creator filters, and a change in the member's board access. Both apps returned the same visible event sets in seven cases. `tests/boards_index_order_parity.py` checks 200 boards with distinct recent-access times across five geared JSON pages; the ordered names, page sizes, total-count headers, next links, and sampled JSON shape matched. These fixtures insert selected event, board, and access rows directly and do not verify the callbacks those model creations would normally produce.

`tests/activity_envelope_parity.py` compares the nested card and comment JSON shapes for a published card and comment, checks their IDs and links, and compares stable rich-text HTML with Fizzy. It exposed an empty-description distinction: omitting the card description produces an empty HTML string, while explicitly supplying or clearing it to `""` leaves an Action Text wrapper. Rizzy now records that presence and renders both states in its card JSON, card collections, and activity payloads. `tests/rich_text_parity.py` checks both empty cases along with the existing nonempty rich-text examples. This covers the tested API representations, not every rich-text editor or attachment path.

`tests/board_empty_rich_text_parity.py` extends the omitted-versus-explicit-empty check to published board descriptions: direct board JSON, board lists, new cards, and activity JSON match Fizzy through empty updates, nonempty text, clearing, and explicit-empty creation. `tests/comment_empty_rich_text_parity.py` checks three comment-body states in direct, list, and activity JSON: an explicit empty string has an Action Text wrapper, JSON null has empty HTML, and a missing body yields 400 on create and update. It also checks transitions among nonempty, null, and explicit empty bodies, plus missing and timestamp-only form submissions. Fizzy's cached nested board JSON on an older card can retain a prior description after the board changes; the board fixture uses a new card at each state and does not claim parity for that cache behavior.

`tests/board_update_noop_parity.py` compares live Fizzy and Rizzy board timestamps, access counts, entropy records, and Action Text state across fourteen repeated or changed updates. Rizzy now preserves the board timestamp for repeated name, access-mode, period, description, and member values; creating the first board entropy record for an unchanged period also leaves the board timestamp alone. Board description metadata distinguishes absent, explicit null, empty, and nonempty states, including explicit null creation. An empty board update now returns Fizzy's 400 JSON response. `tests/board_rich_text_state_migration.py` upgrades a database without the null-state column and preserves an existing explicit-empty record; existing databases cannot infer historical null rows that older Rizzy versions represented as absence. The existing board/comment rich-text and account import/export checks pass after this change. Other board callbacks and asynchronous access cleanup still require comparison.

`tests/null_rich_text_update_parity.py` compares card and published-board updates with JSON null and explicit empty strings. Null now clears the stored text and Action Text presence, whereas an explicit empty string retains the wrapper. Direct JSON, lists, and activity responses match Fizzy for the tested transitions; the board check uses a new card after each board update to avoid Fizzy's older-card fragment cache behavior.

The [500-workflow card write comparison](bench/results/card-workflow-500-2026-09-26.json) checks create, edit, comment, close, and read response shapes and final record/event counts on disposable accounts. Rizzy completed 418–457 workflows/s versus Fizzy's 28.5–32.1 across two eight-client rounds, and 432–442 versus 28.5–28.6 at 32 clients, without HTTP errors. The first concurrent attempt exposed intermittent SQLite lock errors in Rizzy; card edit, comment, and closure mutations now share its in-process write queue, and the recorded runs passed. The source database contained other accounts, the servers used different process models, and exact side effects and browser work still differ. The Rust samples lasted about one second, so these results do not prove sustained production capacity or full application parity.

`tests/cluster_card_workflow.py` found a further cross-process lock race in title-change system comments. The workflow's write transactions now reserve SQLite's writer lock at transaction start; ten fresh two-process runs of 600 workflows each passed with complete card, event, and comment counts. The [post-fix 500-workflow benchmark](bench/results/card-workflow-500-immediate-2026-09-26.json) retained zero HTTP errors and measured 412–450 Rizzy versus 27.5–30.4 Fizzy workflows/s at 8 and 32 clients. This establishes correctness for the tested two-process workload and speed for the local single-process Rust fixture, not higher multi-process write throughput or full scale parity.

The [1,000-workflow Rust cluster comparison](bench/results/cluster-card-workflow-2026-09-26.json) makes the scale limit explicit. At eight clients, one process handled 401–416 workflows/s while four handled 315–320; at 32 clients, one handled 395–420 while four handled 362–369, with complete records and zero HTTP errors. SQLite's serialized writer prevents this workflow from scaling out by adding app processes. MySQL support or another multi-writer storage path, parity of remaining side effects, and sustained multi-level load tests remain required before claiming greater application scale. The 60-second mixed run above covers one concurrency level on one host.

The [conditional board HTML benchmark](bench/results/board-html-conditional-2026-09-25.json) verifies matching 304 responses and shows faster Rust reads on its 100-card fixture through 64 concurrent clients. Rizzy now reuses its prior validator before rendering when a dedicated SQLite connection confirms that no database commit has occurred. `tests/board_html_cached_etag.py` checks 304 reuse and invalidation after a second process renames the board and creates a card. In the [same-host before/after run](bench/results/board-conditional-fastpath-2026-09-26.json), 100-card conditional reads rose from 2,379 to 4,691 requests/s at eight clients and from 1,924 to 4,635 at 32, with no HTTP errors; full-response throughput remained similar. The apps' full HTML responses still differ substantially in size and detail, and this local optimization does not close the full-parity or sustained-scale gates.

The first fidelity gate is to run each upstream API controller test against an equivalent Rust route and compare status, response fields, authorization, and database side effects. The second is visual and interaction testing for each original view. The third is benchmark comparison using parity checked responses and equal data volumes.

The [route registration inventory](docs/route-coverage.md) compares the pinned
source's dispatchable controller actions with Rizzy's path and method table:
all 211 application routes and three Turbo Native navigation routes are now
registered. The Turbo Native responses match the source's HTML text. Rizzy now
registers all nine Active Storage routes. Paired tests compare original blob
bytes, private/public access, single byte ranges, and disposition, plus signed
image representation redirects, proxy, alias, tampering, cache policy, and
cleanup. Rizzy cannot yet decode Rails variation keys, process every source
transformation, or reproduce libvips variant bytes. Fourteen Action Mailbox
framework routes remain unmatched. Route
registration alone does not prove behavioral parity for any of these areas.

`tests/static_csp_parity.py` compares all five static error pages and the
production 404 response byte-for-byte with Fizzy, including its locked-down
static Content Security Policy. It also compares the default dynamic policy
after normalizing the per-request nonce on public HTML, JSON, and a signed-in
account settings page. Rizzy gives its inline scripts that nonce; a browser
sign-in, board creation, and Search panel check completed without page errors.
`tests/csp_configuration_parity.py` also compares every documented source
override, a report URI, report-only mode, and disabled CSP on static and dynamic
responses. Source-specific `config.x` settings and other deployment
configurations remain unverified. The [CSP-enabled board HTML run](bench/results/board-html-csp-2026-09-26.json)
used fresh 100-card databases, four available server CPUs, two production
Fizzy workers, a release Rizzy process, and two reversed-order five-second
rounds at 8 and 32 clients. Rizzy served 5,219–5,843 requests/s versus
Fizzy's 77–89, with no HTTP errors and 15 visible cards in both samples.
The Rust page was about 50.5 KB versus 158 KB for Fizzy, and its visual and
interaction parity remains incomplete. This run confirms the current CSP
build remains faster on this fixture; it does not isolate CSP overhead or
support a product-wide performance claim.

Rizzy's card and comment FTS5 indexes now use the Porter tokenizer from Fizzy's pinned SQLite search schema. `tests/search.py` checks stemmed title and comment queries such as `running`/`run`, `authentication`/`authenticate`, and `walking`/`walk`. `tests/search_stemming_migration.py` downgrades a populated disposable index to the older tokenizer, restarts Rizzy, and verifies both existing index row counts and stemmed matches. The pinned Fizzy search model and card-search suites pass 16 tests and 65 assertions; its custom MySQL stemmer suite passes 25 tests and 50 assertions. Full query grammar, Unicode tokenization, relevance ordering, and the production MySQL adapter still need direct parity checks.

The pinned `api/flat_json_params_test.rb` suite passes 24 tests and 131 assertions. `tests/flat_json_params.py` exercises all 24 equivalent flat-JSON requests in Rizzy, including signup on a fresh account, and checks stored changes for representative records across boards, cards, columns, steps, reactions, notification preferences, join codes, account settings, users, tokens, webhooks, and push subscriptions. This establishes the tested flat-JSON parameter convention, not broader API response or permission parity.

Fizzy's `Card.sorted_by` scope orders equal-time oldest cards by ID ascending. `tests/source_card_tie_order_probe.rb` confirmed that behavior on 17 cards in a disposable copy of the pinned database. Rizzy now shares the same oldest/newest/latest tie order across all-cards JSON, filtered board HTML, and card preview streams. `tests/card_tie_order.py` checks the 15-card page boundary and the order of rendered titles on those surfaces. Other filter combinations and pagination edge cases remain open.

`tests/source_card_update_touch_probe.rb` observed five published-card update cases on a disposable Fizzy database: an empty edit and repeated title leave the card's `updated_at` and `last_active_at` unchanged but touch its board; description and image edits update `updated_at` without changing `last_active_at`; a title change adds one event and advances both card timestamps. `tests/card_update_touch.py` checks the same effects in Rizzy. Explicit `last_active_at` restoration and card timestamp inputs remain covered by `tests/card_timestamps.py`. Other callbacks and concurrent edits still need comparison.

[ROUTE_COVERAGE.md](ROUTE_COVERAGE.md) inventories source controller actions and Rizzy method/path declarations; all 209 normalized signatures are now present. Route presence does not establish functional parity. `tests/client_configuration_route_parity.py` compares the parameterized mobile route against fresh Fizzy and Rizzy servers in 24 root and account-scoped cases: versioned `.json` URLs, JSON negotiation without a suffix, query format selection, missing versions, invalid platforms, and HTML errors. Status, content type, cache, indexing header, and body matched. `tests/client_settings.py` also checks the supported configuration contents and personal timezone updates. Fizzy's three generated legacy `collections` redirects remain excluded from the inventory. A live comparison matched 34 GET and HEAD redirects across ordinary paths, `.json` suffixes, queries, trailing slashes, and an unknown numeric account. These route checks do not establish broader feature parity.

`tests/session_transfer_parity.py` compares profile link generation, replacement of an older link, public QR image rendering and decoded destination, one-year public QR caching, anonymous transfer pages, PUT/PATCH redemption, and replay rejection on both apps. Previously issued QR images now survive link replacement, redemption, and four-hour transfer expiry, as Fizzy's signed URL does. Rizzy signs the QR destination with a persistent local key; a previously issued image also rendered after a Rust process restart. Invalid signatures return Fizzy's 500 error by requested format, while an absent account prefix redirects to the session menu. It does not reproduce Rails' signed-token bytes or exact SVG encoding. The test checks simultaneous Rust redemption from two requests. The profile's broader layout and CSRF meta markup still differ.

`tests/passkey_parity.py` registers the same fixed-key WebAuthn test credential on both running apps, names it, verifies a signed assertion, checks challenge replay and bad-signature rejection, and deletes the credential. The paired MySQL `passkeys-flow` suite now runs this complete flow against fresh Fizzy and Rizzy databases through account-scoped management routes, including the post-registration message. The paired `passkeys-html` suite checks account-scoped index and profile links, unscoped redirect, JSON format rejection, and missing-passkey edit/update/delete responses. Rizzy stores one-use challenges and verifier state in the database, and asks browsers for resident credentials so the account-free sign-in picker can find them. The rendered scripts pass JavaScript syntax checks. This verifies a synthetic credential flow; hardware authenticator/browser interoperability and the full Fizzy passkey presentation still need testing.

`tests/pwa_parity.py` compares the authenticated service worker script against live Fizzy after normalizing its bundled asset URL, then checks Rizzy's manifest, icons, and local offline bundle. `tests/pwa_registration.py` checks that authenticated full HTML pages include the manifest and worker registration while anonymous sign-in does not, and that the menu keeps its conditional response. Chromium unregistered the worker, reloaded the account home, and confirmed registration and page control without console errors. The source's asset preloading, native bridge options, and installed-app behavior remain unverified.

`tests/push_subscription_parity.py` compares JSON creation, duplicate requests, forbidden endpoints, persisted key fields, and deletion against both apps. Rizzy also rejects DNS results outside public address ranges. A notification now adds a durable push job, and the worker generates Fizzy-shaped event or mention payloads, encrypts them with Web Push, signs them with VAPID, and sends to a freshly resolved, pinned public address when a subscription exists. It claims jobs atomically across workers, handles up to 50 jobs concurrently, records per-subscription success for retries, and deletes invalid or expired subscriptions. `tests/notification_bundle_mail.py` verifies real notification queue inserts and source changes; Rust unit tests check representative payload text, signed encryption, and invalid-key cleanup. The pinned Fizzy push model suites passed 31 tests and 36 assertions. Chromium loaded the settings control without console errors and registered the service worker, but its headless notification permission was denied, so a live browser subscription and provider delivery are still unverified. The settings page and excerpt edge cases remain simpler than Fizzy.

`tests/unsubscribe.py` checks anonymous new/confirmation pages, a valid POST changing the setting to `never`, invalid/expired/cross-account tokens, and pending bundle cleanup. The pinned source's unsubscribe controller suite passed 5 tests and 26 assertions. `tests/notification_bundle_mail.py` creates a real watched-board notification, checks four-hour/day/week windows, delivers a due multipart email through a test sendmail program, parses both MIME parts, follows its unsubscribe link, verifies a failed send retries, and runs two deliverers concurrently without duplicate messages. The pinned source's bundle, mailer, and settings suites passed 22 tests and 77 assertions. Rizzy now uses the source mail layout and renders grouped boards/cards, action text, and avatar images or initials. `tests/notification_bundle_batch.py` verifies that one manual run drains 205 due bundles across the former 100-row limit, including equal end times, then checks two concurrent processes claim all 205 exactly once. The timed worker now checks at the source's half-hour schedule with a shared database claim. Actual half-hour timing, edge-case event descriptions, all rich-text forms, timezone presentation across all locales, and full delivery parity remain incomplete.

`tests/mysql_notification_bundle_batch.py` passed the same 205-bundle,
two-process drain on a disposable MySQL 8.4.11 database.

`tests/notification_settings_html.py` follows the browser preferences path used in bundle mail, checks the email-frequency selector, submits the HTML form, verifies JSON updates and unsubscribe-token issuance, and checks both board involvement states through the settings page. Fizzy's settings controller suite passed 4 tests and 24 assertions on its HTML and JSON paths; its board involvement controller suite passed 2 tests and 14 assertions. Chromium clicked Rizzy's board bell in both directions, reloaded the persisted state, and reported no browser errors. Rizzy uses direct JavaScript updates instead of Fizzy's Turbo frame replacement, and its push-help panels and styling remain simpler.

Card reading now returns Fizzy's Turbo response shape: reading a card removes its notification from the tray, reading without a notification returns an empty stream, and marking a card unread returns 204. `tests/notifications.py` checks those responses and their stored read state alongside JSON behavior. The pinned Fizzy card-reading controller suite passed 6 tests and 41 assertions; an isolated source fixture confirmed the exact notification-removal stream, and a live no-notification Turbo POST and DELETE returned 200 and 204 respectively. The populated tray structure and read controls now have a paired check; source broadcasts and exact presentation remain open.

The notifications index and tray now return HTML for browser sessions and preserve their JSON routes. The browser index has unread and read lists, a 500-item unread cap, read pagination, notification controls, and the bulk-read form; individual read/unread actions return the source's remove-and-prepend Turbo stream targets. The tray now uses whole-collection ETags for HTML and JSON, returns 304 on a matching validator, and distinguishes omitted, false, and true `include_read` values as the live source does. `tests/notifications.py` checks rendered content, tray frame and ETag invalidation, pagination stream, individual transitions, bulk-read responses, and database state. The pinned source's index, tray, reading, and bulk-reading suites passed 14 tests and 92 assertions; an isolated source fixture confirmed the index markers and two-stream transitions. Chromium moved an unread notification to Previously seen, moved it back, and cleared it with Mark all as read without console errors. Rizzy uses its own simpler HTML, JavaScript stream application, and styling; source broadcasts, exact cached partials, and full visual parity remain incomplete.

`tests/notification_pagination_paired.py` now compares the source's fetch-on-visible marker and page-two Turbo Stream with Rizzy on fresh 46-notification fixtures. The Rust page loads read notifications 15, then 30, then one at a time through in-place stream updates. In separate Chromium sessions, both apps reached 46 unique notifications in newest-first order by scrolling, kept the index URL, removed the terminal page marker, and reported no page errors. The Rust page handles fetch failures with a manual retry button. Its notification card markup and styling still differ from Fizzy; this fixture checks one user's read list, not concurrent notification arrival or unread overflow.

The [100-notification browser benchmark](bench/results/notifications-html-100-cached-2026-09-25.json) found an initial Rust regression, then measured faster warm Rust reads after batching relationships and caching the first HTML page. `tests/notifications.py` warms one process's page cache, changes read state through another process, and checks that the first process refreshes its page. The cache version covers notification count, latest update, and unread count; a one-second expiry bounds stale details after unrelated card, board, or user changes. A per-user single-flight rebuild avoids concurrent refreshes; the [final 64-client sample](bench/results/notifications-html-100-singleflight-2026-09-25.json) had 6,465 Rizzy versus 778 Fizzy requests/s, with p95 18.4 versus 145.5 ms. The page sizes and presentation remain different, and these local read samples do not establish product-wide speed or scale.

The [scoped four-route JSON load run](bench/results/scoped-api-concurrency-2026-09-25.json) checked matching sampled shapes and counts on 100-card boards, then increased concurrency through 256 clients. At 64 clients Rizzy served 2,398 requests/s with 48.7 ms p95 versus Fizzy's 733 requests/s with 191.6 ms p95. An exploratory 100 ms p95 cutoff yielded a last passing level of 64 clients for Rizzy and 32 for Fizzy. The two target accounts, worker counts, and background contents differ, and the five-second local runs cover reads only; full equivalent sustained capacity remains unproven.

Rizzy's board JSON index now preloads its listed boards' creators and empty published-description records in batches. The [200-board page benchmark](bench/results/board-index-json-200-2026-09-26.json) checked five-page order and JSON shape before two ten-second, reversed-order read rounds on the 100-board fourth page. At 8 clients it measured 890–896 Rizzy versus 562–599 Fizzy requests/s; at 64 clients it measured 707–711 versus 621–622, with zero HTTP errors. The page bodies were 55.1 and 55.6 KB. The source used eight Puma workers; Rust used one release process, averaging roughly 10 CPU cores at 64 clients versus Fizzy's eight, so this does not demonstrate better scale at equal CPU budgets. The account-wide total counts differed because the disposable accounts contained different background boards, and only this JSON page was timed.

The board index query now follows Fizzy's access-time ordering without an additional creation-time sort, and existing Rizzy databases gain an `accesses(user_id, accessed_at DESC)` index on startup. The 200-board five-page parity fixture still passes. A separate [30,000-board in-memory SQLite probe](bench/results/board-index-query-30k-2026-09-26.json) changed the median warm 100-row page lookup from 7.6 ms to 0.04 ms across forty reads and removed the temporary sort from the query plan. This isolates the SQL page lookup rather than the full HTTP request.

A [30,000-board HTTP run](bench/results/board-index-json-30k-8cpus-2026-09-26.json) then found that the full Rust request was slower than Fizzy despite the faster page lookup because it joined every access to a board just to compute the total count. The index-only count through account-scoped access records removed that cost; startup repairs stale access metadata and `tests/access_record_migration.py` checks orphan cleanup. The [same eight-CPU fixture after the fix](bench/results/board-index-json-30k-8cpus-count-index-2026-09-26.json) measured 1,442–1,448 Rizzy versus 108–110 Fizzy requests/s at 64 clients, p95 51 versus 674–746 ms, across two reversed-order eight-second rounds with no errors. It checked five-page order, count headers, links, and sampled JSON shape; the timed response was about 55 KB in both. This demonstrates higher local read capacity for this large board index workload under equal CPU affinity, while full app performance and sustained mixed-write scale remain unproven.

The board JSON index now sends a weak ETag and private revalidation cache header, and returns a bodyless 304 with the same total-count and next-page headers. It computes the validator from the ordered board IDs, update times, and visible board fields before building JSON, so a fresh conditional request skips creator loading and serialization. `tests/boards_index_etag_parity.py` compares a 200-board fourth page against live Fizzy: ordinary, listed, and wildcard validators, a strong validator miss, a rename on the page, and an edit on another page all returned the same statuses and relevant headers. A sixteen-card fixture also passed the selected `.json` route and pagination checks. Exact ETag values are implementation-specific; other collection validators still need comparison.

The newer [balanced Rust-client repeat](bench/results/scoped-api-rust-client-repeated-2026-09-25.json) corrects the older per-worker endpoint mix and checks matching sampled content as well as shape. On fresh isolated accounts, two 20-second rounds at 128 clients measured 2,851–2,951 Rizzy versus 1,038–1,052 Fizzy requests/s, with p95 61.9–64.1 versus 197.2–213.2 ms and zero HTTP errors. Under a 100 ms p95 threshold, the last passing tested level was 128 clients for Rizzy and 32 for Fizzy. Server CPU, proportional memory, and response bytes are recorded in the result. This is still a single-host read comparison of a subset of functionality, with different deployment shapes, so full equivalent product scale and speed remain unproven.

`tests/my_menu_parity.py` checks the account navigation menu's main sections, ETag reuse, board addition invalidation, saved-filter ETag invalidation, and HTML-only response against Fizzy. The pinned Fizzy menu does not render its existing custom-view partial, even after a view is saved; Rizzy now omits its former extra section while retaining the filter collection in the validator. Rizzy links to the menu from its board, card, and activity views. Styling, keyboard behavior, and other conditional caching inputs remain simpler than Fizzy's.

`tests/card_edit_html_parity.py` compares the dedicated edit route's Turbo frame, form fields, Lexxy editor, JSON rejection, save, and reloaded title against both apps. `tests/card_edit_html_paired.py` runs the flow on fresh SQLite apps. The paired test also verifies the card's display frame, a full edit page for ordinary navigation, a small edit frame when requested with the matching `Turbo-Frame` header, and the three stream targets returned after an edit. Rizzy serves both edit response shapes and wraps the full page in its signed-in shell. On the card page, clicking the title now fetches that server-rendered frame and inserts it around the title and description, matching Fizzy's placement. A local Chromium run saved an edit without navigating, retained an unsaved title while another client updated the card, then showed the outside title after Escape; the cancel link also restored the display without navigation or page errors. Rizzy still applies the frame and stream through its own JavaScript rather than Fizzy's Turbo and Stimulus client, and visual parity remains incomplete.

`tests/card_params_parity.py` compares 23 JSON card-create and 23 card-update parameter cases on fresh pinned Fizzy and Rizzy accounts. Both now return the source's 400 JSON response for a missing, empty, non-object, or all-filtered card payload; flat permitted fields still work. Numeric and boolean titles and descriptions cast to the source's stored strings, null titles become Untitled, and arrays or objects are ignored when another permitted field survives. The fixture checks returned title/description, unchanged cards after rejected updates, the account card counter after each creation, and the next assigned number. The paired SQLite and MySQL browser edit flows, card HTML regression, timestamp regression, and 23 Rust unit tests pass. `tests/card_timestamp_invalid_parity.py` separately checks Fizzy's create fallback and failed-update state for invalid and null card dates, plus six accepted date-string forms. `tests/card_timestamp_scalar_parity.py` compares 20 create/update cases with numeric and boolean card dates on fresh SQLite accounts. Both apps now persist the scalar and return 500 for numeric and true creates and every scalar update, while false creates fall back to the current time; the test also checks account counters and GET errors for malformed cards and lists. Fizzy's SQLite date columns store those malformed scalars as numeric values, whereas Rizzy's text columns store their string forms. `tests/mysql_card_timestamp_scalar_parity.py` compares 16 create/update cases on fresh MySQL accounts: both apps reject numeric and true creates and all scalar updates with 500 and no committed card change, while false creates succeed with current timestamps. Other Rails date-casting forms and the malformed SQLite storage-type difference remain open.

Fizzy's `local-save` controller stores the card description draft under `card-<id>` after a 300 ms pause, restores it when editing opens, and removes it after a successful submit. Rizzy now follows that lifecycle in its in-card frame and standalone edit page. A local Chromium run entered rich text, canceled and reloaded the card, confirmed the draft restored in both editors, then saved from the standalone page; the card showed the saved content and browser storage no longer held the draft. The paired edit test checks that both source and Rust forms expose the same local-save key. This covers description draft persistence, not Fizzy's full autoresize, morph guard, and keyboard interaction set.

`tests/webhooks_html_parity.py` compares the browser webhook list, new and edit forms, validation, create/update/delete redirects, rendered detail, and JSON collection against Fizzy. It also checks the shared application shell markers on each webhook page. Rizzy now renders the webhook list, new, detail, and edit pages inside its signed-in shell, with the menu, skip link, and footer controls. Chromium created a webhook and opened its edit form; `tests/webhooks.py` confirmed the JSON API and delivery worker still pass. The Rust pages use simpler styling, and exact payload-byte and broader delivery parity remain outstanding.

`tests/webhook_delinquency_timestamp_parity.py` compares blocked delivery
failures and deactivation against the pinned Fizzy app. Both preserve the
webhook's update timestamp after the first failure, retain the first-failure
time, and advance the webhook timestamp when the tenth failure deactivates it
after an hour. Rizzy now stores tracker rows separately, exports one even for
a zero-failure webhook, and restores source tracker rows on account import.
The pinned Fizzy import probe also accepted a Rizzy account ZIP and restored
one tracker per webhook.
`tests/webhook_tracker_migration.py` checks idempotent SQLite backfill from
legacy webhook columns. A disposable MySQL 8.4.11 server passed partial-schema
recovery, a two-worker 100-delivery backlog, account import, and
`tests/mysql_webhook_tracker_migration.py`'s legacy backfill and second restart.
Cross-host delivery ordering and less-common event variants are still open.
Webhook DNS selection now pins the first public address in mixed DNS answers,
matching the pinned Surfguard policy, and keeps an all-private answer blocked.
The response-body limit now matches Fizzy's 100 KiB. The Rust DNS safety unit
tests pass. A local pinned-connection transport test checks the signature and
timestamp headers, a successful 202 response at exactly 100 KiB, and
`response_too_large` one byte over the limit. A local invalid TLS response now
reports `failed_tls`, matching Fizzy's delivery error category. Production DNS
selection still uses only a public pinned address; successful internet delivery
and these transport cases have not yet been checked in a paired live test.
Rizzy now selects Fizzy's generic JSON, Slack JSON, Campfire HTML, and Basecamp
form payload shapes by webhook URL, including the corresponding content types
and an event description without a viewer. Its unit test checks the format
examples and special-character escaping recorded in the pinned source tests;
the pinned `Webhook::Delivery` suite passed 29 tests and 67 assertions. The
Rust worker check confirms stored content types and private-target refusals
for generic, Campfire, and Basecamp hooks. `tests/webhook_wire_payload_parity.py`
creates equivalent published-card deliveries in disposable production Fizzy
and Rizzy databases and compares the complete Slack, Campfire, and Basecamp
content types and body bytes. It found and closed the source JSON serializer's
`\u0026` and `\u003c` escaping difference. The same test now compares the full
generic JSON structure and every stable value after masking generated IDs,
timestamps, and board/user IDs embedded in links. It clears Fizzy's JSON cache
before rendering: a warm cache from the API request otherwise retains that
request's local host in source webhook links instead of the configured public
host. Rizzy now serializes generic card and comment payload fields in the
same order as Fizzy's Jbuilder templates. The paired wire test compares their
raw JSON after normalizing generated IDs, publication keys, and timestamps;
escaping and field order match for a card in a named column on a public board
and for its comment, so the HMAC signs the same bytes for equivalent records.
Cache-dependent generic output, untested event variants, Slack worker
delivery, and external success and TLS paths remain open. The read-only
`inspect-webhook` command exposes a queued Rust delivery body and content type
for this diagnostic check without sending it.
The paired wire test now also creates a comment event. Generic JSON matches
every stable comment field, and Slack, Campfire, and Basecamp content matches
after normalizing the separately generated comment ID; Basecamp content is
decoded from its form body for that comparison. The test exposed a text-format
link difference: Fizzy links to the card's `#comment_...` anchor, while Rizzy
had linked to the standalone comment URL. Rizzy now uses the event link for
those three formats, while retaining the standalone URL in generic JSON.

`tests/webhook_cleanup_parity.py` runs Fizzy's `Webhook::Delivery.cleanup` and
Rizzy's `cleanup-webhook-deliveries` command against separate SQLite fixtures
with 503 eight-day-old and three recent deliveries. Both delete the 503 old
rows across the 500-row batch boundary and retain the recent rows. Rizzy now
has a `(created_at,id)` cleanup index on SQLite and MySQL, and a quarter-hour
worker with a database claim shared across processes. A disposable MySQL 8.4.11
run of `tests/mysql_webhook_cleanup.py` passed the same deletion count, chose
the new index in `EXPLAIN`, and retained it after restart. The automatic
quarter-hour timing and cleanup contention with live deliveries remain open.

`tests/card_controls_parity.py` checks pin and watch HTML button states, conditional ETags, Turbo replacements, and JSON toggles against both servers. Rust's watch stream also updates the comment subscriber section. The main card page still uses its own controls and JavaScript rather than Fizzy's frame-driven menu and broadcasts; the complete card interaction model remains incomplete.

The published card page now opens its editor when the title is clicked and checks an authenticated card revision while the page is visible. The revision includes the newest comment ID so a system comment committed after the title update is not missed. A local Chromium check changed a card through a separate API client: the system comment arrived while an open Lexxy draft and the old card body remained intact; Escape revealed the new title and description; another outside update appeared automatically after the editor closed. The page loaded without browser errors. A server-sent event connection batches active card revision probes every 1.5 seconds, up to 200 cards per database query, and shares each result among viewers of that card in the same Rizzy process; browser polling remains a fallback when the stream disconnects. `tests/card_live_stream.py` checks three viewers on two cards receiving edits written through a second process. Chromium showed an open stream, preserved its draft, and revealed the remote edit on Escape without page errors. Existing comment frames now carry their stored update time, allowing a remote comment edit to replace its displayed frame while an open local edit form stays untouched. Chromium verified a remote edit appeared in an unedited comment, then verified an unsaved comment edit stayed in place across another remote update and Cancel revealed the new server version. The paired `tests/comment_edit_html_parity.py` still passed against fresh pinned Fizzy and Rizzy instances after the frame change. An outside closure now updates the visible Done status, stage selection, closure label, and its HTTP method; a browser-dispatched click used the refreshed method to reopen the card. At 1280 × 640 on fresh paired cards, both Fizzy and Rizzy placed their Mark as Done buttons below the initial viewport at about y=674; an agent-browser pointer click scrolled each button into view and completed the card. The earlier initial-position failure did not establish a layout difference. A later browser check saw outside assignment, tag, reaction, and watch changes update the open page, including the subscriber count, and the refreshed tag picker still opened. The paired `tests/card_controls_parity.py` continued to pass. This differs from Fizzy's Action Cable broadcast. Some other card controls still do not all refresh in place; query cost across many distinct cards and large-client capacity still need measurement. Step additions, edits, completion changes, and removals now rebuild the displayed list through a stable delegated handler. A Chromium check changed and added steps from another API client while a new-step draft was focused: the list updated and the draft's text, focus, and cursor position survived. The refreshed checkbox saved to the database, and submitting the preserved draft created a third step without page errors. Image attachment and removal now update the background and image control on an open card. The paired `tests/card_image_parity.py` checks that deleting an image advances card and board update times, but leaves last activity unchanged, as Fizzy does. Chromium saw an outside image attach and removal without reload, then uploaded through the restored control; the image appeared and there were no page errors.

The published card new-step row now uses a disabled 18-pixel checkbox beside a separate text form, matching the pinned source structure instead of a square text glyph. Fresh [Fizzy](docs/visual-baselines/fizzy-card-step-1280x640.png) and [Rizzy](docs/visual-baselines/rizzy-card-step-1280x640.png) screenshots show the card at 1280 × 640. Agent-browser exposed the disabled checkbox in Rizzy’s accessibility tree, measured it at 18 × 18 pixels, and submitted a new step with Enter; the new step and next empty row appeared. Another signed-in session added a step while the new-step field held an unfinished draft; the live refresh displayed the outside step and preserved the draft value and focus. `tests/card_controller_html.py` and `tests/steps_turbo.py` pass. Other card styling and the complete Turbo interaction model remain incomplete.

`tests/card_board_html_parity.py` compares the card's board picker choices, frame, conditional response, browser form move, JSON move, and unsupported JSON picker against both apps. Chromium opened the picker from a card and moved that card to another board. `tests/card_move_access_parity.py` now creates equivalent private-board fixtures on live Fizzy and Rizzy: an owner move grants an assignee access and retains their assignment, pin, watch, and notification; a member move without board-admin rights grants no assignee access and removes those inaccessible records. Both apps also move the original card event to the destination and create one board-change event. The fixture inserts users and attached records directly into each database, then exercises the API move and waits for Fizzy's cleanup jobs; it does not cover every notification source or the complete browser interaction. `tests/card_move_access.py` retains the Rust-only regression. The picker page and menu interaction remain visually simpler than Fizzy's.

`tests/comment_edit_html_parity.py` compares comment creation through JSON and form Turbo requests, source-style before/update stream targets, a backdated JSON comment, flat JSON create/update parameters, the JSON-only comment index, the edit page's frame and controls, single-comment HTML and Turbo-format rejection, Turbo edits and removal, and the card-page frame target against both apps. It also checks the source's unusual plain HTML mutation behavior: create, update, and delete take effect before the route responds 406, including form-submitted updates and deletes. The pinned Fizzy comment controller suite passed 15 tests and 85 assertions. Chromium posted two comments consecutively without navigation or errors, with the editor resetting after each; it also edited and deleted in place, then kept two editors open, saved one, and canceled the other without targeting the wrong comment. The card page now uses Fizzy's messages frame and group label, comment avatar/content/author structure, subscriber footer, and collapsed system history control. A fresh live Fizzy/Rizzy run checked those HTML markers, the epoch timestamp targets, two system comments from closing and reopening a card, and comment CRUD/Turbo flows. Chromium showed the subscriber count changing from one to zero when the creator unwatched the card with no page errors. The creator's first-comment prompt and later prompt match Fizzy; the new comment editor disables empty posting, saves unfinished content locally, restores it after reload, and clears it after a successful post. Chromium verified restored content and validity, Ctrl+Enter posting and editing, Escape cancel, historical/current/dynamically inserted local timestamp formatting, the system-history expand/collapse control, and no page errors. Rizzy applies streams and local-time formatting through its card-page JavaScript rather than the source Turbo and Stimulus clients; its standalone edit page now applies save, cancel, and delete responses in place, while its layout and full comment presentation and broadcast behavior remain incomplete. A Chromium check on the standalone page confirmed save kept the URL and replaced the comment frame, Cancel restored that frame, and Delete removed the frame and stored comment.

The paired comment test now also unwatches the card creator before their first comment and checks the stored watch afterward. Both Fizzy and Rizzy reactivate exactly one watch row when the comment is created, matching the source's comment callback. This covers the local SQLite side effect; watcher broadcasts and the full notification pipeline remain separate parity work.

`tests/comment_touch_parity.py` compares comment create, edit, repeated edit, empty-body, null-body, repeated null, and delete timestamp effects on disposable Fizzy and Rizzy SQLite databases. Both advance the card and board update times on changed writes, advance `last_active_at` only on creation, and leave every timestamp alone on repeated identical edits. Rizzy now locks the parent board before comment writes and touches the card in the edit/delete transaction. The four-process MySQL cluster check additionally races 200 comment creates, edits, and deletes against same-card title changes, then 200 comment creates against board moves. Three fresh runs passed both phases without HTTP failures; a fourth checked all 200 resulting comment and creation-event rows and the deletion of the earlier 200 comments. Comment creation now uses the card's locked board for mentions and activity settings after a concurrent move. Other comment callbacks, simultaneous edits of one comment, and multi-host writes remain open.

`tests/board_accesses_pagination_parity.py` compares the complete JSON board-access user list on disposable Fizzy and Rizzy SQLite accounts with 200 additional active members. Both return 15, 30, 50, 100, then 6 users, matching alphabetic order, total and next-page headers, standard user fields, private-board access flags, and `access_only` and `watching` involvement. The source also sends a next-page link on the out-of-range empty sixth page; Rizzy matches it. A member with access can read the list, a member without access gets 404, and revoking the first member removes their access while retaining the other member's watching state in both apps. This checks one private board and selected members; HTML responses, other roles and boards, and the full set of live membership side effects remain open.

`tests/column_cards_etag_parity.py` compares named-column card JSON caching against pinned Fizzy with 16 cards over two geared pages. Both return a weak ETag, private cache policy, JSON content type, total and next-page headers, and 304 responses to the current tag, a list containing it, or `*`; a strong form of the weak tag returns 200. Editing a first-page card invalidates only that page. Editing the second-page card moves it to the front under the latest-activity sort and invalidates both pages. The MySQL core flow additionally checks that a step creation invalidates the column validator. This covers one named column, current-user reads, and card/step edits; other columns, permissions, and cache invalidation sources remain open.

`tests/board_special_column_etag_parity.py` compares the Maybe, Not now, and Done board-column JSON validators and authenticated HTML conditional responses against Fizzy. A card moving from Maybe to Not now invalidates those two JSON collections while Done stays cacheable; closing a different card invalidates Maybe and Done while Not now stays cacheable. Both apps return matching weak-tag, cache-policy, content-type, total-count, and 304 decisions for these fixtures. Rizzy still renders simpler HTML for the three views, and other state transitions remain to be compared.

`tests/named_column_show_etag_parity.py` compares the named-column index and show routes with live Fizzy. The index is JSON-only, while show returns both JSON and authenticated HTML. Both expose weak validators and 304 responses; show sends the checked count and changes its validator when a card enters, while the index changes after that entry or a rename. Rizzy now returns paginated cards on the HTML show page. `tests/column_touch_parity.py` compares card, board, and column timestamp changes after triage, title edits, comments, changed and repeated column renames, and untriage. The tested side effects match; HTML markup, additional pagination states, permissions, and other column callbacks remain open. A fresh MySQL core and four-process cluster run passed with the column-touch triggers; the move/title race fix rechecks the card's current board before editing it.

`tests/column_lifecycle_parity.py` compares new-column positions and the board and card timestamps affected by create and delete against live Fizzy. Creating a column now touches the board; deleting one touches every card on that board without advancing card activity, clears the deleted column from its cards, touches the board, and leaves other columns' timestamps unchanged. `tests/column_turbo_parity.py` compares browser form create/update streams, reorder streams in both directions and at a boundary, the HTML delete redirect, and the pure Turbo delete response with Fizzy. The MySQL core test checks lifecycle effects; the four-process cluster test checks concurrent column positions and disjoint concurrent swaps. A transaction-scoped board guard skips per-card board/column trigger work during deletion while retaining normal card-write callbacks; SQLite and MySQL upgrade checks passed. The Turbo streams use Rizzy's simpler column markup, so this does not establish visual or interaction parity.

`tests/column_params_parity.py` compares column create/update JSON parameters against live Fizzy, including empty and whitespace names, numeric name coercion, invalid and null colors, the stored values, error responses for empty or null inputs, and card/board/column timestamp effects. Rizzy now matches those tested cases. Other validation shapes and browser-form errors remain to be compared.

Column updates now read the current name and color after taking the database board lock. The four-process MySQL cluster test sends 30 simultaneous name/color edit pairs to one column and checks that every pair retains both values. Live Fizzy comparisons still match changed and repeated update timestamps, JSON parameters, and Turbo responses. This tests independent field edits on one board, not every simultaneous column operation.

The [matched column-delete benchmark](bench/results/column-delete-sqlite-guard-split-2026-09-26.json) timed an authenticated delete on boards with 1,000 and 10,000 cards, checking all card update times, unchanged activity times, column placement, and the untouched other-column timestamp. Three trials per size used fresh production Fizzy and release Rizzy databases and the same two CPUs. At 1,000 cards, median latency was 2.9 ms for Rizzy versus 15.1 ms for Fizzy; at 10,000, it was 21.1 versus 16.0 ms. The [30,000-card extension](bench/results/column-delete-sqlite-guard-split-30000-2026-09-26.json) measured medians of 70.0 versus 36.4 ms. The [pre-optimization run](bench/results/column-delete-sqlite-before-bulk-touch-2026-09-26.json) had measured 33.3 ms for Rizzy at 10,000 cards. A [one-pass SQLite trial](bench/results/column-delete-sqlite-single-pass-2026-09-27.json) passed the lifecycle parity test but increased Rizzy's 30,000-card median to 94.0 ms versus 32.9 ms for Fizzy; that change was reverted. A [same-day split-path recheck](bench/results/column-delete-sqlite-split-recheck-2026-09-27.json) measured 74.8 ms for Rizzy versus 33.3 ms for Fizzy at 30,000 cards and records the Rust binary hash. Rizzy still trails Fizzy on this large-board write. Fixtures clone real cards directly into each database, so the deletion work is matched but full card-creation side effects and broader application scale are not established.

The [30,000-card index trade-off probe](bench/results/column-index-tradeoff-2026-09-26.json) tested disposable copies of the same Rizzy database. Its current wide column index gave a 0.019 ms median first-page read and 0.255 ms count, but clearing 10,000 column links took 35.4 ms. A narrow board/column index reduced that update to 19.2 ms while raising the read to 2.37 ms and count to 1.95 ms. This direct-SQL probe explains the read/write trade-off and supports retaining the current index while read parity and throughput are still under development; it does not establish application-wide performance.

The [column-delete strategy probe](bench/results/column-delete-strategy-index-30000-2026-09-27.json) cloned one 30,000-card SQLite fixture into seven copies per strategy and checked card timestamps, column links, the surviving column's timestamp, and the board timestamp after each transaction. The former split update had a 62.7 ms median. A disjoint touch and nullification was 60.3 ms, a single CASE update was 80.1 ms, and an epoch-based trigger bypass with the wide index was 57.7 ms. A partial index containing only cards assigned to named columns plus the epoch bypass was 42.0 ms; dropping both card-update triggers entirely, which is not a valid production implementation, was 45.8 ms with the wide index. These are direct SQLite transaction timings, excluding HTTP and server work.

The [board-page index A/B](bench/results/column-index-board-read-30000-2026-09-27.json) kept the same 30,000-card fixture, running server, two CPUs, response sizes, and browser session while alternating wide and partial indexes in two reversed-order rounds. At eight clients, the wide index gave a 374.35 requests/s median for the board page versus 170.75 with the partial index; p95 rose from 28.5 to 60.935 ms. Named-column throughput stayed near 2,190 requests/s for both. The partial index's faster delete therefore moves too much cost to ordinary board reads and was not adopted. The production index remains wide.

SQLite column deletion now increments each touched card's bulk epoch so the board and column touch triggers skip their per-card guard lookup. The [paired 30,000-card HTTP recheck](bench/results/column-delete-sqlite-epoch-30000-2026-09-27.json) measured median delete latency of 62.3 ms for Rizzy and 32.8 ms for Fizzy across three trials, compared with 74.8 and 33.3 ms in the prior same-day split-path recheck. The [1,000/10,000-card recheck](bench/results/column-delete-sqlite-epoch-small-2026-09-27.json) measured Rizzy medians of 3.1/20.9 ms and Fizzy medians of 11.2/16.3 ms. The Rust binary hash in each artifact identifies the tested uncommitted build. `tests/column_bulk_epoch_migration.py` confirmed both fresh and upgraded SQLite trigger definitions; the paired column-touch suite still matched normal card, board, and column timestamp effects. The large-board delete remains slower than Fizzy.

A later [SQLite epoch-trigger trial](bench/results/column-delete-sqlite-epoch-probe-2026-09-27.json) skipped the per-row bulk-guard lookup during deletion and passed the paired lifecycle and normal column-touch checks. On three fresh 30,000-card trials, median delete latency was 66.6 ms for Rizzy versus 35.5 ms for Fizzy. The change was reverted because it remained slower and required a trigger migration; the committed delete path still uses the split updates and bulk guard.

The [expanded probe](bench/results/column-index-tradeoff-maybe-pair-2026-09-26.json) included the untriaged Maybe column. Excluding null column IDs from the wide index cut the unlink step to 20.6 ms but raised a Maybe first-page read from 0.022 to 2.411 ms. A second compact Maybe index restored the read to 0.027 ms but brought unlink back to 32.6 ms and raised the Maybe count from 0.275 to 2.054 ms. These are direct SQLite measurements on three disposable copies, not end-to-end HTTP results. The existing index remains until a layout improves the write without sacrificing common column reads.

`tests/named_column_html_parity.py` compares the empty named-column HTML state and a 16-card, two-page fixture against live Fizzy. Rizzy now renders the authenticated page header, column Turbo frame and drag container, rich draggable previews, empty placeholder, and geared automatic-pagination frames and link. Both apps showed 15 cards on page one and one on page two; unlike JSON, neither HTML response sent an `X-Total-Count` header. Chromium loaded all 16 Rust previews automatically, found no page errors, navigated back to the board, and searched from the bottom utility bar. The 1280-pixel [Fizzy](docs/visual-baselines/fizzy-named-column-1280.png) and [Rizzy](docs/visual-baselines/rizzy-named-column-1280.png) screenshots are visual references, not pixel parity. The source application shell, exact card styling, drag behavior, and additional page states still need comparison.

The [named-column HTML benchmark](bench/results/named-column-html-16-c1-8-32-5s-2026-09-26.json) used fresh equal 16-card fixtures, the same eight server CPUs, and two reversed-order five-second rounds at 1, 8, and 32 clients. Each app returned 15 cards on the first page and one on the second; all timed phases had zero HTTP errors. Rizzy served 1,769–4,582 full-page requests/s versus Fizzy's 72–174, a 21–30× paired throughput ratio. At 32 clients, Rizzy's p95 was 11–12 ms versus Fizzy's 218–219 ms. The first-page HTML sizes were 35,840 versus 120,814 bytes, reflecting Rizzy's still-incomplete application shell and markup. This is a faster result for the currently implemented page on one host, not full visual parity, an equal-work serialization comparison, or a maximum-scale measurement.

`tests/assignment_html_parity.py` compares the card assignment picker's frame, self option, conditional ETag, unsupported JSON and Turbo formats, selected state, JSON assignment toggle, Turbo replacement of the card metadata and picker frame, and self-assignment with Fizzy. The pinned Fizzy assignment and self-assignment controller suites passed 6 tests and 47 assertions. Rizzy's standalone picker and card-page dialog now apply the Turbo response in place. Chromium assigned and unassigned from the card page, reopened the picker, filtered the options, and found no console errors. The picker and metadata styling remain simpler than Fizzy's.

`tests/assignment_html_paired.py` runs that assignment flow against fresh disposable Fizzy and Rizzy accounts. It confirms that a plain HTML assignment POST changes the assignment and returns Fizzy's empty HTML 406 response, instead of Rizzy's former redirect. `tests/assignment_limit_paired.py` creates 100 accessible people per app and compares the picker at 99 and 100 assignees, a successful transition to 100, empty JSON 422 responses for further assignment and self-assignment, unchanged counts after rejection, and removal back to 99. The pinned source sends a hidden warning and visible unassigned choices in the raw HTML, then applies the limit with its browser controller; Rizzy now follows that split with a shared picker script. Paired Chromium card-dialog checks at 100 found the warning visible, the one unassigned choice hidden, and that choice still hidden after searching for it in both apps. Rizzy had no page errors during this check; Fizzy logged its unrelated passkey conditional-mediation error on the loopback origin.

`tests/assignment_keyboard_browser_paired.py` exercises the same browser flow on fresh 99-assignee cards in both apps. Opening with `a` selects the first item and focuses search; Down, Right, and Up select the same items without moving focus; searching `cafe` selects `Café 099`; Enter assigns that person and closes the picker; reopening shows the 100-assignee limit; Escape closes it. The paired run passed after Rizzy added picker selection and navigation to its shared script. Less common modified keys, mobile behavior, and exact dialog focus and styling remain open.

Assignment records now store `updated_at` in SQLite and MySQL rather than reconstructing it from `created_at` during export. `tests/assignment_record_migration.py` checks SQLite backfill and legacy inserts; an existing MySQL database with four assignment rows gained the column on startup and all four rows retained their creation time as the fallback update time. A fresh MySQL schema and a legacy-form insert also passed. Account export now uses the stored value, and the account import test checks that assignment creation times survive while update times refresh. `tests/assignment_record_parity.py` matched Fizzy's assignment create/remove effects on a card's update and activity times, board update time, system comment, and event. A four-process MySQL run had exposed a lost toggle when two assignment requests read membership concurrently. Rizzy now serializes the membership decision and write under the card's board lock; four subsequent runs checked 200 double-toggle pairs and all final assignment rows. Event, comment, and notification side effects still occur after that membership transaction, so their ordering under simultaneous requests needs further audit.

`tests/tagging_html_parity.py` compares the tag picker frame, creation control, conditional ETag, unsupported JSON and Turbo formats, selected tag state, JSON toggle, and Turbo replacement of the card tag block and picker frame with Fizzy. The pinned Fizzy tagging controller suite passed 5 tests and 31 assertions. Rizzy's card dialog and standalone picker now apply tag changes in place. Chromium created a tag, reopened the picker with its selected state, filtered options, and removed the tag without navigation or console errors. The picker styling and complete source tag validation behavior remain incomplete.

The card dialog and standalone tag picker now share a keyboard controller: opening or filtering selects the first visible option, Up/Down changes the selected option, and Enter toggles that option or creates a tag when only the create option remains. Filtering now ignores case and accents and waits 100 ms after input as Fizzy does. On a disposable paired browser fixture, both apps selected and removed an existing Café tag with `cafe` plus Enter, then selected Create tag for an unmatched name and created it with Enter. Rizzy also selected a second tag with ArrowDown and toggled a tag from the standalone picker. The fresh paired `tests/tagging_html_parity.py` route/cache/Turbo suite passed. Popup styling and all tag-validation edge cases are still open.

`tests/tag_title_edge_parity.py` compares eleven JSON tag-title inputs on fresh paired accounts, checking HTTP status and stored tag titles after each request. Fizzy removes only one leading `#` after trimming the input, preserves following spaces, accepts a single `#` as an empty stored title, rejects a second leading `#` with 422, and rejects missing or blank parameters with 400. Rizzy now matches those cases; the normal tag picker/Turbo suite and account export fixture still pass. This does not cover every parameter type or error body.

`tests/reaction_html_parity.py` compares card and comment HTML reaction lists and forms, emoji choices, JSON creation, Turbo creation and removal streams, and the source's plain HTML creation side effect: the reaction is saved before the route responds 406. The pinned Fizzy card and comment reaction controller suites passed 15 tests and 77 assertions. Rizzy renders card and comment reactions in one batched card-page query and applies both Turbo mutations in place. `tests/reaction_delete_forms_paired.py` now compares the source's card and comment delete forms, POST delete overrides, Turbo removal, and record deletion; `tests/reaction_delete_browser_paired.py` reveals and deletes both types in each browser without leaving the card page. Rizzy's standalone new form and reaction presentation remain simpler than Fizzy's. The local source returns 500 for reaction `new.json` and Turbo-format index requests because those renders lack templates; those broken formats remain outside the cross-app check.

`tests/reaction_touch_parity.py` now checks the stored timestamp effects of adding and removing both card and comment reactions on fresh SQLite databases against pinned Fizzy. Creation advances the card and board `updated_at` values and the card `last_active_at`; deletion advances only `updated_at`. Comment reactions also advance the comment's `updated_at` on both paths. Rizzy applies those touches in one board-first transaction. The MySQL core flow checks the same four effects, and the four-process cluster test checks 200 reaction deletions racing title edits. These checks do not establish every reaction validation, event, or visual behavior.

A Chromium check followed the card's Assign link and assigned the current user, selected 👍 in the reaction form and submitted it, edited a comment through Lexxy, and created a tag through the tag picker. Each action returned to the card without console errors. This covers one owner, one card, and a desktop viewport; it does not establish full interaction or visual parity.

`tests/account_card_create_parity.py` checks Fizzy's account level `POST /cards` route with JSON creation on a specified board and a browser form request that redirects to the card's draft. Rizzy routes it through the same card creation and draft logic used by its board nested route.

The filter settings refresh route returns a Turbo Stream save or delete toggle for the current filter without changing it. `tests/filters.py` checks unsaved, saved, and account-scoped filters. `tests/mysql_pair_selfhost.py filters-browser` now checks a signed-in save, closed-card results, duplicate save, refresh, and delete against pinned Fizzy on separate MySQL databases. It also checks that JSON Accept receives `406` on create, refresh, and delete, while create and delete retain their database side effects. The paired lifecycle passes. Exact markup and the complete filter settings UI have not been matched.

The all-cards browser page now requests that toggle after a user changes a filter, displays Fizzy's bookmark-outline icon with an accessible save/delete label, and applies the returned Turbo replacement without navigating. A disposable Chromium fixture opened a filtered page with no initial toggle, selected Golden through the status picker, saved the view, and removed it; the control switched between Save and Delete and the database gained then lost the filter row. `tests/filters.py`, `tests/cards_html.py`, and the Rust unit tests pass. Rizzy still navigates the full page for filter changes rather than preserving Fizzy's Turbo filter settings, and the board page and remaining filter UI do not yet offer the same complete workflow.

The all-cards browser page now exposes board, status, sort, text, tag, assignee, creator, assignment, and date controls, with a filtered card grid and automatic preview pagination. Its card previews include Fizzy's board/number header, title body, column name/color, tag, completed-step count, author and assignee avatars, assignee names, comment count, boost count, background image, and metadata footer regions. Page-scoped queries batch the five related data types and card image URLs; avatar images use the existing User avatar route. The board's filtered view now renders the same full previews instead of title-only links. The all-cards status choices now follow Fizzy's six supported values, No one sits inside Assigned to as an exclusive choice, and the board picker uses the user's recent access order. `tests/cards_html_parity.py` compares the 15-card first page, two-card second page, closed cards, text search, oldest sort, assigned and unassigned results, recently accessed board ordering, tag and step display, and unchanged JSON route against live Fizzy; `tests/cards_html.py` covers the browser form's empty optional fields, column and metadata appearance, uploaded avatar and image in both all-cards and filtered-board previews, and preview append response. Chromium switched an all-cards fixture from one named assignee to No one and back, with each submission clearing the conflicting choice and showing the corresponding card. A fresh source-versus-Rust `tests/board_filter_html_parity.py` run still matched column mode, state filters, term pagination, oldest sorting, and empty results after the filtered view change. Chromium loaded an image-backed preview with a photo avatar, followed its card link, and loaded the board with both media assets and no page errors: [Rizzy media fixture](docs/visual-baselines/rizzy-preview-media-1280.png). Earlier, Chromium loaded the grid and appended the remaining cards while scrolling; its search form initially exposed a blank-ID filter bug, which was fixed and verified. The pinned source's preview controller test and a temporary 17-card integration probe passed, but the local production Fizzy fixture returned 500 on a first-page preview response requiring a next link, so direct preview-stream parity remains pending. The local source's 64 CSS files were precompiled and served for a styled comparison at 1280 × 633 pixels: [Fizzy](docs/visual-baselines/fizzy-cards-1280.png) and [Rizzy](docs/visual-baselines/rizzy-cards-1280.png). The Rust grid has similar placement and card density, but the layouts are not pixel identical. Card interaction controls and other preview details remain incomplete. Chromium verified the Rust page filter, global search, empty notifications, and pinned-card panel with a populated pin.

`tests/card_column_html.py` checks the card column chooser's Maybe, Not now, named, and Done options, the selected stage after state changes, JSON rejection, and account access. Chromium loaded the chooser from a card, moved it into a named column, and showed that column as selected on reload. The live source returned a column chooser frame with the same core stage labels. Rizzy still uses its own card layout and JavaScript mutation path rather than Fizzy's complete Turbo frame markup and behavior.

The published card page now places its blue panel, board label, title, description, stage rail, and editable steps near the styled source's positions at 1280 × 633 pixels: [Fizzy](docs/visual-baselines/fizzy-card-1280.png) and [Rizzy](docs/visual-baselines/rizzy-card-1280.png). Chromium completed and restored a step, added a step, moved the card to Not Now and back, toggled Golden Ticket, opened the utility search panel, uploaded and removed a background image through the source-compatible direct upload endpoint, and added and removed a card tag without console errors. The browser still lacks Fizzy's full assignee/reaction interface, rich card metadata, exact lower-page message design, and native Turbo behavior. The tag picker uses a simpler input and current-tag list. The screenshot comparison covers one short card on a desktop viewport, not a full visual parity gate.

A fresh 390 × 800 browser comparison of the same blank-description card is captured in [Fizzy](docs/visual-baselines/fizzy-card-empty-mobile-390x800.png) and [Rizzy](docs/visual-baselines/rizzy-card-empty-mobile-390x800.png). Rizzy now collapses the unused description space, shows the mobile Done check, and gives the new-comment editor the source's 34-to-365-pixel horizontal span with the Bold control visible. The Rust viewport has no horizontal overflow, and a separately loaded card with a nonblank description retains its normal description spacing. The main card panel ends at about 438 pixels in both screenshots; the header, metadata, action buttons, and editor still differ visibly. `tests/card_controller_html.py` and `tests/steps_turbo.py` pass on the release build.

The card's back, Golden Ticket, image, tag, edit, completion, watch, and pin controls now use local copies of the pinned source SVG silhouettes through CSS masks. Their accessible labels remain text. The mobile no-image card background follows the actual grid rows, ending its blue band behind the upper half of the action controls as in the source screenshot. Chromium loaded the assets without horizontal overflow; pinning switched the mask from `unpinned.svg` to `pinned.svg`, and unwatching switched from `bell.svg` to `bell-off.svg` after the corresponding server mutations. The tag picker still opened, a card with a nonblank description kept its content and 390-pixel page width, and following a card link from Home gave the mobile back arrow the accessible label "Back to Home." Image-backed cards, control spacing, metadata, and remaining interactions still need direct paired verification.

The published card footer now renders author and assignee avatars, Added/Updated dates, familiar names, and an assignee-avatar trigger in the same metadata regions as Fizzy. Its card reactions sit in that footer. The existing assignment picker replaces the richer metadata through its Turbo response; a browser check toggled assignment off and on, reopened the picker after replacement, and used the `a` shortcut without page errors: [Rizzy card metadata fixture](docs/visual-baselines/rizzy-card-meta-1280.png). A fresh source-versus-Rust `tests/assignment_html_parity.py` run matched the metadata regions, assignment and self-assignment responses, Turbo replacements, and the hidden picker on a closed card; the reaction parity suite still matched card and comment mutations. The exact avatar-trigger dialog presentation, reaction controls, footer layout, and lower-page messages remain different from Fizzy.

The pinned `cards_controller_test.rb` suite passes 31 tests and 170 assertions. `tests/card_controller_html.py` checks Rizzy's inline-code title link and escaping, hidden self-assignment control, delete-button visibility, and deletion as a member, card creator, board creator, and owner. A board creator who is only a member can no longer delete someone else's card; this now follows Fizzy's card-specific permission rule. HTML deletion redirects to the board, while JSON deletion remains 204. Chromium opened and canceled the confirmation dialog, used the `m` shortcut to self-assign, and submitted the browser deletion form; it landed on the board without an error overlay. The broader controller suite, attachment edge cases, and exact source Turbo presentation remain to be compared on Rizzy.

The local `tests/parity.py` scenario currently passes 142 comparisons against the running reference server using fresh accounts. It checks status and JSON shape for the core workflow, selected state values, search results, tag, assignment and card state filters, account and board entropy, join-code metadata, board access and involvement, webhook management, basic notification routes and settings, card reading, publication, activity events and descriptions, event deletion, and system comment count. `tests/auth.py`, `tests/join.py`, `tests/email_change.py`, `tests/verification.py`, `tests/webhooks.py`, and `tests/notifications.py` exercise Rust authentication, join, email change, verification, webhook, and notification flows with disposable accounts; they are not yet cross-implementation comparisons. Neither test replaces the full upstream test suite.

`tests/filter_parity.py` passes 13 additional cross-app card-result comparisons, including terms, closure dates, closers, a backdated card, and a card approaching auto-postponement. `tests/saved_filter_parity.py` checks that a saved filter selects the same closed card on both apps; the Rails filter is created through its model layer because its Turbo controller requires a browser session. `tests/filter_timezone_parity.py` compares creation and closure date windows around local midnight, week, month, and year boundaries in Los Angeles and Tokyo, including JSON cards, a filtered board page, and Turbo card previews. Rizzy now evaluates those windows in the request's timezone cookie rather than UTC. `tests/source_time_window_probe.rb` records Fizzy's exact UTC ranges across New York spring and fall transitions, Santiago's missing midnight, Havana's repeated midnight, and Apia's skipped civil day; the Rust time-window unit test matches all seven cases. The MySQL core flow also checks creation and closure filters around Los Angeles midnight on Oracle MySQL 8.4. The full filter UI and other time zones remain unverified.
`tests/stalled_parity.py` additionally checks the stalled index before and after an old spike, a fresh comment, and postponement. Local `tests/stalled.py` checks spike creation from assignment, reopening, and multi-person comments. These checks do not cover all background-job timing or all search syntax.

`tests/exports.py` checks user export ZIP content, board access scope, download permissions, asynchronous completion, and the 24-hour request limit. A live Fizzy export confirmed the same sample card's board, title, status, escaped description, and comment body format. `tests/export_attachment_parity.py` compares the original bytes and ZIP entries for card and comment rich-text embeds, relative links in their HTML, deduplication, and exclusion of the separate card background image against Fizzy. `tests/account_export_parity.py` compares account export creation, completion, authenticated download, and the ZIP envelope against the live source. `tests/account_transfer.py` checks permissions, rate limit, source-style record paths, rich-text GID conversion, and original file bytes on an isolated Rust account. `tests/account_import.py` imports that archive into a new Rizzy account, checks restored record counts, flags, owner, rich text, original file bytes and card rendering, re-exports it, and verifies duplicate-ID, cross-account reference, duplicate publication key, invalid-ZIP, worker recovery, and expired failure cleanup. The same test imported a live Fizzy 500-card/500-comment export with 1,000 events and 500 watches. `tests/enrich_transfer_archive.py` adds source-schema card due dates, blob metadata, a variant and attachment, a portable User rich-text attachment, a mention, a Mention notification, and a notification bundle to the small archive. Rizzy imported and re-exported that archive with the record counts and selected values preserved; Fizzy's validator accepted the re-export and its importer restored those additional rows, date, metadata, and signed User attachment in an isolated database. `tests/notification_source_migration.py` checks that an existing Event notification survives the polymorphic source migration, a Mention notification renders, and both source deletion paths clean up notifications. This does not cover every source record type or the complete browser flow. `tests/mentions.py` checks Rails-compatible signed User attachment IDs from the global and board mention pickers, live mention creation from card and comment attachments, repeat edits, self mentions, watches, notifications, draft publication, and access revocation. `tests/user_attachables_migration.py` checks stable backfill for existing databases. `tests/prompt_parity.py` compares User prompt text and board scope plus card picker markup, latest ordering, search, comment matches, ten-result cap, exact number and draft behavior, and ETag revalidation against live Fizzy. The native mention editor, persisted image variants, push delivery, full email rendering, and complete presentation remain incomplete. Rizzy rejects unsupported nonempty record groups and currently caps uploaded and expanded archives at 512 MiB. The export worker retries stale jobs after five minutes; the import worker retries after a 30-minute stale lease. Both remove completed ZIPs after 24 hours; failed import accounts expire after seven days.

`tests/account_export_internal_blobs_paired.py` compares fresh Fizzy and Rizzy account ZIPs with three uploaded blobs and one direct upload whose file transfer was never completed. Both archives include the ordinary blob and file, include metadata but no file for the pending blob, and exclude blob metadata, attachment records, and file bytes for blobs attached to internal `Export` or `Account::Import` records. Both also export a deactivated member with no identity; Rizzy now restores that user with a null identity instead of inventing an email address. The test imports the source ZIP into Rizzy and checks that the pending blob remains without file bytes while the completed blob has its file. It also imports a source-format ZIP with a nested storage key, matching Fizzy's `FileRecordSet` test: Rizzy regenerates the key and restores the bytes. Unsafe empty, dot, parent, or backslash path segments remain rejected by Rizzy's ZIP reader. The test verifies that Rizzy recreates the supporting attachment index on startup. `tests/account_transfer.py` still confirms normal card images and rich-text files export after this change. Complete account transfer parity remains open.

New seeded, signed-up, and imported Rizzy accounts now create their System user without an identity, matching Fizzy. `tests/account_export_internal_blobs_paired.py` checks the stored identity and ZIP fields on fresh paired accounts. Existing Rizzy databases detach synthetic System identities on startup without changing the user's stored update time; an unused synthetic identity is removed. `tests/user_deactivation_migration.py` checks that upgrade and a stable restart from an old non-null user schema. Creator lookups now retain identity-less System users, and `tests/auto_postpone_parity.py` checks that the system-authored comment remains readable through both apps' JSON routes. The MySQL upgrade path has not been exercised in this workspace.

Fizzy's generic account importer assigns a fresh `updated_at` to imported records. Rizzy now does this for every supported generic record type whose update time is stored locally, while preserving its source `created_at`; `tests/account_import.py` checks both fields on a small fixture and on the 5,891-card source archive. Generic record types without a local update-time column still need schema and export alignment. A re-export after import is therefore not expected to preserve every source `updated_at` value verbatim.

Tag and Tagging records now have stored update times, and Taggings also have stable IDs and creation times. The tagging route writes those values; account export uses them instead of inventing a new Tagging ID and the parent card's timestamp on each run. Account import preserves the source IDs and creation times and refreshes update times as Fizzy's generic importer does. `tests/account_transfer.py` checks the ZIP records against the database, `tests/account_import.py` checks imported and re-exported IDs/times, and `tests/tag_record_migration.py` upgrades old SQLite rows and checks old-form inserts. The schema includes a MySQL upgrade path, but no MySQL service is available in this environment to execute that migration or transfer flow. Other generic records that still reconstruct update times need the same treatment.

Pin and Reaction records now store `updated_at` alongside `created_at`. Their create routes set both fields, account export reads the stored value, and account import preserves the source creation time while refreshing the update time. The small account export/import fixture now includes a pin and a reaction and checks their ZIP and restored values. `tests/interaction_record_migration.py` backfills SQLite rows from the old schema and verifies old-form inserts; the paired pin HTML and reaction timestamp suites still pass. The MySQL schema upgrade compiles but awaits execution against a MySQL service.

Fizzy's specialized user importer preserves each source user's `updated_at`. Rizzy now stores that field, backfills older databases from creation time, keeps it through the legacy nullable-identity table rebuild, and exports the stored value. `tests/account_import.py` checks every user's creation and update times through import and re-export of the real Fizzy archive; `tests/account_transfer.py` checks a changed name advances the time while a repeated no-op does not. `tests/user_deactivation_migration.py` checks the old-schema backfill, deactivation, and rejoining. Runtime access changes now touch users and boards; the importer restores archived user times after its bulk access inserts, as Fizzy's bulk importer bypasses callbacks.

Event records now store their own update time. Existing Rizzy databases backfill it from event creation time, while imported source events receive a fresh update time and account exports use the stored field. `tests/notification_source_migration.py` checks the old-schema backfill and old-form inserts; `tests/account_import.py` checks a real source archive with 5,980 events through import and re-export. Event metadata from older Rizzy databases cannot recover any update time that was never stored.

`tests/board_views_parity.py` compares private special-column JSON status, response shape, card order, and pagination, plus public column membership and pagination, against live Fizzy. It checks the source-style public pagination link and destination. Chromium loaded a 47-card public board across three pages of 15, 30, and 2 distinct previews without page errors. `tests/public_page_html_parity.py` compares anonymous board and card pages: Maybe, Not Now, named, and Done sections with published card counts, rich public description, card description and read-only steps, a public card image and social preview URL whose original bytes load without authentication, public caching, and removal after unpublishing. It now checks source-style preview articles with tags, assignees, named-column stages, a Done stamp, inline-code titles, and anonymously downloadable card image bytes. The pinned public board, card, and named-column controller suites passed 8 tests and 25 assertions. Chromium loaded the Maybe frame and expanded Not Now without an error overlay. [Fizzy](docs/visual-baselines/fizzy-public-board-1280.png) and [Rizzy](docs/visual-baselines/rizzy-public-board-1280.png) screenshots at 1280 × 633 show the public board styling; Rizzy has the dark centered layout and richer previews, but the typography, column interactions, native Turbo frame behavior, public card presentation, and other details still differ. The earlier [100-card](bench/results/public-board-html-100-2026-09-25.json) and [1,000-card](bench/results/public-board-html-1000-2026-09-25.json) anonymous read runs report faster Rust responses on matching counts and first-page titles; the [richer-preview 100-card rerun](bench/results/public-board-rich-html-100-2026-09-25.json) keeps that read advantage while increasing Rizzy's stream response from 7.9 KB to 28.9 KB. Fizzy's stream response remains 88.2 KB, so these runs do not establish full page or product-wide performance parity. The test checks that private JSON is protected; `tests/auth_format_parity.py` covers bearer format and unauthenticated redirect behavior separately.

`tests/not_now_record_parity.py` compares persisted Not Now creator and time, public preview stamps, repeated POST behavior, and record removal on close and triage with the pinned source. The second Not Now POST resumes a postponed card in this revision; Rizzy reproduces that observed behavior. `tests/not_now_record_migration.py` backfills older flag-only databases from their latest postponement event or card timestamps and confirms stable restarts. Public card detail pages now show the source-style dated Done and Not Now stamps. Account export and import preserve the Not Now record ID, user, and creation time rather than reconstructing them from the card flag; import gives it a fresh update time as Fizzy does. The [100-card Not Now read sample](bench/results/public-not-now-html-100-2026-09-25.json) reports faster local Rust reads on matching first-page titles and stamp markers, with much smaller Rust HTML; it is not a sustained or fully equivalent scale result.

`tests/goldness_record_parity.py` compares Golden Ticket record creation, repeated no-op creation, deletion, re-creation, unchanged card activity time, and the public preview class with the pinned source. `tests/goldness_record_migration.py` backfills flag-only databases and confirms stable restarts. Account export/import now preserve Goldness record IDs and timestamps as well as Not Now records. Existing legacy records use the card update timestamp because their original Goldness creation time was not stored.

Activity spikes now retain source-style record IDs across updates and account transfers; `tests/activity_spike_record_migration.py` adds stable IDs to existing Rizzy records without changing their timestamps. Fizzy's account ZIP can contain an account-scoped spike whose card is absent from the archive. Rizzy preserves that record and reexports it, while a card-deletion trigger still removes spikes attached to deleted local cards. `tests/account_import.py` imported and reexported a source ZIP with 5,891 cards, 5,980 events, and one such spike; this verifies the represented records in that fixture, not complete account-transfer parity for every source feature or archive size.

Watch records now retain their IDs and creation times through account transfer; toggles update their stored update times, and import assigns a fresh update time as Fizzy does. `tests/watch_record_migration.py` backfills older Rizzy databases using card creation time when the original watch time was unavailable; `tests/watch_record_lifecycle.py` checks stable IDs and repeat requests. The same source ZIP import/reexport check matches all 5,979 watch records by ID and non-update fields and verifies their fresh imported update times.

`tests/watch_touch_parity.py` compares owner watch-on, watch-off, repeat no-op, and watch-on again against fresh pinned Fizzy and Rizzy databases. Both advance watch, card, and board update times only on state changes, leaving card activity time intact. Rizzy now rechecks the card's current board under its board-first write lock, so a concurrent move cannot leave the watch mutation attached to a stale board lock. The four-process MySQL cluster test ran 200 watch/move pairs and, with the MySQL client configured, checked four representative final watch records; two earlier runs without that database check also passed. This does not cover every watch recipient or broadcast path.

Access records now retain their IDs, account scope, creation time, involvement, and last access time through account transfer; import refreshes their update times as Fizzy does. `tests/access_record_lifecycle.py` checks stable identity and update times as involvement changes; `tests/access_record_migration.py` backfills records on older databases and verifies stable restarts. The real Fizzy ZIP's 32 accesses import and reexport with matching IDs and non-update fields plus fresh imported update times. Source Access, board accessibility, and involvement suites pass 12 tests and 53 assertions. Existing Rizzy access rows had no stored ID or creation time, so migration assigns an ID and uses board creation time as the fallback. `tests/source_access_touch_probe.rb` on a disposable pinned-source database confirms create, changed involvement, `accessed_at`, and delete touch both user and board; repeated involvement and recent-access no-ops touch neither and leave Access's time unchanged. `tests/access_touch_migration.py` verifies those stored times and upgrades the old no-op update trigger. Board access revocation and account user deactivation now share cleanup of the removed user's board mentions, notifications, watches, pins, and assignments; `tests/user_deactivation_migration.py` checks all five categories while preserving another user's records, and the mention and join flows still pass. Fizzy queues this work after commit, while Rizzy performs it in the request transaction; asynchronous timing, broadcasts, and full browser side effects remain to be matched.

User settings now have stored IDs, creation times, and update times alongside their email frequency and timezone. New non-system users start with Fizzy's `every_few_hours` default. `tests/user_settings_record_lifecycle.py` verifies stable IDs, changed timestamps, and repeat no-ops; `tests/user_settings_record_migration.py` backfills older users without losing their selected values. The real Fizzy ZIP's user settings record imports and reexports with matching ID and non-update fields plus a fresh imported update time. Source settings, timezone, and notification settings suites pass 10 tests and 45 assertions. Older Rizzy databases cannot recover metadata that was never stored, so migration uses the user creation time. The complete settings UI and delivery behavior still need wider parity checks.

Entropy now has persisted account and board record metadata. `tests/entropy_record_lifecycle.py` checks default and overridden periods, stable IDs, card update timestamps after entropy changes and repeated account and board entropy saves, unchanged record metadata on a repeat, and board deletion. `tests/entropy_record_migration.py` gives older databases stable account and board records. The pinned source entropy model and controllers pass 16 tests and 71 assertions. Importing the real Fizzy ZIP retains the destination account entropy ID and updates its period, as Fizzy's importer does; importing a Rizzy ZIP with a board override preserves the board entropy ID and timestamps. Existing Rizzy databases use account creation or board update time when the original entropy metadata was unavailable. Browser response markup and scheduled-job timing still need wider comparison.

`tests/entropy_params_parity.py` compares account and board entropy requests on fresh pinned Fizzy and Rizzy databases. Invalid integer and flat JSON values return empty 422 responses; numeric strings, leading-number strings, and floats store the same seven-day value; null returns 422; booleans return the same 500 JSON error; and a missing field returns the same 400 JSON error. Account JSON success values and both stored entropy values match. The test uses an existing board entropy record and excludes the successful board JSON value because Fizzy's production fragment cache can return its earlier value, as noted below. That response cache behavior still differs.

`tests/board_entropy_touch_parity.py` compares three board entropy saves, including a repeated value, on live source and Rust SQLite accounts with a card in a named column. Both now touch the card on every save, keep its activity time and the board and column update times unchanged, create or update the board entropy record only when the value changes, and preserve the stored period. The expanded MySQL core flow verifies the same board/column/card timestamp effects and one row-epoch increment for each of the first and repeated saves.

`tests/account_entropy_touch_parity.py` compares changed, repeated, and changed-back account saves on two live source and Rust SQLite boards, one inheriting the account period and one overriding it. Every save touches cards on both boards without touching board, column, or card activity times; only changed account values update the account entropy record. The expanded MySQL core flow checks first and repeated account saves with the same timestamp and row-epoch effects. `tests/mysql_account_entropy_trigger_migration.py` verifies replacement of the older account trigger on an existing database. Larger card sets and the production board JSON cache remain outside these checks.

Rizzy now runs an hourly auto-postponement sweep at minute 50 and exposes an `auto-postpone` command for an immediate run. `tests/auto_postpone_parity.py` invokes Fizzy's pinned model method and the Rust command against separate fresh SQLite databases. Due cards under account and board periods acquire a system-owned Not Now record, an automatic-postponement event, and a system comment; recent, closed, and already postponed cards remain unchanged, and repeating the sweep makes no further changes. The SQLite check also verifies that an existing Not Now record keeps a card inactive when Rizzy's old flag disagrees, as Fizzy's record-based active scope does. `tests/not_now_record_migration.py` verifies that SQLite startup backfills a flag-only legacy card before a sweep. `tests/mysql_auto_postpone.py` checks 205 ordinary due cards, a recent card, and both inconsistent flag states on MySQL, runs two Rust sweep commands concurrently across the 100-card batch boundary, and requires one Not Now record, event, and system comment per due card. Each card's move, event, comment, and webhook queue writes commit together. The hourly database claim prevents multiple Rizzy processes from scheduling the same slot; the actual clock-triggered run, crash recovery after a claimed slot, multi-account and MySQL scale, and every event/broadcast side effect have not yet been verified against Fizzy.

Rizzy now runs Fizzy's daily unused-tag cleanup after 04:02 UTC and exposes `delete-unused-tags` for a manual run. It removes a tag only when no card uses it, and updates or deletes saved filters that referenced the tag in the same transaction. `tests/unused_tag_cleanup.py` checks used and unused tags, a tag-only saved filter, a surviving filter, a digest collision, and a repeated cleanup on SQLite. Its filter outcomes were also checked against the pinned Fizzy job in an isolated source database. `tests/mysql_unused_tag_cleanup.py` passed on Oracle MySQL 8.4.11 with 209 unused tags across three 100-tag query batches, a used tag, the same saved-filter cases, two concurrent cleanup commands, and an idempotent repeat. The clock-triggered run and concurrent tagging at cleanup time remain unverified.

The [100- and 1,000-due-card SQLite sweep](bench/results/auto-postpone-sqlite-100-1000-2026-09-26.json) and [5,000-due-card sweep](bench/results/auto-postpone-sqlite-5000-2026-09-26.json) each used four boards, an equal number of recent cards, two pinned CPUs, fresh databases, and two reversed-order rounds. These results were rerun after the record-based active-scope fix. Timing starts inside Fizzy's `Card.auto_postpone_all_due` and Rizzy's equivalent function, excluding process startup and fixture setup. At 5,000 due cards, Fizzy took 68.08–68.11 seconds and Rizzy 0.645–0.689 seconds, a 99–105× advantage for this sweep body. Every one of the 10,000 cards per phase was checked for the expected system-owned Not Now record, automatic event, system comment, rich-text record, and activity-time direction. These direct-insert fixtures have no watchers, webhooks, attachments, or queued delivery workers, and the check does not establish full callback or product-wide performance parity.

Closure records now retain their IDs, closer, and creation times through account transfer; import refreshes their update times as Fizzy does. `tests/closure_record_lifecycle.py` checks repeat close/reopen requests and reclose identity; `tests/closure_record_migration.py` upgrades the old table and backfills flag-only cards while preserving IDs across restarts. The source closure model and controller suites pass 10 tests and 50 assertions. Import and reexport of the real Fizzy ZIP match all 52 closure IDs and non-update fields plus fresh imported update times. Older Rizzy databases cannot recover original closure IDs that were never stored, so migration assigns stable replacement IDs.

Board publications now have their own stored ID, key, and timestamps. `tests/publication_record_lifecycle.py` checks repeated publish/unpublish and re-publish identity; `tests/publication_record_migration.py` backfills boards whose keys predate the separate record. The source publication model and controller suites pass 13 tests and 74 assertions. Import and reexport of the real Fizzy ZIP match all 12 publication IDs and non-update fields plus fresh imported update times. Older Rizzy databases receive stable replacement IDs and use the board update time because the original publication metadata was not retained.

Rich text record metadata now survives card, comment, and board edits and account transfer. `tests/rich_text_record_lifecycle.py` checks stable IDs and deletion cleanup, and `tests/rich_text_record_migration.py` backfills inline text already stored by Rizzy, using its parent record timestamps when the original Action Text times were unavailable. The source ZIP import/reexport check matches the IDs, bodies, and timestamps of all 5,790 Action Text records. Rizzy exports blank Action Text bodies as JSON null, as Fizzy does, and import preserves that null state; the account transfer fixture checks an explicit-empty board description through export and reexport. The archive format collapses empty and null bodies to the same JSON null. Rizzy still stores their content inline for rendering, and its Active Storage attachment model does not yet preserve every Action Text association and variant byte representation.

The private board HTML now renders a centered Maybe column with collapsed Not Now, named, and Done columns, similar to the styled source at 1280 × 633 pixels: [Fizzy](docs/visual-baselines/fizzy-board-1280.png) and [Rizzy](docs/visual-baselines/rizzy-board-1280.png). It includes the first 15 rich Maybe cards in the initial response, fetches rich previews for other columns on expansion, and offers card links, draft creation, named-column creation, further pages, a watcher toggle, utility panels, and native browser drag and drop. Four Fizzy-compatible drop POST routes return a Turbo Stream for the destination column; `tests/board_drop_parity.py` compares their state changes and response target with live Fizzy. `tests/board_filter_html_parity.py` also checks the Rust board structure, fragment previews, and watcher toggle. Chromium verified rich lazy loading, column switching, watch state changes, and the search panel without console errors. The board still lacks Fizzy's full Turbo frame behavior, complete filter controls, keyboard drag handling, and exact visual design. A synthetic 30,000-card [query probe](bench/results/board-column-index-2026-09-25.json) justified a named-column index, confirmed that existing databases add it on startup, and compared initial-response queries before and after lazy loading; it is not an end-to-end scale result. The [board HTML benchmark](bench/results/board-html-2026-09-25.json) checks matching first-page card counts and reports faster Rust reads on this fixture. The [card HTML benchmark with reactions](bench/results/card-html-with-reactions-2026-09-25.json) checks matched steps, a comment, a tag, and card and comment reactions before timing, and also reports faster Rust reads. Different HTML byte sizes and incomplete behavior prevent a product-wide performance claim.

The board filter panel now offers account tags and active owner, admin, and member users for Tagged, Assigned to, Added by, and Closed by, plus No one. Fizzy's `User.active` scope excludes its internal System user; Rizzy's board people controls now do the same, with a fresh paired `tests/board_filter_html_parity.py` run checking the exclusion. Checkbox choices submit immediately, keep selected values across reloads, and treat No one as exclusive with named assignees. That paired run also matched Fizzy's tag, assignee, unassigned, creator, and closer result sets alongside the existing state, term, sort, and pagination cases. Chromium selected a tag and then checked both the named-assignee and No one paths on a two-card board with no page errors. Saved-filter routes still accept a board ID and `tests/filters.py` passes persistence, deduplication, permission, and Turbo response checks. The pinned source's custom-view menu partial is not rendered, and its initial board and all-cards pages omit the saved-view toggle target. Rizzy now matches that visible behavior; saved views remain reachable through their URL. The filter controls use native checkboxes and form submission rather than Fizzy's combobox dialogs and Turbo client; full keyboard behavior and exact filter styling remain incomplete.

The board HTML now switches to a filtered grid when Fizzy's filter-active conditions apply. It uses the same SQL filter builder as Rizzy's card API, forces the current board scope, and paginates results. Its form exposes text, status, and sort controls while preserving other active filter fields on submission. `tests/board_filter_html_parity.py` compared unfiltered columns, explicit defaults, Done and Not Now, text search across two pages, oldest sort, and empty results with live Fizzy. Chromium checks exercised search, Done, and page navigation without console errors. Fizzy's full filter picker controls, saved-filter UI, and card preview markup remain incomplete.

`tests/board_settings_html_parity.py` compares Fizzy and Rizzy's New Board and Board Settings pages, form creation, name/access/entropy update, redirect paths, and HTML deletion. `tests/board_access_controls.py` checks selected-first member ordering, people filter and bulk-control markup, a saved private-board membership change, and disabled controls for non-admins. Chromium filtered a four-person list, selected none while retaining the owner, selected all, and saved owner/Bob access without console errors. The pinned Fizzy board controller suite passed 33 tests and 207 assertions. The settings layout, exact switch and avatar presentation, rich-text description editor, and CSRF meta markup still differ. JSON board creation/deletion and column view checks passed after the HTML handlers were added.

`tests/account_settings_html_parity.py` compares the Account Settings browser page, people listing, account-name edit, auto-close form and invalid period against Fizzy. `tests/join_code_html_parity.py` compares the linked invite page, limit editor, valid and invalid limit changes, reset, and QR dialog. Both apps' anonymous QR SVG routes were scanned and matched their current invite URLs; resetting the code changed the image link while the old signed image remained readable. Chromium opened and closed Rizzy's QR dialog with the image loaded, alongside earlier checks of account auto-close selection, the export dialog, navigation home, and the settings → invite → limit editor flow. Both pages retain their existing JSON endpoints. The settings and invite styling remain simpler than Fizzy's.

`tests/account_cancellation.py` covers the owner-only danger zone and cancellation endpoint, single-tenant no-op, immediate loss of HTML, JSON, and personal-route access, exclusion from the session menu and identity account list, day-29 retention, and day-31 deletion. Its populated two-account fixture verifies dependent record and file cleanup plus foreign-key integrity and surviving access to the other account. A fresh paired MySQL `account-cancellation` suite exercises the owner browser flow against pinned Fizzy with multi-tenant mode enabled: the delete action, redirect, recorded initiating owner, immediate identity/menu exclusion and settings denial, and retained account, board, and card all match. The test also caught and fixed Rizzy's redirect on a cancelled account's scoped `/my/identity.json` request, for which Fizzy returns 403. The paired `account-cancellation-retention` suite creates a second account with a board, card, and uploaded card image in each app. The canceled account also has a user avatar, card text attachment, and comment image. At day 29, Fizzy's due-incineration job and a Rizzy restart retain both accounts, their records, and all five file payloads. At day 31, explicit incineration jobs in both apps remove only the canceled account and its board and card. Rizzy immediately removes its four files; Fizzy queues four Active Storage purges, whose serialized jobs remove those files and blobs when performed. The surviving card image and blob retain their original bytes, and its owner's settings API still returns 200 in both apps. The `account-cancellation-concurrent` suite launches two independent Rizzy `incinerate-due` processes against the same due account, holds its cancellation row locked until MySQL reports both workers waiting there, then releases them. Both workers exit successfully; the same scoped deletion, surviving account, and file outcomes match the single Fizzy job. This verifies an overlapping two-worker Rust incineration attempt, not concurrent Fizzy job behavior or multi-host scale. The `account-cancellation-mail` suite confirms Fizzy enqueues one cancellation mail job and Rizzy stores one transactional mail job, retries after a failed send, respects a fresh claim, recovers a stale claim, and delivers a multipart text/HTML message with matching normalized subject and text content. The `account-cancellation-mail-concurrent` variant holds that job's row locked until two independent Rust delivery processes are both waiting; after release, exactly one message is captured, the workers report one and zero deliveries, and a later run sends nothing. Rizzy brands the message and uses configurable `RIZZY_SUPPORT_EMAIL`; it is not a byte-for-byte copy of Fizzy's HTML layout or Solid Queue implementation. Cleanup of other attachment forms, live-worker purge timing, external mail delivery, and multi-host workers remain unverified.

`tests/join_browser_html_parity.py` compares Fizzy and Rizzy's public join page, an existing member's redirect, invalid code and email responses, a new member's email form redirect, magic-code confirmation, verification, name entry, and landing redirect. Chromium followed a real invite through code entry and name setup to the account home with no error overlay. The Rust join page is visually simpler than Fizzy's public layout; the identity setup and sign-in pages still need a full visual pass. `tests/join.py` continues to pass JSON joining, limits, permissions, and a two-process redemption race.

`tests/account_people_html_parity.py` compares the people listing, browser form promotion and demotion, forbidden role changes, browser removal redirect, identity detachment, and rejoining with a new user ID. Chromium exercised the role toggle and confirmed removal cleared the person from the list without an error overlay. `tests/user_deactivation_migration.py` rebuilt an older user table with nullable identity IDs, preserved its foreign keys and User attachment ID, and checked removal/rejoining through the API. A separate populated database migration retained 501 cards and events, six boards and accesses, and had no foreign-key violations. `tests/user_attachables_migration.py`, `tests/notification_source_migration.py`, `tests/join.py`, and `tests/account_import.py` passed after the schema change. Full Turbo form behavior and remote-session disconnection are still incomplete.

`tests/events_html_parity.py` checks the account home timeline, board-filtered events, three activity columns, local-hour placement with a New York timezone cookie, day fragments, column pages, next-day links, and invalid day/column responses against live Fizzy. `tests/events_saved_filter_parity.py` creates board-and-creator saved filters in both apps and checks that the timeline, day fragment, and Added column apply them, ignore conflicting query filters, and reject unknown IDs. It also compares ETag groups for the full page, day fragment, three columns, and user frame, checks 304 responses, verifies equivalent saved and direct filters share a validator while different definitions do not, creates a new event through another Rust process to check invalidation, and checks that a renamed board appears after the one-second HTML cache expires. A browser flow created and published a card, then opened its Added event. Rizzy uses a bounded 301-event day read; busy days fall back to exact counts and at most 100 events per column. The event cards, filter controls, page layout, and complete event descriptions remain simplified or incomplete. The [warm timeline benchmark](bench/results/events-html-etag-cached-final-2026-09-25.json) shows faster Rust reads on this fixture, but the source page is much larger and complete page parity remains unverified.

`tests/users_events_html_parity.py` checks the user activity Turbo frame against live Fizzy: the self heading, a newly published event, a next-day link back to the user route, and invalid user/day inputs. `tests/user_profile_html_parity.py` checks the browser profile, edit form, name-change redirect, invalid name, and JSON name after saving. Chromium loaded both pages with the user timeline and no error overlay. Profile avatar upload, the full email-change presentation, passkey control styling, theme/transfer panels, and exact Turbo behavior are still incomplete; the existing JSON avatar flow passed its cross-app regression check after form updates were added.

`tests/email_change_html_parity.py` checks the profile edit link, browser email-change form, invalid/same-email redirects, and missing user against live Fizzy. Rizzy's development-mode valid form renders the local confirmation page. `tests/email_confirmation_html_parity.py` checks the browser confirmation and expired-link pages against Fizzy, plus Rizzy's valid-token redirect, new session, and one-use behavior; Chromium verified the expired-link screen after automatic form submission. `tests/email_change.py` still passes JSON confirmation, expiry, one-use, and session-switch checks. Browser flash alerts, exact confirmation styling, and real mail delivery configuration remain incomplete.

`tests/auth_format_parity.py` checks Fizzy's bearer-token format restriction, write-method rejection for read tokens, the exact HTTP Token denial, and unauthenticated redirect behavior against both running servers. Rizzy now requires JSON for bearer access and redirects missing credentials to `/session/new`. The sign-in HTML is simplified. Browser and Turbo tests use session cookies; bearer tokens in those formats are rejected.

`tests/request_forgery_parity.py` compares the pinned source's header-only forgery decisions with Rizzy on twelve JSON write cases: absent, same-origin, same-site, cross-site, invalid, and HTTPS `Sec-Fetch-Site` conditions plus correct and incorrect `Origin`. It also checks a same-origin cookie-backed form, compares the exact 422 HTML response for a cross-site form, and verifies that the source's anonymous unsubscribe action remains exempt. The pinned source forgery suite passed 4 tests and 20 assertions. Rust's local HTTP and forwarded HTTPS form probes verified allowed writes and rejection without a valid fetch-site header. `tests/board_settings_html_parity.py` still passes through browser-style form creation, update, and deletion after the middleware change. This matches the source's present `header_only` strategy on the tested paths; the broader cross-origin JavaScript response check and full reverse-proxy deployment behavior still need verification.

`tests/search_engine_indexing_parity.py` compares Fizzy and Rizzy's `X-Robots-Tag: none` header on anonymous sign-in, authenticated JSON boards, and a freshly published public board, plus the source's header behavior on the service worker, client configuration, health, and static error CSS routes. All seven route types matched live. The pinned source indexing suite passed 3 tests and 8 assertions. Other controller responses and mounted storage routes have not been exhaustively compared.

`tests/allow_browser_parity.py` compares the pinned Rails modern-browser thresholds against live Fizzy on Chrome, Firefox, Safari, Opera, Internet Explorer, older Edge, two bot agents, Baidu, classic Opera, and iOS Safari/Chrome. It also checks the exact 406 HTML body, cache and indexing headers, private-route authentication precedence, the generated signup redirect, exempt health and CSS paths, and rejection of an old-browser board write without a database change. All 27 cases matched Rizzy. The pinned source browser suite passed 6 tests and 18 assertions; the request-forgery and search-indexing comparisons still pass after the middleware ordering changes. This covers representative user agents; full `useragent` gem classification and every controller callback combination remain to be compared.

`tests/platform_html_parity.py` compares Fizzy and Rizzy full-page body attributes for desktop, mobile web, native iOS and Android, empty and multiple bridge-component lists, and an `x_user_agent` cookie overriding the request header. It also checks the public sign-in and published-board layouts, which carry `data-platform` without application-layout bridge attributes, and the application-layout device-transfer page. The board page now returns a weak ETag: both apps returned 304 when the user agent changed and 200 with a new ETag after a board rename. All fourteen live cases matched. The pinned source platform model and controller suites passed 10 tests and 19 assertions; Rizzy's PWA registration regression still passes. Native bridge controllers, title/inset behavior, and complete visual layout remain incomplete. The Rust ETag is computed after rendering, so it does not yet avoid the board query and template work on a 304.

`tests/timezone_etag_parity.py` compares the source's timezone-cookie validator behavior with Rizzy on initials avatars, unfiltered board HTML, and the notification tray. Both apps returned 304 for an unchanged timezone, 200 with a new ETag after switching to New York or Los Angeles, and the original ETag for an invalid timezone. The pinned source timezone controller test passed 1 test and 3 assertions. Rizzy now uses the pinned ActiveSupport mapping of 154 named timezone aliases for ETag validation, activity days, and notification-mail display times. `tests/source_timezone_alias_probe.rb` confirmed on a disposable Fizzy database that aliases have distinct board validators from their equivalent IANA names, and `tests/timezone_aliases.py` checks those decisions in Rizzy, including 304 reuse and a raw cookie alias containing `&`. A live card-page probe found that Fizzy accepts that raw alias and renders it in the timezone meta tag; Rizzy's decoder now preserves the ampersand rather than treating it as a form separator. All 154 aliases resolve in a Rust unit test; the mail delivery and unit suites pass. The full avatar upload comparison, platform HTML comparison, other conditional endpoints, and every timezone-dependent presentation remain to be compared.

`tests/access_token_html_parity.py` compares the profile link, personal access token index, new form, browser creation redirect, short-lived reveal, list without the secret, and browser revocation against live Fizzy. It also verifies that a revealed read token works and stops working after revocation. Rust retains token hashes for authorization; a separate reveal record holds the new token for ten seconds, is bound to the creating identity, and is removed after expiry. The page styling remains simpler than Fizzy's.

`tests/session_html.py` checks the local browser form flow from email entry through a development magic code, `/landing`, the account menu, and account Home. Chromium also submitted a valid six-character code through the magic-link page's paste handler with a CSP nonce and reached Home. JSON session requests remain supported. The paired MySQL `session-menu` suite compares one-account redirect, two-account links and landing navigation, and the empty-account chooser against pinned Fizzy. The public chooser now follows the source's popup structure on desktop and mobile. Account selection beyond these states and passkey presentation still need work.

`tests/magic_link_mail_paired.py` renders sign-in and sign-up messages from the pinned Fizzy mailer and captures Rizzy's real sendmail output for the matching flows. After normalizing the generated code, server origin, and Fizzy/Rizzy name, both parsed messages match in subject, complete plain text, and HTML content and layout. Rizzy now sends multipart verification mail, puts the code in the subject, distinguishes sign-in from sign-up, and includes Fizzy's passkey hint when the identity has an account. Its support address is configurable through `RIZZY_SUPPORT_EMAIL`; MIME transport encoding and multi-account identity cases remain to be compared.

`tests/export_mail_paired.py` now compares the pinned Fizzy user and account export completion mailers with messages captured from Rizzy's actual asynchronous export worker. Both kinds match subject, full plain text, and HTML content after normalizing brand and generated record IDs. The test also follows each emailed Rizzy link with the owner token and confirms it resolves to the completed export. Mail transport encoding, live delivery infrastructure, and other lifecycle mailers remain open.

`tests/import_mail_paired.py` compares the pinned import mailer with Rizzy's actual import worker for a completed account import, a duplicate-account conflict, and an invalid ZIP. It generates a real account export ZIP, imports it, repeats it, and submits invalid bytes. The resulting multipart messages match subject, complete plain text, and HTML after normalizing the brand and imported account slug; the success message links to the new account's landing route. Generic and insufficient-storage failures, mail transport encoding, and live mail delivery remain unverified.

`tests/cancellation_mail_paired.py` creates an owner cancellation through Rizzy's route, drains its queued mail job, and compares the parsed multipart notice with the pinned Fizzy cancellation mailer. Subject, complete plain text, and HTML content match after normalizing the brand; a second drain sends no duplicate. The existing cancellation lifecycle check covers access removal and later incineration. Live mail delivery and the full range of account names and cancellation times remain unverified.

`tests/email_change_mail_paired.py` compares the pinned UserMailer with a Rizzy message captured from a real email-change request. Both send a single HTML part with the same subject, styled confirmation link, safety text, and footer after normalizing generated user IDs, tokens, and origins. The paired test opens the emailed link, confirms the change, and checks the stored identity address. Other account states and live mail transport remain unverified.

Two-account Chromium captures at 1280 × 633 are [Fizzy](docs/visual-baselines/fizzy-session-menu-desktop.png) and [Rizzy](docs/visual-baselines/rizzy-session-menu-desktop.png); 390 × 844 captures are [Fizzy](docs/visual-baselines/fizzy-session-menu-mobile.png) and [Rizzy](docs/visual-baselines/rizzy-session-menu-mobile.png). The desktop panel, heading, and menu bounds match within one pixel; on mobile the panel and menu bounds differ by about two pixels because the brand text has a different width. The footer copy remains intentionally distinct, and the captures cover only dark mode and this account fixture.

`tests/signup_html.py` checks the browser signup route, sign-in page link, email form, magic-code redirect, completion form validation, account creation, and landing redirect on a fresh Rust database. The pinned source's targeted signup, completion, and magic-link controller tests passed (23 runs, 94 assertions); the disposable multi-tenant source and Rust flow is compared in `tests/mysql_pair_selfhost.py signup-browser` below. A rollback-only source signup probe established the initial data counts: one Playground board, 11 published cards, 11 publication events, 11 creator watches, and one board access. Rizzy now seeds those records, and the test matches every starter card's title and plain text against the source. Chromium followed email, code, and name entry to Playground and rendered the orientation video card. `tests/auth.py` also checks that an authenticated identity can create another account through completion, as the source controller permits. Starter card HTML and other source signup side effects have not been exhaustively compared.

Repeated signup completion now numbers account names for one identity, following Fizzy's first, second, and third account pattern while using the Rizzy brand. `tests/signup_account_names_paired.py` signs in to disposable source and Rust SQLite apps, completes three accounts with one first name and one with another, and compares the stored names after normalizing the brand. It also sends eight simultaneous completions for the same identity through two Rizzy processes; all returned 201 and received distinct names in order. A separate fresh MySQL 8.4.11 two-process, eight-request run had the same result after adding an identity row lock around name selection and account creation. The disposable paired MySQL browser signup suite, Rust browser signup and auth suites, and 24 Rust unit tests pass. More complex names, account imports, and multi-host completion remain open.

`tests/json_suffix_parity.py` compares selected collection and item `.json` URLs, a configuration route, pagination links, token denial, and board creation/deletion with an HTML Accept header. `tests/column_position_parity.py` compares left and right column moves, no-op boundary moves, and an unknown ID using disposable boards. Board columns now show add and edit dialogs with Fizzy's nine named colors, an options menu with left/right moves and delete confirmation, and a maximize link. Chromium added a Lime column, moved Beta left while preserving its expanded state, renamed and recolored it, canceled and then confirmed deletion, and saw its card return to Maybe. The JSON column order matched the browser after the move. A fresh paired run of `tests/column_position_parity.py` and `tests/board_filter_html_parity.py` passed. Add and edit now apply the Rust Turbo Stream response in place; a fresh Chromium session kept its loaded cards, expanded column, and page session while adding, moving, and renaming columns. Keyboard selection and focus restoration still worked after a column replacement. After a reload, the same order and colors came from the server. The paired `tests/column_turbo_parity.py` still passes. Delete follows Fizzy's redirect by reloading the board. The menu presentation and some morph details remain different; format handling beyond the tested paths remains incomplete.

`tests/step_read_parity.py` compares step detail JSON, edit JSON rejection, session-backed step and edit pages, updates, and missing-step responses against Fizzy. The Rust edit form was exercised in a browser. `tests/steps_turbo.py` checks source-style Turbo Stream actions and targets for form-backed step creation, update, and deletion, plus JSON create/update/delete and completed-state persistence. The pinned Fizzy step controller suite passed 9 tests and 66 assertions. Rizzy now renders the `new_step_card` insertion target on the card page and accepts form checkbox values; its full step markup, bubble contents, and browser behavior still differ from Fizzy's.

Steps now store their own update times. `tests/step_record_migration.py` checks the backfill for existing databases and old-form inserts; `tests/account_transfer.py` checks that an edit changes the step time and that create and edit touch a published card and its board. Account import refreshes step update times and export uses the stored value. The small transfer fixture checks a step through import and re-export. A committed `tests/source_step_touch_probe.rb` run against a disposable copy of the pinned Fizzy database established create, edit, repeated no-op edit, and delete timestamp effects on published and draft cards: both types touch the card, only published cards touch their board, and a no-op changes neither. Deleting a published card does not touch its board. Rizzy now applies that board rule to card inserts and updates and removes the old delete trigger; `tests/card_board_touch_migration.py` checks the upgrade of existing triggers, draft step actions, publication, published step deletion, and card deletion. Other step side effects and complete browser behavior remain to be compared.

`tests/self_assignment_parity.py` compares assignment and unassignment of the current user, including the card's returned assignee list and an unknown card response.

`tests/draft_workflow_parity.py` signs in through magic links on both servers and compares browser draft creation and resumption, draft redirects, JSON editing, publication, search indexing, activity creation, creating another draft, the initial board column structure, board display after publication, Lexxy and prompt markup, and the published card page's title, description, board, and comment content. Rizzy serves the same Lexxy 0.9.32 browser component as Fizzy for draft editing and on its published card page. Browser checks confirmed formatted text and `@` mentions survive draft save/reload, `#` card references insert and survive reload, and published card edit, comment, pin, and completion controls work. A Lexxy browser upload used Fizzy's Rails Active Storage module, saved and reloaded a text attachment, and verified its original bytes and Card embed record in Rizzy. A 24-card board fixture checked Rust's Maybe, named, Not Now, and Done placement, golden ordering, first-page counts, loading the rest, draft navigation, and adding a column. The Rust HTML design and interaction details are still much simpler than Fizzy's; board and card views are not visually equivalent.

`tests/mysql_draft_cluster.py` runs that draft flow on MySQL and passes a draft
between two Rust app processes for edit and publication. It also checks 32
concurrent publishes on separate boards, with published cards visible from the
other process. The pinned source draft and publish controller suites passed 5
tests and 36 assertions. `tests/mysql_pair_selfhost.py draft` now runs the
browser draft flow on fresh Fizzy and Rizzy MySQL databases and matched all 11
workflow summaries, including magic-link sign-in, draft resumption, Lexxy
markup, publication, search indexing, activity, and add-another redirects. A
fresh paired SQLite regression also passed after making this comparison
reusable. The separate Rust cluster fixture covers same-board publication;
source comparison for concurrent publication and the full browser interaction
surface remain open.

`tests/card_image_parity.py` compares the card background image path against both servers: authorized direct-upload metadata, checksum-backed file transfer, attachment during creation or update, duplicate-use denial, private and public blob access, retrieved bytes and content type, and image removal. `tests/storage_cleanup.py` checks that Rizzy removes blob records and bytes after draining queued purges for image replacement and card or board deletion. `tests/embed_parity.py` compares uploaded image and plain-file embeds in card descriptions and comments, placeholder handling, attachment status, normalized rendered HTML, private/public blob access, download disposition, preview generation, and cleanup after card, comment, and board deletion. Fizzy's signed URL and representation token formats are not reproduced; image preview bytes also differ from its libvips encoding. The signup Playground renders its source-hosted video and image attachments. General remote embeds, other media forms, and exact browser upload presentation remain incomplete.

`tests/card_image_purge_parity.py` compares deferred cleanup after image
replacement, explicit removal, card deletion, and board deletion on fresh paired SQLite
databases, including queued-job state before draining and file bytes after.
`tests/storage_cleanup.py` also checks Rizzy's queued cleanup after card
deletion. `tests/mysql_pair_selfhost.py card-image-purge` passed the same
paired deferred cleanup cases on disposable MySQL databases. Concurrent
card-image purge paths remain to be checked.

`tests/embed_purge_parity.py` and `tests/mysql_pair_selfhost.py embed-purge`
compare the staged Action Text lifecycle on SQLite and MySQL: removing a JPEG
embed queues its original while retaining two variants, draining that job queues
the variants, and later drains remove their bytes. A blob shared by a card and
comment is queued only after the last attachment is removed; board deletion
also queues its embed. A queued purge leaves an embed intact if it gains a new
attachment before the job runs, and a later last detach queues it again. Both
paired suites passed that case. `tests/embed_parity.py` drains the queue before
asserting eventual file cleanup. The two-process tests above exercise Rust
concurrent deletion and worker cleanup; equivalent concurrent Fizzy behavior
and independent-host worker coordination remain unverified.

The [two-round JPEG embed-removal comparison](bench/results/embed-removal-deferred-2round-2026-09-27.json)
timed 12 last-attachment removals per app per round, reversing app order on
fresh SQLite accounts with two server CPUs and paused purge workers. Rizzy's
median PATCH latency was 0.73–0.82 ms versus Fizzy's 27.34–31.75 ms. Both
queued all 14 originals including warmups, retained 28 derived variants,
returned the same JSON field names, and agreed on the removed description and
attachment state; the Rust response was about 20 bytes shorter. This is a
single-client request latency comparison for one operation, not evidence of
maximum throughput or whole-app scale.

Fresh `tests/mysql_pair_selfhost.py card-image` and `embeds` runs now matched
the pinned Fizzy app on disposable MySQL databases. The image test also read
both databases around removal: card and board update times changed while the
card's last-activity time did not, matching Fizzy. The embed fixture checked
upload and preview bytes, permissions, and normalized Action Text HTML. Its
direct SQL and filesystem cleanup assertions were skipped on MySQL; the
separate Rust-only MySQL storage test covers selected cleanup cases.

`tests/card_update_multipart.py` exercises the pinned CardsController's multipart image update path with a session cookie on published and draft cards, including a browser-style POST with `_method=patch`. It checks both pages' file forms and the remove-image control after upload. Rizzy streams uploaded bytes to storage while computing the checksum, attaches the blob in the card transaction, and emits the three source Turbo Stream targets; the draft response omits reactions. The pinned source CardsController suite passed 31 tests and 170 assertions, including these update cases. Chromium selected `moon.jpg` on a published card and confirmed the POST, reloaded image, and loaded bytes. On a draft, it confirmed autosave PATCH before image POST, a persisted unsaved title, the loaded image, and successful removal without browser errors. Rizzy's multipart card route has a 64 MiB request limit, and its Turbo replacement markup uses the Rust card presentation rather than Fizzy's exact partials. Other upload limits, validation responses, and browser interactions still need comparison.

`tests/avatar_parity.py` compares public initials and conditional responses, direct-upload attachment, public original blob and 256px JPEG/PNG thumbnail access, replacement, invalid SVG rejection, and removal. It also checks animated GIF and WebP avatar uploads and thumbnail responses on fresh paired servers. The avatar, representation, and original-blob redirects carry Fizzy's `text/html; charset=utf-8` content type. The shared representation renderer uses libvips' centered thumbnail operation for fill variants; the paired test checks exact SHA-256 equality of the JPEG, PNG, GIF, and WebP thumbnail bytes. A variant record and derived blob exist immediately after avatar attachment, with the source variation digest, dimensions, byte count, and checksum. The test verifies those side effects before the first thumbnail request. Fresh paired SQLite and `tests/mysql_pair_selfhost.py avatar` runs passed. The profile editor uses the bundled Active Storage direct-upload module to select and preview an avatar, submit its signed ID, and remove the avatar. Chromium exercised upload and removal on a fresh account, checking that the owner remained active; `tests/profile_avatar_form.py` repeats the form and deletion checks against a local Rust server. The legacy `.thumb` file still uses Rust's image encoder, and exact output for other dimensions and source files remains unverified. Identity avatars and exact profile-editor presentation remain incomplete.

The [current deferred-purge avatar attachment benchmark](bench/results/avatar-attach-deferred-2round-2026-09-27.json) used two fresh SQLite rounds in reversed app order, 12 replacements per app per round, two server CPUs, paused purge workers, and equal 70,367-byte PNG thumbnails. Rizzy's median PATCH was 41.78–42.26 ms versus Fizzy's 66.62–70.51 ms; first anonymous thumbnail retrieval was 1.4–1.5 ms versus 16.19–17.74 ms. The artifact records the committed Rust revision and release-binary hash. The [earlier immediate-purge run](bench/results/avatar-attach-eager-2round-2026-09-27.json) is retained for comparison. The [two-round warmed retrieval benchmark](bench/results/avatar-thumbnail-exact-2round-2026-09-27.json) used the pre-deferred-purge build with equal thumbnail bytes and reversed order. At 32 clients, Rizzy completed 2,095–2,098 requests/s versus Fizzy's 491–494; at 64 clients, 1,832–1,860 versus 552–555. All eight phases had zero HTTP errors. These are single-host, one-avatar measurements, not a maximum-capacity or whole-app scale claim.

The fresh SQLite replacement probe found that Fizzy retains the old original and derived blobs while queuing an `ActiveStorage::PurgeJob`. `tests/avatar_purge_parity.py` and `tests/mysql_pair_selfhost.py avatar-purge` now compare the staged lifecycle: replacement detaches the old avatar, the parent purge removes its original and queues the derived blob, and the second purge removes that derivative. Both paired tests pass on disposable databases. `tests/avatar_purge_workers.py` checks that two Rust processes drain three originals and three derivatives, recovering an abandoned claim. Rizzy uses its own durable queue rather than Solid Queue's exact job schema and serialization. Card-image and Action Text staged purges are covered for the cases described above; other attachment types and concurrent races remain open.

`tests/rich_text_parity.py` compares common Action Text card descriptions, comments, and published board descriptions against Fizzy: plain-text extraction, safe formatted HTML, paragraph/list breaks, automatic links, retained safe class attributes, and removal of unsafe attributes and protocols. It does not cover the full Rails sanitizer allowlist, all mention forms, general remote/media embeds, or every HTML edge case. `tests/embed_parity.py` covers the tested uploaded image and plain-file forms. `tests/search_rich_text_parity.py` checks that formatted card/comment text and attachment filenames are searchable while HTML tags, attributes, and entities are not; it also checks edits, deletes, and the 32 KiB indexed-content limit against both servers. An existing-database migration was verified with a preexisting formatted card. The browser search page now renders separate card and comment hits with escaped highlights, excerpt clipping, numeric-card forwarding, blank and no-match states, and automatic pagination links; `tests/search.py` exercises these alongside the unchanged deduplicated JSON route. `tests/search_html_parity.py` creates equivalent live fixtures on both apps and compares hit counts, card versus comment rendering, snippets including fallback truncation, escaping, JSON deduplication, invalid page values, and pagination. The pinned source search controller suite passed 12 tests and 96 assertions. The [current 100-card search run](bench/results/search-html-100-page-first-2026-09-25.json) and [1,000-card run](bench/results/search-html-1000-page-first-2026-09-25.json) match result counts over four pages and report faster Rust reads on those fixtures; the source and Rust HTML sizes differ considerably, and full ActiveSearch query grammar, exact visual and Turbo behavior, and large-scale indexing remain incomplete. `tests/smoke.py` sends an encoded literal angle bracket to reflect the source API's HTML-input semantics.

`tests/rich_text_allowlist_parity.py` now compares 45 card descriptions against disposable Fizzy and Rizzy accounts. A first probe differed in 15 of 30 cases; after extending the pinned tag and attribute allowlists and following Action Text's plain-text rules for figures, blockquotes, and nested lists, the expanded 45-case suite matches. The cases include tables, details, semantic inline tags, images, video, CSS properties, rejected CSS URLs and expressions, and malformed or disallowed markup. A further 27-case probe found that Fizzy retains PNG data images, CSS variables, and rounded borders where Rizzy did not. Rizzy now permits only Fizzy's tested data URL media types, including PNG, JPEG, GIF, CSS, and plain text, while the paired suite confirms rejection of SVG and HTML data URLs. The older common rich-text comparison and formatted search-index comparison still pass live. Ammonia's CSS parser and Rust's conservative function filter are not a complete port of Loofah's CSS value sanitizer; uncommon border-prefixed properties and nested URL fallbacks still differ, and untested HTML, CSS, and attachment forms remain open.

The [1,000-card, 64-client mixed API repeat](bench/results/sustained-mixed-1000-64-rich-text-2026-09-26.json) pins the broader rich-text build. In two reversed-order 30-second rounds, Rizzy completed 124.27–124.59 units/s versus Fizzy's 19.79–23.62, with zero HTTP errors and expected final record counts. Fizzy's throughput differed from the earlier artifact, so this verifies the current build's advantage on that workload without attributing a speed change to the sanitizer.

`tests/search_html_parity.py` now also checks Fizzy's ASCII-only query sanitization with accented and non-Latin input against disposable source and Rust databases. A search for `café` finds the separate `caf` card and highlights `caf`; a query containing only `東京` has no terms. This closes those Unicode query cases, but does not establish parity for the full search grammar, ranking, or indexing at scale.

The same live test now covers an embedded-quote query in HTML and JSON search that previously matched a card only in Rizzy. Rust now follows ActiveSearch's SQLite term parsing: a quote inside a word stays in that term, and an empty quoted term is preserved. A deterministic 180-query differential probe using punctuation, quotes, accented text, and nine search terms found one result difference before the fix and none after it. That fixture does not cover all query combinations, result ranking, or scale.

`tests/search_snippet_differential.py` starts disposable signed-in Fizzy and Rizzy servers with the same card fixture and controlled creation times. All 259 query cases now return matching result titles and excerpt text. Rizzy uses SQLite FTS5 `snippet` and `highlight` for browser excerpts, matching the pinned ActiveSearch adapter's 20-token window and match markers; its previous regular-expression excerpt differed on long descriptions. Both HTML and JSON search now apply the source's leading-integer card lookup before full-text search, including queries such as `123*`, `123|beta`, and `1_2`. An additional 733-query probe found five browser mismatches caused by reversed equal-time hit order. Rizzy now uses ascending hit ID as the tie order, and the permanent test compares the exact four-page sequence of 110 equal-timestamp hits. The expanded probe then found zero mismatches. New API-created cards and comments now store microsecond creation times, as Fizzy does, while their JSON retains milliseconds. The test verifies 40 rapidly created hits of each type have unique stored times and the same two-page order in both apps. Existing records created before the precision change can still have millisecond storage, so broader ordering parity remains open alongside query grammar, markup, and browser interactions.

`tests/timestamp_precision_parity.py` compares a supplied six-digit timestamp through card creation, comment creation, and card update on fresh Fizzy and Rizzy databases. Both preserve the microsecond instant in storage and return the same three-digit JSON value. The Rust timestamp parser no longer truncates supplied microseconds, and card `last_active_at` JSON now formats stored microseconds to the source's millisecond precision. `tests/card_timestamps.py`, the equal-time card listing, card update-touch comparison, search differential, and account export/import passed after this change. Preexisting millisecond rows and other record types have not been migrated to six-digit storage.

The [post-snippet 1,000-card, 200-comment search run](bench/results/search-html-1000-sqlite-snippet-2026-09-27.json) matched result counts and title multisets across four pages. On the same eight server CPUs, eight Fizzy workers and one release Rizzy process served 488 versus 350 first-page requests/s at eight clients and 457 versus 390 at 32; on page four, they served 310 versus 222 and 298 versus 233. All two-second phases had zero HTTP errors. Fizzy's first-page response was about 56 KB while Rizzy's was about 9 KB, so this is a faster read of the current Rust page, not evidence of equal rendering work or greater application-wide scale. The later card-number coercion change does not affect this benchmark's nonnumeric query.

The [microsecond card/comment creation build recheck](bench/results/search-html-1000-microtime-2026-09-27.json) used fresh 1,000-card, 200-comment fixtures, eight Fizzy workers, one release Rizzy process, and eight shared CPUs for both servers and the sequential load client. It records the pinned Fizzy revision and Rust binary SHA-256. Counts and title multisets still matched over four pages, and every two-second phase had zero errors. At 8 and 32 clients, Rizzy served 495 and 460 first-page requests/s versus Fizzy's 360 and 384; on page four it served 314 and 301 versus 227 and 240. The first-page HTML remained about 9 KB versus 56 KB, so this verifies no obvious read-throughput regression on the current route; it does not establish equal-work speed or maximum scale.

The search page now follows more of Fizzy's rendered structure: skip link and main landmark, the form's Turbo attributes and clear control, result link and timestamp attributes, and the automatic-pagination frame and link. `static/search.js` clears and refocuses the field, localizes result times, and loads the next page on intersection. Comment hits now include the card creator's avatar and a matched image endpoint; the page queries creator data with the selected hits rather than per result. The paired `tests/search_snippet_differential.py` checks the structural markers and avatar image response on a populated page; `tests/search_html_parity.py` covers empty, card, comment, escaping, excerpt, and pagination cases. Chromium showed 15 results initially, loaded all 40 across two frames after scrolling, cleared and focused the input, and opened a result card without page or console errors before the avatar change. Fizzy's full application shell, exact Turbo behavior, and styling remain incomplete.

The [source-form search read recheck](bench/results/search-html-1000-source-form-2026-09-27.json) uses a fresh 1,000-card, 200-comment fixture, the same eight CPUs for both servers and the sequential load client, eight Fizzy workers, and one release Rizzy process. It records the source revision and Rust binary hash and passes the four-page count/title gate. At eight and 32 clients, Rizzy served 462 and 442 first-page requests/s versus Fizzy's 341 and 340; on page four, 304 and 292 versus 224 and 236. All phases had zero errors. Rizzy's first-page HTML grew to 17.9 KB but remained smaller than Fizzy's 56.3 KB, so the measured lead still does not establish equal-rendering-work speed or maximum scale.

After the comment-avatar change, the [fresh two-round search benchmark](bench/results/search-html-1000-avatar-fresh-2round-2026-09-27.json) repeated the 1,000-card, 200-comment fixture with a reproducible `--fresh` harness and reversed app order in round two. Four-page result counts and title sets matched, and all 16 two-second phases had zero HTTP errors. At 32 clients, Rizzy returned 468 first-page requests/s in both rounds versus Fizzy's 353–393 and 272–293 fourth-page requests/s versus 223–241. Its corresponding p95 ranges were 80–85 and 147–153 ms versus Fizzy's 131–148 and 216–225. The two apps still render different HTML sizes and incomplete Rust presentation, so this is evidence for the current read route on one host, not product-wide speed or maximum scale.

The published card now links each step's text to its edit route and opens an inline rename/delete form on click. The card renders Fizzy's ordered step list, a Turbo frame per step, a POST form with `_method=patch` and the hidden unchecked value, and a source-shaped new-step form. The same frame renderer supplies Turbo replacements and the direct step read. Direct show and edit pages use the signed-in shell; the show response has Fizzy's nested frame, and edit has separate PATCH and DELETE override forms. `tests/step_read_paired.py` starts disposable Fizzy and Rizzy servers and compares those structures, JSON values, and Turbo form rename/delete responses. `tests/steps_turbo.py` checks the Rust card markers and form-style POST toggle as well as JSON and Turbo mutations. The card browser client now fetches the rendered edit frame and applies create, update, and delete Turbo Streams instead of constructing its own editor and sending JSON mutations. `tests/step_browser_paired.py` exercises edit, Escape cancel, save, completion, creation, and deletion in both real browsers and verifies that each remains on the card URL. Its Rust check keeps typed text and focus through a concurrent API edit and live card refresh; Escape then shows the remote value. The Rust client still implements Turbo frame behavior itself and the card styling remains different, so full frame, focus, and visual parity remain open.

The fresh 1280 × 633 [Fizzy card](docs/visual-baselines/fizzy-card-step-frame-1280.png) and [Rizzy card](docs/visual-baselines/rizzy-card-step-frame-1280.png) show the current step-frame rendering with two matching step texts. Rizzy now hides the desktop back arrow, gives watched/pinned side controls the source's light/dark states, dims the disabled add-step checkbox, and places the tag control and steps closer to the source. The paired browser suite caught an edit-open race immediately after completion: the pending checkbox stream could replace the frame being edited. Rizzy now waits for that frame's mutation before opening the editor, and the suite passed again. Branding, some icon and font details, metadata, and mobile presentation still differ.

`tests/draft_image_touch_parity.py` compares image attachment and removal on a browser-created draft using fresh paired servers and SQLite databases. Both apps advance the draft card's update time for each action, leave its last-activity time unchanged, and do not touch its board. This extends the published-card deletion check in `tests/card_image_parity.py`; other attachment callbacks remain to be compared.

`tests/steps_index_etag_parity.py` compares the JSON step index's weak ETag, private cache policy, 304 response headers, wildcard and listed validators, create/edit/delete invalidation, no-op stability, and distinct empty-card validators against fresh Fizzy and Rizzy servers. The [100-step JSON read run](bench/results/steps-index-100-2026-09-27.json) used equal eight-CPU server affinity, eight Fizzy Puma workers, one Rust process, a separate Rust load client, and two reversed-order three-second rounds at eight and 32 clients. It matched step content and completion before timing, observed equal 9,067-byte response sizes, and saw zero HTTP errors. Rizzy served 2,841–2,905 requests/s versus Fizzy's 584–688, with p95 of 3.06–12.45 ms versus 24.44–78.02 ms. This is one read path on one host; it does not establish full application speed or maximum scale.

Fizzy's production JSON response immediately after changing an empty board's entropy can show its cached old value. Its focused controller test expects the new value and passes. The parity scenario checks the saved value after a board update invalidates that cache; Rizzy currently returns the new value immediately.

The signed-in search page now has a source-like dark list layout, centered title and brand, search field, highlighted result titles, and a persistent bottom bar. Paired screenshots use the same disposable search fixture at [1280-pixel Fizzy](docs/visual-baselines/fizzy-search-1280.png), [1280-pixel Rizzy](docs/visual-baselines/rizzy-search-1280.png), [390-pixel Fizzy](docs/visual-baselines/fizzy-search-390.png), and [390-pixel Rizzy](docs/visual-baselines/rizzy-search-390.png). Chromium loaded 15 initial hits at both widths with no horizontal overflow or page errors, cleared and refocused the input, and previously appended all 40 hits on scroll. Search now shares Home's interactive menu, pins tray, notification tray, and footer search modal. Chromium opened and populated each control, switched between trays, kept the main search query intact, and found 15 footer-modal results. The footer CSS is shared with Home, whose layout and controls still loaded in Chromium. The paired 259-query search differential, pin HTML, notification tray HTML, and Home add-card checks passed after the change. Exact Turbo behavior, full application-shell markup, and remaining typography details still differ. The visual comparison covers one search query and two viewports, not the full search interface.

The [post-layout fresh search benchmark](bench/results/search-html-1000-visual-shell-2026-09-27.json) used 1,000 cards and 200 comments, eight shared CPUs, eight Fizzy workers, one release Rizzy process, and two rounds with reversed target order. Counts and title sets matched across four pages, and all 16 two-second phases had zero HTTP errors. At 32 clients, Rizzy served 463–471 first-page requests/s versus Fizzy's 387–399 and 290–291 fourth-page requests/s versus Fizzy's 239–241; p95 latency was 79–84 versus 134–148 ms on page one and 127–149 versus 203–237 ms on page four. Rizzy's first-page HTML was 22,281 bytes versus Fizzy's 56,285, so the result demonstrates faster throughput for this particular search read under different rendering work. It does not establish one-for-one application speed or greater maximum scale.

The [interactive-shell search recheck](bench/results/search-html-1000-interactive-shell-2026-09-27.json) repeated that fresh fixture and two reversed-order rounds after adding the shared menu and footer controls. The first-page HTML is now 49,284 bytes versus Fizzy's 56,285; page four is 108,196 versus 119,994. Four-page result counts and title sets matched, with zero HTTP errors in all 16 phases. At 32 clients, Rizzy served 451 first-page requests/s versus Fizzy's 377–402 and 282–284 fourth-page requests/s versus Fizzy's 234–235; p95 ranges were 83–91 versus 126–147 ms and 133–136 versus 193–235 ms. The work and markup still differ, and this is one read path on one host, so broader speed and maximum-scale claims remain open.

The published card page now uses the same interactive application menu and footer as Home and search instead of its separate JSON-only utility dialog. Its pin and notification trays, footer search, keyboard shortcuts, and menu load through the shared shell. Chromium opened the menu on a signed-in card, pinned that card, found it in the pins tray, then opened footer search and found 15 matching cards without page errors. It loaded the same card at [1280 pixels](docs/visual-baselines/rizzy-card-shell-1280.png) and [390 pixels](docs/visual-baselines/rizzy-card-shell-390.png) without horizontal overflow. `tests/card_controller_html.py` and the Rust unit tests passed after the replacement. The card's detailed layout and lower-page interactions still differ from Fizzy; these screenshots are Rust-only and are not pixel-parity evidence.

A fresh paired card fixture at 1280 × 640 captured [Fizzy](docs/visual-baselines/fizzy-card-notch-1280x640.png) and [Rizzy](docs/visual-baselines/rizzy-card-notch-1280x640.png). Fizzy puts its completion control in a notch below a compact metadata footer; Rizzy now does the same instead of nesting it in the tall footer. The control begins at 674.4 px in Fizzy and 674.9 px in Rizzy. At 390 × 800, paired [Fizzy](docs/visual-baselines/fizzy-card-notch-390x800.png) and [Rizzy](docs/visual-baselines/rizzy-card-notch-390x800.png) screenshots now show Rizzy's full-width card, horizontal stage selector, compact metadata, six mobile actions, and inactivity notice. The notice uses the card's last activity plus the board's effective postponement period and is absent for Done and Not Now cards, as in Fizzy's closure partial. Its completion button begins at about 499 px, matching the source's vertical position; the comment composer now begins at 628 px in both screenshots, with no horizontal overflow. Chromium opened and dismissed the mobile editor, changed the stage to Done and back, and pinned then unpinned a card in a fresh browser session. The source still differs in action icon art, metadata details, card color, and composer styling. `tests/card_controller_html.py` checks the notice deadline and its closed and postponed conditions. The source's exact visual and interaction parity remains open.

Fresh 390 × 800 completed-card screenshots pair [Fizzy](docs/visual-baselines/fizzy-card-closed-390x800.png) with [Rizzy](docs/visual-baselines/rizzy-card-closed-390x800.png). Rizzy now omits the add-step field, renders the Done stamp with the persisted closer and date, displays the completed-by and Undo message, and retains pin and watch. The stage row begins at 231 px, the metadata footer at 281 px, the card body ends at 422 px, the watch/pin controls begin at 438 px, and the comment composer begins at 630 px in the Rust screenshot, close to the source positions. The pinned-source and Rust `tests/closure_record_lifecycle.py` checks both exercise close, repeated close, Undo, and reclose, and compare the completed-card HTML markers. A paired `tests/not_now_record_parity.py` run checks the signed-in Not Now stamp and continued add-step control alongside the already matched public stamp and record lifecycle. Chromium also received a completion from another client and changed the open page to the stamp and Undo message without a reload; clicking the newly inserted Undo restored the add-step field and inactivity notice without browser errors. Icon art, metadata detail, and composer styling remain visibly different.

The [fresh card HTML recheck](bench/results/card-html-1000-interactive-shell-2026-09-27.json) used 1,000 cards per app, eight shared CPUs, eight Fizzy workers, one release Rizzy process, and a signed-in card with the same two steps, comment, tag, card reaction, and comment reaction. The content-marker gate passed and neither two-second phase returned HTTP errors. Rizzy's response was 98,680 bytes versus Fizzy's 73,878. At 32 clients, Rizzy served 5,979 requests/s versus Fizzy's 444, with p95 latency of 8.65 versus 132.32 ms. The [fresh harness](bench/card_html_fresh.py) records the source revision and Rust binary hash; run it with `--cards 1000 --seconds 2 --concurrency 8,32`. This is one warm signed-in card read with different markup and no browser asset or interaction load. It does not prove one-for-one card rendering, sustained whole-app throughput, or a maximum-scale limit.

The [inactivity-notice card recheck](bench/results/card-html-1000-inactivity-notice-2026-09-27.json) extended the sample gate to require the notice and its `indays` time target in both apps at the pinned source revision. With 1,000 cards, eight shared CPUs, eight Fizzy workers, and one release Rizzy process, the two-second 32-client sample measured 6,358 versus 479 requests/s and p95 7.99 versus 119.65 ms, with no HTTP errors. The sampled HTML was 103,645 versus 73,878 bytes; the larger Rust markup reflects remaining presentation differences. This is still a short warm read of one card and does not establish whole-app or maximum-scale superiority.

The [completed-card read](bench/results/card-html-1000-closed-2026-09-27.json) requires the dated completion message, Done stamp, Undo, and no add-step or inactivity notice in both sampled pages. Its 1,000-card fresh fixture used the same eight shared CPUs, eight Fizzy workers, and one release Rizzy process. At 32 clients it measured 5,748 versus 474 requests/s, p95 9.05 versus 128.5 ms, and zero HTTP errors in two-second phases. Rizzy's sampled HTML was 107,907 bytes versus Fizzy's 73,107, so this demonstrates a faster scoped read despite different markup, not full visual parity or sustained scale.

The all-cards page now uses the shared interactive menu and footer too. Chromium checked its menu, text filter, footer search, and a populated pins tray at desktop and mobile widths, with no horizontal overflow at 390 pixels: [desktop grid](docs/visual-baselines/rizzy-allcards-shell-1280.png) and [mobile pin tray](docs/visual-baselines/rizzy-allcards-pintray-390.png). `tests/cards_html.py`, the Rust unit tests, and strict route registration checks passed. The screenshots are Rust-only; the filter controls, card previews, and exact application shell still differ from Fizzy.

The [fresh all-cards HTML run](bench/results/cards-html-1000-shared-shell-2026-09-27.json) used 1,000 cards per app, eight shared CPUs, eight Fizzy workers, one release Rizzy process, and 15 visible previews on each page. At eight clients, Rizzy served 3,430 requests/s versus Fizzy's 318; at 32 clients, 3,042 versus 432. All four two-second phases had zero HTTP errors. Rizzy's page was 64,885 bytes versus Fizzy's 138,208, so this is a scoped read comparison with different rendering work. It does not establish whole-app speed or maximum scale. Reproduce with `python3 bench/cards_html_fresh.py --cards 1000 --seconds 2 --concurrency 8,32`.

The signed-in board page now uses the shared interactive menu and footer on both its column and filtered-grid paths. Its old global card styles are scoped to the board so the populated pin tray keeps the shared dark card presentation. Chromium opened the account menu and populated pin tray, found three cards through footer search, used the `c` shortcut to open a draft, and filtered the board down to one card. The page had no browser errors or horizontal overflow at 390 pixels: [desktop board](docs/visual-baselines/rizzy-board-shell-1280.png) and [mobile pin tray](docs/visual-baselines/rizzy-board-shell-pin-390.png). `tests/board_html_cached_etag.py`, the Rust unit tests, and a fresh paired `tests/board_filter_html_parity.py` run passed. Exact board controls, Turbo behavior, and visual layout still differ from Fizzy.

A same-fixture browser comparison found that Rizzy's board and all-cards filter toggles were absent from the accessibility tree. Both now expose a button with the source's expand/collapse labels and state, work with keyboard Escape, and retain their existing filter submission. At a 390-pixel viewport, opening either panel keeps document width at 390 pixels instead of expanding it to 483 pixels on the board or 425 pixels on all-cards. The all-cards status selection was exercised in Chromium. `tests/paired_board_browser_fixture.py` reproduces the disposable paired fixture. Fizzy's independent quick-filter popovers and Rizzy's grouped fields still differ.

The [fresh signed-in board HTML run](bench/results/board-html-1000-shared-shell-2026-09-27.json) used 1,000 cards per app with 15 visible previews, eight shared CPUs, eight Fizzy workers, and one release Rizzy process. In four two-second phases with zero errors, Rizzy served 4,397 requests/s versus Fizzy's 165 at eight clients, and 4,953 versus 214 at 32 clients. Rizzy returned 90,722 bytes versus Fizzy's 156,477, and its warm board cache and markup differ. This recheck covers one local read path, not equal application work or a maximum-scale limit. Reproduce with `python3 bench/board_html_fresh.py --cards 1000 --seconds 2 --concurrency 8,32`.

The authenticated named-column page now uses the same menu, pins and notification trays, and footer search as the board page. Its grid card styles are scoped to `main` so the shared pin tray keeps the same presentation when opened from a column. A fresh paired `tests/named_column_html_parity.py` run matched the empty, 15-card first, and one-card second page structure; `tests/named_column_show_etag_parity.py` still matched the index/show validators. Chromium loaded the second page automatically, opened the menu and populated pin tray, and found three cards through footer search. The final page had no browser errors or horizontal overflow at 390 pixels: [desktop](docs/visual-baselines/rizzy-named-column-shell-1280.png) and [mobile pin tray](docs/visual-baselines/rizzy-named-column-shell-pin-390.png). The screenshots show Rizzy only; exact card layout and Turbo behavior still differ from Fizzy.

The [fresh named-column HTML run](bench/results/named-column-html-100-shared-shell-2026-09-27.json) used 100 cards per app, 15 visible previews, eight shared CPUs, eight Fizzy workers, one release Rizzy process, and two reversed-order rounds at eight and 32 clients. All eight two-second phases had zero errors. Rizzy served 4,143–4,263 requests/s at eight clients versus Fizzy's 291–408, and 4,916–5,031 at 32 clients versus Fizzy's 379–439. Rust HTML was 59,804 bytes versus Fizzy's 121,093, so this measures different rendering work on one local read path. It does not establish whole-app speed or maximum scale. Reproduce with `python3 bench/named_column_html_fresh.py --cards 100 --seconds 2 --concurrency 8,32 --rounds 2`.

`tests/triage_board_parity.py` now compares Fizzy and Rizzy after a card moves
between boards: both clear its old column, reject the old board's column with
404, accept the destination board's column, and untriage it. The paired column
timestamp test still matches. A fresh four-process MySQL cluster run raced 200
triages and 200 untriages with moves, observed no HTTP 500s, and checked that
no card linked to a column on another board. Other cross-board mutations and
full concurrency parity remain unverified.

Close, reopen, postpone, gild, and ungild now inspect card state after locking
its current board and card. The paired closure, goldness, and Not Now lifecycle
tests still match pinned Fizzy, including repeated requests. A fresh
four-process MySQL cluster run passed 200 flag/move pairs and 200 simultaneous
double-close or double-gild pairs, checking final flags, record counts, and
closure event counts. This verifies those workflows on one host; the other
write paths and a whole-app throughput or scale advantage remain open.

The [post-lock mixed API run](bench/results/mixed-flag-lock-1000-32-30s-2026-09-27.json)
used fresh 1,000-card SQLite fixtures, 32 clients, eight assigned app CPUs,
eight Fizzy Puma workers, one release Rizzy process, and two reversed-order
30-second rounds. Each unit made five JSON reads and four card writes,
including closure. Sampled response content and shape matched before load;
each phase had zero HTTP errors and the expected final card, event, and
comment counts. Rizzy completed 122.99–124.62 units/s versus Fizzy's
28.05–28.64, about 4.3–4.4 times as many on this workload. Rizzy's p95 unit
latency was 487–491 ms versus 1,532–1,555 ms. The artifact records both
revisions, the release binary hash, CPU, and memory samples. This local API
comparison does not establish full application parity or multi-host capacity.

`tests/card_move_sequence_parity.py` matches pinned Fizzy for an image-bearing
card moved from a source board through a middle board to a final board, then
moved to that same final board again. It compares the two event source and
destination names and each board's storage-ledger total. The existing paired
move-access and storage-ledger checks also passed after the Rust move route
began locking and rechecking the card's actual source board. On MySQL 8.4.11,
`tests/mysql_same_card_moves.py` forced two app processes to wait on the same
source board before moving the card to different destinations. Three fresh
runs passed with two accurate move events and all image bytes on only the
final board; the four-process cluster and storage materialization tests also
passed. Other cross-board races, full route parity, and multi-host scale
remain unverified.

Pin/unpin writes now lock and recheck the card's current board and the user's
access before changing a pin. The paired card-control and pin-tray tests still
match Fizzy's HTML, ETag, Turbo, JSON, and unpin behavior; the paired
move-access test now also confirms both apps return 404 when a former member
tries to pin after losing access. `tests/mysql_card_move_access.py` forced a member
pin and an owner move to a private board to wait on the same lock; three
fresh MySQL runs left no inaccessible pin. The four-process cluster passed
200 owner pin/move pairs and found 200 retained pins where access persisted.
This does not establish all concurrent permission behavior or full parity.

Watch/unwatch writes now use the same current-board access recheck. The paired
watch timestamp test still matches Fizzy, and the move-access comparison
confirms both apps return 404 for a watch attempt after access loss. The
private-board MySQL fixture forced both pin/move and watch/move waits in three
fresh runs and left neither personal row after the member lost access. The
four-process MySQL cluster still passed its owner watch/move phase. Other
notification, broadcast, and concurrent permission behavior remains open.

Card edits, tags, steps, comments, assignments, and reactions now recheck the
actor's access inside the locked current-board transaction. Tag creation is
also in that transaction, so a rejected toggle leaves no new tag. The paired
move-access fixture matched Fizzy's 404 responses for post-loss edits, tags,
assignments, steps, comments, and reactions. Two fresh MySQL runs forced an
owner move to wait ahead of member reaction, tag, assignment, and step writes;
all four returned 404 after the move, with no new reaction, tag, assignment,
or step row. Tag, reaction, and comment timestamp comparisons still matched
Fizzy. The MySQL core and fresh four-process cluster passed. These checks
cover selected writes and one board-move ordering, not all permission changes.

Publish, delete, and read/unread card writes now take the same current-board
access lock. The private-board MySQL fixture forced an owner move ahead of a
member creator's delete and reading requests, then ahead of publication of an
actual browser-created draft. Each queued member write returned 404; the
deleted card remained on the destination, and the draft remained unpublished
without a publication event. A paired Fizzy/Rizzy access-loss fixture matched
the ordinary 404 responses for all three routes. The 142-comparison paired
JSON workflow, two-server MySQL draft flow with 32 concurrent publishes,
fresh MySQL core, and four-process cluster passed after the change. Other
access revocations and the full draft/browser lifecycle remain open.

JSON card creation and browser draft creation now lock the destination board
and recheck the creator's access inside the write transaction, before reserving
an account card number. The paired Fizzy/Rizzy move-access fixture matched 404
for both board-scoped and account-scoped JSON create attempts after access
loss. In two fresh MySQL 8.4.11 runs, `tests/mysql_card_creation_access.py`
forced board-access revocation to queue ahead of a JSON create, then repeated
it with an authenticated browser draft create. Each late request returned 404
without inserting a card or advancing the account counter. The SQLite browser
draft flow, paired 142-comparison API workflow, fresh MySQL core, two-server
draft suite, and four-process cluster still passed. Revocation paths outside
this board update and other creation side effects remain open.

The [post-creation-lock mixed API run](bench/results/mixed-create-access-1000-32-30s-2026-09-27.json)
used fresh 1,000-card SQLite fixtures, 32 clients, eight assigned app CPUs,
eight Fizzy Puma workers, one release Rizzy process, and two reversed-order
30-second rounds. Each unit made four JSON reads plus create, edit, comment,
close, and read writes. Sampled response shape and content matched before load;
all four phases had zero HTTP errors and verified final card, event, and comment
counts. Rizzy completed 107.16–123.48 units/s versus Fizzy's 17.07–28.22;
the paired ratios were about 6.3 and 4.4. Fizzy's first phase was much slower
than its second, so this local result is variable. The artifact records the
committed Rizzy revision and exact release-binary hash. It does not isolate
the access-check cost or establish whole-app performance or multi-host scale.

`tests/mysql_pair_selfhost.py signup-browser` now compares multi-tenant browser
signup on disposable Fizzy and Rizzy MySQL databases. It checks the signup
page, invalid email with no new records, identity and magic-link creation,
code verification, blank-name HTML and JSON validation, account creation,
and authenticated signup redirects without another identity or link. The
paired run found that Rizzy's blank-name page omitted Fizzy's `txt-negative`
error class; the form now uses the source's error markup. This suite does not
establish full signup parity, including tenant limit behavior in a fresh
single-tenant run and every completion-page state.

The signup form now uses a source-shaped public shell, centered panel, input,
button, and responsive light/dark styling. Browser comparison at 1280 × 633
and 390 × 844 is preserved in [desktop source](docs/visual-baselines/fizzy-signup-desktop.png),
[desktop Rust](docs/visual-baselines/rizzy-signup-desktop.png),
[mobile source](docs/visual-baselines/fizzy-signup-mobile.png), and
[mobile Rust](docs/visual-baselines/rizzy-signup-mobile.png). At the mobile
viewport the heading begins at the same vertical pixel; the input and button
are within two pixels of the source positions. Rizzy uses its own logo and an
independent-implementation footer rather than claiming Fizzy's branding or
37signals support. The completion form, flash animation, and full light-theme
geometry still need visual comparison.

The public email sign-in and verification pages now share that shell. The
paired `session-browser` suite compares missing pending-authentication state,
one-time alerts, HTML and JSON invalid-code responses, one-time shake state,
valid sign-in, and the global `/landing` to account-menu redirect chain on
fresh Fizzy and Rizzy MySQL databases. The paired `session-rate-limit` suite
checks ten invalid code submissions, the eleventh-request redirect and
one-time alert, and a JSON 429. Both suites passed. Desktop and mobile browser
comparisons are saved for [email entry source](docs/visual-baselines/fizzy-session-new-desktop.png),
[email entry Rust](docs/visual-baselines/rizzy-session-new-desktop.png),
[email entry mobile source](docs/visual-baselines/fizzy-session-new-mobile.png),
[email entry mobile Rust](docs/visual-baselines/rizzy-session-new-mobile.png),
[code entry source](docs/visual-baselines/fizzy-magic-link-desktop.png),
[code entry Rust](docs/visual-baselines/rizzy-magic-link-desktop.png),
[code entry mobile source](docs/visual-baselines/fizzy-magic-link-mobile.png), and
[code entry mobile Rust](docs/visual-baselines/rizzy-magic-link-mobile.png).
The code-entry field and main text positions were within one pixel at desktop
and mobile in the tested viewport. The visible logo and footer identify Rizzy
as independent, while the source shows Fizzy and 37signals. The development
code hint, other form errors, and all sign-in edge cases have not been
exhaustively compared. A paired passkey HTML run also found that one missing
passkey page had an extra dynamic CSP nonce; its static 404 body now matches
the source and that suite passes.

The paired `signup-rate-limit`, `signup-rate-limit-invalid`, and
`signup-rate-limit-json` suites now exercise eleven requests each on fresh
disposable MySQL databases. Fizzy and Rizzy both handle the first ten requests
in the relevant format, redirect the eleventh to `/signup/new`, create no
identity or magic link for the excess request, and display "Try again later."
once on the redirected page. Invalid emails count toward the source's limit,
so Rizzy now checks the signup limit before validation. The MySQL attempt
counter now holds a per-bucket database advisory lock across count and insert;
SQLite begins an immediate write transaction for the same operation. The
`signup-rate-limit-cluster` suite starts two Fizzy and two Rizzy processes on
separate shared MySQL databases, submits one signup then 23 concurrent signups
to each pair, and checks that exactly ten magic links were created and 14
requests were throttled in both. This verifies the tested burst, not every
rate-limit action or distributed capacity under sustained load.

`signup-completion-cluster` now uses four independent signed-in sessions across
two Fizzy and two Rizzy processes. The first paired run created all four Fizzy
accounts but returned two Rizzy database errors because separate processes
selected the same next account number. Rizzy now uses a transactional
`account_external_id_sequences` row for seed, signup completion, and import;
allocation commits before the larger account setup transaction. Three paired
rounds then created 12 distinct accounts numbered 2–13 on each side, with
matching owner, board, card, event, and active-watch counts per account.
`tests/account_sequence_migration.py` passed on both SQLite and MySQL after
dropping the sequence table from an existing account database, restoring it
from the highest existing account number, and retaining a higher sequence
value across another restart. This closes the observed collision and migration
case. It does not establish sustained signup throughput or every concurrent
account setup side effect.

`tests/mysql_pair_selfhost.py route-get` checks the status code and content type
of 92 distinct pinned Fizzy GET paths against Rizzy, using a JSON request and
absent resource identifiers on fresh paired MySQL databases. All 92 matched.
The `route-get-contract` variant also matches redirect paths and queries,
semantic JSON error bodies, and the exact hash and length of non-JSON error
bodies on all 92 paths. The `route-get-html` variant now matches the same
status, content-type, redirect, and error-body contract for signed-in HTML
requests on all 92 paths. These checks cover routing and missing-resource
behavior only. The paired `route-get-valid` suite now creates a board, column,
webhook, card, step, comment, and reaction in each fresh MySQL database, then
matches status, content type, JSON shape, and selected stable values for 30
GET paths with valid fixture resources. The fixture also includes a postponed
and a closed card, a populated tag, and activity and search results. With the same owner email seeded in both databases, the
suite now recursively compares values in the successful JSON bodies except
generated IDs, timestamps, and signing secrets; in-app URL paths are compared
after normalizing generated IDs in paths and query strings. It also checks that the generated webhook
secret has the source length. The stable values include board and card index
titles, the populated Not Now and Closed column titles, board and column
names, column-card placement, card title and board relationship, webhook
destination, step and comment text,
reaction content, and empty delivery lists. The pin, watch, and preview GET
paths add paired format-contract checks for a real card or account. It
found and closed a user-activity format mismatch: a JSON request for an
existing user now returns Fizzy's 406 response. The suite does not compare
full response values or HTML, and most valid-resource routes, complete
permissions, and side effects remain open. The suite now compares total-count,
next-link, and ETag presence on the 30 responses, confirms the exact Rack-style
weak ETag for an identical empty tag body, and exercises conditional `304`
responses on populated user and tag lists. Rizzy adds body-derived ETags to
successful JSON GETs that do not already have a route-specific validator;
the pinned source's Rack middleware uses the same SHA-256 prefix and
cache-control default. The suite now also compares body-derived validators and
private cache control for ten JSON `201` creates, a JSON `200` board update, and
a repeated `PATCH` with `If-None-Match` that must still return `200`. Rizzy's
buffered response middleware covers `200` and `201` JSON and HTML bodies on
all methods, with conditional `304` handling limited to GET and HEAD. Four
paired reaction Turbo Stream create/delete responses also match Rack's body
ETag and cache policy. The existing board and named-column pagination/ETag
checks still pass. Other formats and conditional routes remain to be compared.
The `route-get-valid-html` suite also uses persisted fixtures and a signed-in
browser session to compare status, content type, ETag presence, and resource-text
markers on 30 populated browser GET paths. For non-200 responses it also compares
body length and digest. The suite makes conditional requests for 25 HTML
pages and the steps index. Rizzy now hashes the final HTML bytes after CSP nonce
insertion, supplies Rack-style weak ETags and cache control on buffered HTML
GETs without a route-specific validator, and emits the source's CSP nonce meta
tag on full pages. Card reactions, comment reactions, and comment detail now
wrap their existing Turbo frames in a signed-in full page on direct HTML GETs,
matching the source's document, main, and footer structure and changing CSP
nonce validator behavior. The 59 paired probes and MySQL reaction workflow pass.
The shared page content, styling, and controls still need exact parity work.
It found that Rizzy returned JSON 200 for a browser request to the card steps
index where Fizzy returns HTML 406; the handler now matches Fizzy. It also
found and fixed the same format mismatch on board-access listings and
named-column card lists. The paired step-read, populated JSON, private/public
board-view, and named-column browser suites still pass. These markers
do not establish equivalent page structure, controls, styling, or interaction.
The paired `join-code` flow also checks invalid QR signatures and absent account
prefixes with JSON and SVG Accept headers. Full route parity remains open.

Rizzy's automatic account incineration now runs at minute 16 of local hours 0,
8, and 16, following the pinned source's eight-hour recurrence. A database
slot claim prevents multiple Rizzy processes from running the same scheduled
sweep. The explicit `incinerate-due` command remains available for immediate
job execution; the SQLite cancellation and paired MySQL retention checks use
it to verify day-29 retention and day-31 deletion without waiting for a clock
slot. The schedule check covers all 24 hours at minutes 15–17 and a non-UTC
offset. Recovery after a process exits mid-sweep and live cross-host scheduling
remain unverified.

The remaining recurring cleanup jobs now use the pinned source's clock slots:
magic links and device transfers every four local hours at minute 0, exports
hourly at minute 20, and imports hourly at minute 25. The existing tag,
notification, webhook, and auto-postpone workers also evaluate their slots in
the host's local timezone, matching Solid Queue's unset recurring timezone;
their claim keys use UTC so a repeated local hour has a distinct key. Workers
skip the minute in which they start, as the source scheduler plans the next
occurrence at boot; unused-tag cleanup now checks its exact local 04:02 slot
instead of running any time later that day. Export and import job recovery
still runs independently of cleanup. The source schedule parses to
`0 0,4,8,12,16,20 * * *`, `20 * * * *`, and `25 * * * *`.
`tests/recurring_auth_cleanup.py`, `tests/exports.py`, and
`tests/account_import.py` verify explicit SQLite cleanup and retention.
`tests/mysql_recurring_auth_cleanup.py` verifies stale/fresh magic links,
transfers, completed exports, completed imports, ZIP files, and deletion of an
eight-day-old failed import account while another account survives on disposable
MySQL 8.4.11. Rust tests cover all clock hours, startup-minute skipping, and a
concurrent one-winner SQLite slot claim. `tests/recurring_live_clock.py` also
started three SQLite Rizzy processes with fixed local timezone offsets so the
four cleanup jobs became due at one real UTC minute. All four removed their
expired records, retained fresh records, and wrote the expected UTC slot
claims. `tests/recurring_startup_slot.py` booted inside an export cleanup
minute and observed no cleanup or claim through that minute. These tests
observe the worker clock boundary, while production timezone configuration,
the other recurring jobs' live boundaries, recovery from a process exit
mid-job, and multi-host timing remain open.

The [128-client](bench/results/mysql-mixed-1000-4boards-4process-c128-15s-2026-09-27.json)
and [256-client](bench/results/mysql-mixed-1000-4boards-4process-c256-15s-2026-09-27.json)
MySQL mixed runs used fresh 1,000-card, four-board fixtures, four app CPUs,
four Rizzy processes, four Fizzy Puma workers, and 15 seconds per target.
Rizzy completed 4,044 and 4,125 workflow units at 266.29 and 266.39 units/s,
respectively, with zero HTTP errors and verified final card, event, and comment
counts. Fizzy returned one HTTP 500 in each run from an ActiveSearch MySQL
deadlock; its final counts therefore did not pass the workload gate. These
artifacts show that Rizzy sustained this local workload through 256 concurrent
clients, but neither run supports a paired speed ratio, a maximum-capacity
claim, or multi-host scale. The prior 16-client paired MySQL runs remain the
clean mixed-workload throughput comparison at this fixture size.

A later [16-client two-round check](bench/results/mysql-mixed-1000-4boards-4process-c16-20s-2026-09-27.json) on commit `49fbbb5` recorded one Rizzy MySQL `1213` deadlock during card creation in its second round; Fizzy also recorded an ActiveSearch deadlock in each round. That run fails the error gate and supports no speed ratio. With MySQL deadlock tracing enabled, a fresh [32-client two-round repeat](bench/results/mysql-mixed-1000-4boards-4process-c32-30s-repeat-2026-09-27.json) completed without errors or count mismatches: Rizzy handled 248.02–248.84 workflow units/s and Fizzy 25.48–25.50 on the same local four-CPU, four-process/four-worker setup. A separate Rust-only 90-second, 64-client stress pass completed 16,087 units with verified card, event, and comment counts and no HTTP errors. The intermittent Rust deadlock has not been reproduced with tracing or fixed; transaction lock analysis and repeated write-load reliability remain open.

Commit `dd6617a` retries the complete JSON card-create transaction after MySQL SQLSTATE `40001`, with at most three retries. `tests/mysql_card_create_deadlock_retry.py` forces a cycle between an account row and board row; three runs observed the Rust request receive `1213`, return 201 after retry, and persist exactly one card and publication event with the correct card number. `tests/mysql_card_creation_access.py` and the MySQL core flow also passed. In the [two-round 16-client mixed check](bench/results/mysql-mixed-1000-4boards-4process-c16-20s-retry-2026-09-27.json) at the committed revision, Rizzy completed 270.68 and 259.76 workflow units/s with zero HTTP errors and verified card, event, and comment counts. Fizzy had one ActiveSearch deadlock in each round; MySQL's trace identifies competing inserts into a `search_records_11` full-text document index. This run therefore does not support a paired speed ratio. The retry handles the tested deadlock, but other write paths and sustained multi-host reliability still need validation.

Browser draft creation and resumption now use the same bounded whole-transaction MySQL retry. The forced-lock test runs a second account/board cycle through a signed-in HTML POST and verifies the draft redirect, account card counter, one drafted row, its watch, and no publication event for that draft; both cycles log MySQL `1213`. The MySQL draft cluster flow passed across two processes, including 32 same-board concurrent publishes, and the disposable paired MySQL draft browser flow still matched pinned Fizzy. Other card writes have not received equivalent forced-deadlock coverage.

Draft publication now retries its complete MySQL transaction after SQLSTATE `40001`, at most three times. `tests/mysql_card_publish_deadlock_retry.py` creates a browser draft, forces a board/card lock cycle, observes `1213` in the app log, and verifies that publication returns 201 with one event and its original watch intact. A fresh two-process MySQL draft cluster run also passed its ordinary flow and 32 same-board concurrent publications. This covers the forced cycle and local process concurrency; other publication lock orders, distributed hosts, and sustained reliability remain unverified.

Card updates now retry their entire database transaction after MySQL `40001` as well. `tests/mysql_card_update_deadlock_retry.py` forces a card/board lock cycle; three fresh runs logged `1213`, returned the updated card, and left exactly one title-change event and system comment. Paired MySQL card-edit and rich-text embed workflows and SQLite multipart image/card-edit flows still pass. Other write handlers remain to be checked under contention.

Title edits now insert their event and system comment inside the card update transaction, as Fizzy's save callback does. `tests/mysql_card_title_atomic.py` forces an event-insert failure and a comment-insert failure on separate attempts: each returns 500 with the original title and no title event or comment; a subsequent successful edit stores one of each with matching event/comment timestamps. The existing forced `1213` card-update test and a fresh paired browser/API card-edit suite still pass. Other concurrent edit patterns, post-commit actions, and multi-host behavior remain open.

Card edits now retain Fizzy's pre-lock title snapshot while resolving other omitted fields against the locked row on each attempt, including deadlock retries. `tests/mysql_card_partial_update_race.py` holds a row lock, changes the title and timestamps in a second transaction, then queues a description-only PATCH. Both apps restore the earlier title, preserve the concurrent timestamps, and create no title-change event or system comment. Its explicit-title variant creates one title event and comment in each app, with the earlier title in the event particulars. The paired MySQL card-edit flow, forced title-event rollback, forced `1213` retry, and 23 Rust unit tests pass. In [two](bench/results/mysql-card-partial-lock-500-c16-15s-2026-09-27.json) [reversed-order runs](bench/results/mysql-card-partial-lock-repeat-500-c16-15s-2026-09-27.json) of the 500-card, four-board, 16-client MySQL workload, four processes completed 255.68–262.11 workflow units/s and one process 156.31–201.65, all with zero errors and exact card, event, and comment counts. The repeat's one-process phases were 186.56–188.23 versus 194.65–202.80 before this change; the earlier run's final 156.31 phase remains unexplained. That earlier workload scaled across local processes. These artifacts predate the title-snapshot parity change. An initial current-binary paired two-client run had unequal client-to-board assignments and its ratio is withdrawn; the [corrected run](bench/results/mysql-mixed-matched-boards-500-c2-15s-2026-09-27.json) completed both reversed-order rounds with zero errors, equal board assignments, and verified counts at 142.61–144.17 Rizzy versus 16.75–16.88 Fizzy workflow units/s. A [corrected 16-client run](bench/results/mysql-mixed-matched-boards-500-c16-15s-2026-09-27.json) also completed both rounds with zero errors, equal client-to-board assignments, and verified counts at 291.64–299.81 Rizzy versus 25.82–28.73 Fizzy workflow units/s. Maximum sustainable throughput, multi-host capacity, and full feature parity remain unverified.

The [current clean-tree 16-client MySQL repeat](bench/results/mysql-mixed-clean-500-4boards-c16-20s-2026-09-27.json) used commit `9a181b0`, four boards with 500 starting cards, four app CPUs, four Rizzy processes, and four Fizzy workers. Both reversed-order 20-second rounds matched sampled response shapes and stable content, returned zero HTTP errors, and passed exact card, event, and comment count checks. Rizzy completed 259.65 and 266.88 workflow units/s versus Fizzy's 25.04 and 24.84, or 10.37–10.74 times as many for this JSON read/write mix. The preceding [32-client run](bench/results/mysql-mixed-current-500-4boards-c32-30s-2026-09-27.json) used the same Rust binary and had zero Rizzy errors in both rounds; Fizzy returned one card-edit 500 in its first round, so only its second round passed the paired comparison gate. The [earlier 16-client run](bench/results/mysql-mixed-current-500-4boards-c16-20s-2026-09-27.json) also passed both rounds with the same Rust binary, but its worktree was marked dirty because benchmark error reporting was being edited after the run began. These results cover one selected workflow on one MySQL host. They do not establish full feature parity, maximum sustainable load, or multi-host capacity.

A fresh [32-client MySQL recheck](bench/results/mysql-mixed-current-500-c32-2026-09-28.json) used clean Rizzy commit `6444bdc`, the pinned source, 500 cards on four boards, four app CPUs, four Rust processes, four Fizzy workers, and two reversed-order 15-second rounds. Sampled workflow responses and client-to-board assignments matched. In round one both apps had zero errors and verified final counts; Rizzy completed 125.91 workflow units/s versus Fizzy's 8.2. In round two Rizzy completed 284.13 units/s with zero errors and verified counts, while Fizzy reached 25.21 units/s but returned one card-edit 500 from an ActiveSearch/MySQL deadlock, invalidating that phase as a paired comparison. The large within-app throughput variation and source failure prevent a stable 32-client speed ratio or maximum-capacity claim from this artifact. Neither phase measures multi-host scaling.

The MySQL mixed benchmark now accepts `--warmup-units` to run the same fixed number of complete read/write workflows against each app immediately before its timed phase. It distributes those units evenly across boards and Rust processes, records their counts and errors separately, and verifies total card, event, and comment counts including warmup writes. A [four-client smoke run](bench/results/mysql-mixed-fixed-warmup-100-c4-2026-09-28.json) used eight warmup units per target, 100 starting cards on four boards, four app CPUs, and two reversed-order eight-second rounds. Each warmup and timed phase had zero errors, equal warmup board assignments, matching sampled responses, and exact final counts. Rizzy completed 218.71 and 230.86 units/s versus Fizzy's 25.04 and 25.65. The artifact marks the checkout dirty because the benchmark warmup code was under edit; the release binary itself was built from committed application code. This validates the warmup method at four clients, not a high-concurrency or maximum-scale claim.

The clean-tree [32-client fixed-warmup run](bench/results/mysql-mixed-fixed-warmup-500-c32-2026-09-28.json) used commit `be57c9b`, 500 starting cards on four boards, 32 complete warmup units per app, four app CPUs, four Rust processes, four Fizzy workers, and two reversed-order 15-second phases. Both warmups completed without errors and added eight units per board; each Rust process handled eight. Rizzy's timed phases completed 269.89 and 279.63 units/s with zero errors and exact final counts. Fizzy's first timed phase returned one card-edit 500 from an ActiveSearch/MySQL deadlock, so it supports no paired ratio. The second phase passed every gate: Rizzy completed 279.63 versus Fizzy's 27.69 units/s, or 10.1 times as many, with p95 unit latency 149 versus 1,291 ms. This is one clean paired phase for a selected JSON workflow on one host, not proof of whole-app parity, maximum sustainable throughput, or multi-host capacity.

The card-board lock helper now returns the already locked current card fields to the edit handler, removing its second MySQL card-row query. The forced partial-edit race, title rollback, `1213` retry, paired MySQL card-edit flow, paired 142-check core flow, concurrent same-card moves, inaccessible move/write races, and 23 unit tests pass. A fresh [reversed-order two-round MySQL load](bench/results/mysql-card-snapshot-reuse-500-c16-15s-2026-09-27.json) completed 254.82 and 261.44 workflow units/s with four processes and 159.38 and 194.65 with one, with zero errors and exact counts. The large variation in one-process phases across otherwise identical runs prevents attributing a throughput gain or loss to this query reduction. This benchmark checks local process scaling, not a higher maximum capacity or a direct Fizzy comparison.

An earlier paired [two-client mixed MySQL run](bench/results/mysql-mixed-paired-500-4boards-c2-15s-2026-09-27.json) used fresh equal 500-card, four-board fixtures, four app CPUs, four Rizzy processes, four Fizzy workers, and two reversed-order 15-second rounds. Its preflight and sampled responses matched, all four phases had zero HTTP errors and exact final card, event, and comment counts. It nevertheless assigned Fizzy's two clients to different boards and Rizzy's to the same board; its 6.93–6.94× ratios are withdrawn as matched-contention comparisons. The recorded `rizzy_dirty` flag reflects the untracked result file from the immediately preceding run; the Rust source and binary were unchanged at `f921620`. At [eight clients](bench/results/mysql-mixed-paired-500-4boards-c8-15s-2026-09-27.json), Rizzy completed 205.58 and 210.87 units/s with zero errors and exact counts, while Fizzy returned one card-PATCH 500 in each round from an ActiveSearch/MySQL `1213` deadlock. Those eight-client rounds fail the paired comparison gate and support no speed ratio. The corrected two-client comparison is recorded above; neither run establishes one-for-one application parity, maximum capacity, or multi-host scaling.

The first [500-card, four-board MySQL load](bench/results/mysql-title-atomic-rust-cluster-500-c16-15s-2026-09-27.json) after that change returned a card-create 500 in each four-process round. An InnoDB deadlock trace showed creation holding the account row before inserting rich text while an edit held a rich-text insert lock before its title event requested a shared account lock. Rizzy now acquires that shared account lock at the start of a MySQL card edit. In the [reversed-order two-round recheck](bench/results/mysql-title-atomic-account-share-500-c16-15s-2026-09-27.json), one process completed 202.80 and 194.65 workflow units/s; four completed 260.27 and 265.68, gains of 28% and 37%. All four phases had zero HTTP errors and exact final card, event, and comment counts. The forced title-failure and `1213` tests and the disposable paired MySQL card-edit flow passed again. This demonstrates one-host process scaling for the tested mixed JSON workflow, not a Fizzy throughput comparison, a maximum-capacity result, or multi-host reliability.

The [post-ETag MySQL read check](bench/results/mysql-read-etag-1000-c32-4process-15s-2026-09-27.json) ran two reversed-order 15-second rounds at 32 clients with fresh equal 1,000-card fixtures, four app CPUs, four Rizzy processes, and four Fizzy workers. Four JSON read responses were compared for shape and stable content before timing. Rizzy served 4,597–4,927 requests/s versus Fizzy's 283–350, or 14.1–16.2× in the paired rounds; all phases had zero errors and unchanged card, event, and comment counts. The committed `be93544` binary included body-derived JSON ETags. This verifies throughput for those four read routes on one host, not equivalent HTML rendering, mixed writes, or multi-host scale.

The clean-tree [current MySQL read scale run](bench/results/mysql-read-scale-current-1000-c8-32-64-128-2026-09-28.json) used pinned Fizzy `477c943` and Rizzy `855859a`, fresh equal 1,000-card fixtures in each of two reversed-order rounds, four server CPUs, four Fizzy workers, four Rizzy processes, and the same MySQL 8.4.11 host. It compared canonical responses for board, card, card index, and activity JSON before timing. All 16 ten-second phases returned zero HTTP errors, each Rust process received requests, and both apps retained exactly 1,000 cards and events with no comments. Rizzy served 3,547–3,676 requests/s at 8 clients and 4,862–5,065 at 128; Fizzy served 342–349 and 346–351 respectively. At 128 clients, Rizzy's p95 was 76–84 ms versus Fizzy's 480–499 ms. On the measured 100 ms p95 threshold, Rizzy passed through 128 clients while Fizzy passed at 8 and failed at 32 and above. This is stronger concurrency evidence for these four read routes on one host; neither app's maximum capacity, equivalent browser rendering, mixed writes, or multi-host scaling is established.

The board filter bar now renders separate Board, sort, status, tag, assignee, creator, and closer controls instead of one grouped panel. The Board control stays hidden on an unfiltered board and opens the generic cards view when selected, as in the pinned source. The other quick controls retain selected values and keep the bar expanded through `expand_all=true` submissions. A fresh paired `tests/board_filter_html_parity.py` run passed filter results and collapsed/expanded HTML states. Chromium compared source and Rust control labels, selected Done, opened the Board and people popovers, and confirmed no horizontal overflow or browser errors with a popover open at 390 pixels. `tests/board_html_cached_etag.py` confirms that expanded and collapsed unfiltered board pages have distinct validators and conditional responses. The Rust controls still use native radios and checkboxes and full navigation rather than Fizzy's combobox dialogs and Turbo frames; exact keyboard, visual, and all-cards filter behavior remains open.

The [fresh 1,000-card board HTML recheck](bench/results/board-html-1000-quick-filters-2026-09-27.json) includes the added board-choice lookup and expanded-state handling. With 15 visible previews, eight shared CPUs, eight Fizzy workers, and one release Rizzy process, two-second phases measured 4,209 versus 160 requests/s at eight clients and 4,791 versus 215 at 32, without HTTP errors. Both sampled pages contained 15 cards, but Rust rendered 96,610 HTML bytes versus Fizzy's 156,477. This is a scoped same-host read comparison, not product-wide speed or a maximum-scale result.

The all-cards filter bar now also exposes Fizzy's separate Board, sort, status, tag, assignee, creator, and closer quick controls. It keeps selected controls visible when collapsed, submits choices immediately, and carries `expand_all=true` through navigation. `tests/cards_html_parity.py` passed on fresh paired databases for the existing filtered result sets and pagination, plus collapsed/expanded states and selected labels; `tests/cards_html.py` passed the media-rich preview, pagination, and form regression. Chromium compared the source and Rust control labels, selected Done and a board, and kept the filter bar expanded after both submissions. At 390 pixels the status popup stayed within the viewport without horizontal overflow or browser errors. The controls still differ from Fizzy's combobox and Turbo interactions, and the full layout remains visibly different.

The [fresh 1,000-card all-cards HTML recheck](bench/results/cards-html-1000-quick-filters-2026-09-27.json) used eight shared CPUs, eight Fizzy workers, one release Rizzy process, and 15 visible previews. Two-second phases measured 3,618 versus 323 requests/s at eight clients and 3,260 versus 455 at 32, with zero HTTP errors. Rust's sampled page was 67,865 bytes versus Fizzy's 138,208. These are faster scoped reads on one host with different HTML size and incomplete interface parity; they do not establish whole-app speed or greater multi-host scale.

Board and all-cards quick-filter popovers now include a local `Filter…` search when there is more than one eligible board, tag, or active person, following the pinned source's conditional controls. The client normalizes case and decomposed accents like Fizzy's `filterMatches`, hides unmatched choices, and clears the search when the popover closes. The disposable paired browser fixture now includes two boards and Café/Backlog tags. Chromium narrowed the source and Rust board lists from two choices to Café when typing `cafe`, and narrowed Rizzy's tag list from two choices to `#café`. Escape restored both board choices when the Rust menu reopened. Selecting the filtered tag left only its card in the all-cards result, with no browser errors or horizontal overflow at 390 pixels. The paired board and all-cards HTML tests pass with two-board search-field checks; full dialog and keyboard behavior still differs from Fizzy.

The board and all-cards quick-filter popovers now expose a labeled dialog, select the first visible choice when opened or searched, move that selection with Up/Down, and apply it with Enter while retaining focus in the search field when present. Chromium exercised a searched two-board menu and a no-search status menu: ArrowDown selected the next choice and Enter submitted the expected filtered URL in each case. The paired board and all-cards HTML suites still pass. These checks cover basic keyboard interaction, not the source dialog's full focus, Turbo navigation, or visual behavior.

A paired Chromium check against the pinned Fizzy revision found no `filter-settings-save-toggle` element after selecting Done on either the board or all-cards page. Fizzy's settings partial does not render that target initially, so its Turbo replacement response has no visible destination in these flows. Rizzy's newly added browser bookmark control therefore exceeded the reference behavior; it has been removed from both pages. The saved-filter create/delete and settings-refresh routes remain available and keep their source-style Turbo responses. This matches the observed browser state, while the complete Turbo filter interaction still differs.

The all-cards filter now fetches updated results without reloading the page. It refreshes the card grid, count, title, and selected controls, advances the URL, and restores earlier filter states on browser Back. It cancels stale requests and ignores a pagination response from an older filter state. In Chromium, text search, Clear all, Done status, and Back preserved a page marker while updating the URL and visible cards. `tests/cards_html.py` and 23 Rust unit tests pass. This is a manual frame update; full Turbo navigation parity remains open.

Board filters now also update results in place and advance browser history. The existing columns node stays mounted so its keyboard, column-menu, and drag handlers remain attached when switching between the full board and filtered grid. Watch and Add column controls are delegated to the persistent board stage, and newly rendered lazy columns are observed again. Chromium exercised search, Done, Clear all, Back, Add column, and column expansion without a page reload or losing the columns node. Fresh disposable paired `tests/board_filter_html_parity.py` and `tests/cards_html_parity.py` runs passed, along with the 23 Rust unit tests. This covers filter navigation and those retained handlers; other Turbo streams, visual states, and broad browser parity remain open.
