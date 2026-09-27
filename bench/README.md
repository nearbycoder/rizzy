# Comparative load runs

The [shared-shell named-column read](results/named-column-html-100-shared-shell-2026-09-27.json)
uses fresh 100-card fixtures, 15 visible previews, eight shared CPUs, eight
Fizzy workers, and one release Rizzy process. Two reversed-order rounds at
eight and 32 clients had zero errors. Rizzy served 4,143–4,263 and
4,916–5,031 requests/s at those levels; Fizzy served 291–408 and 379–439.
The Rust response was 59,804 bytes versus 121,093. This measures one local
read path with different markup, not whole-app speed or maximum scale.
Reproduce with `python3 bench/named_column_html_fresh.py --cards 100 --seconds 2 --concurrency 8,32 --rounds 2`.

The [shared-shell signed-in board run](results/board-html-1000-shared-shell-2026-09-27.json)
used fresh 1,000-card fixtures, 15 visible previews on each page, eight shared
CPUs, eight Fizzy workers, and one release Rizzy process. Two-second runs at
eight and 32 clients had zero errors; Rizzy served 4,397 and 4,953 requests/s
versus Fizzy's 165 and 214. Rust HTML was 90,722 bytes versus 156,477, and
Rizzy's warm board cache and markup still differ. These figures cover one local
read, not whole-app speed or maximum scale. Reproduce with
`python3 bench/board_html_fresh.py --cards 1000 --seconds 2 --concurrency 8,32`.

The [shared-shell all-cards run](results/cards-html-1000-shared-shell-2026-09-27.json)
used fresh 1,000-card fixtures and checked 15 visible previews on both
signed-in pages. Eight Fizzy workers and one Rizzy process shared eight CPUs.
At eight and 32 clients, Rizzy served 3,430 and 3,042 requests/s versus
Fizzy's 318 and 432, with zero errors in four two-second phases. The Rust
HTML was 64,885 bytes versus 138,208, and the full markup still differs.
This measures one local read path, not application-wide speed or maximum scale.
Reproduce with `python3 bench/cards_html_fresh.py --cards 1000 --seconds 2
--concurrency 8,32`.

The [populated-notification Home repeat](results/home-media-html-notification-tray-98-2026-09-26.json)
and [same-binary follow-up](results/home-media-html-notification-tray-repeat-98-2026-09-26.json)
followed the richer notification card and Home badge behavior. Both used the
same release binary hash, 100 matched events, three columns, selected matching
media counts, four assigned server CPUs, and two reversed-order five-second
rounds at eight and 32 clients. Rizzy's page was 162,819 bytes versus Fizzy's
216,774. All 16 phases had zero HTTP errors. The first run ranged from
3,166–7,498 Rizzy requests/s versus 209–414 Fizzy; the follow-up ranged from
3,067–3,147 versus 142–214. Both targets slowed during the first run and
remained slower in the follow-up, so the host's changing throughput limits an
absolute-rate comparison. Rizzy remained faster for every paired phase, but
these warm-cache local reads do not measure notification interactions, the
whole application, or maximum scale.

The [Home footer repeat](results/home-media-html-footer-98-2026-09-26.json)
followed the Pins, Search, and Notifications tray implementation. Fresh matched
samples still showed 100 events, three columns, and matching selected media
counts. Rizzy's HTML grew to 160,643 bytes versus Fizzy's 216,774 bytes. Two
reversed-order five-second rounds on four assigned server CPUs had zero HTTP
errors: Rizzy served 6,928–7,460 requests/s at eight clients and 5,626–6,746
at 32, versus Fizzy's 319–376 and 380–414. Rizzy p95 was 1.84–1.99 ms and
8.49–10.83 ms, versus 37.09–49.44 ms and 132.11–133.08 ms. The Rust Home
cache was warm; these results measure local reads of differently sized pages,
not tray interactions, whole-app speed, or maximum capacity. Reproduce with
`python3 bench/home_media_html.py --cards 98 --seconds 5 --concurrency 8,32 --rounds 2`.

The [Home media HTML run](results/home-media-html-98-2026-09-26.json)
uses fresh 98-card boards plus one image-rich card and one comment containing
a file and remote video. Before timing, both pages showed 100 events, three
columns, and matching image, background, file, and video counts. Four Fizzy
Puma workers and one Rizzy process shared four assigned server CPUs; the
Python clients used separate CPUs. Two reversed-order five-second rounds at
8 and 32 clients served 7,049–7,786 and 6,375–7,199 Rizzy requests/s,
respectively, versus 315–371 and 324–392 for Fizzy. All eight phases had zero
HTTP errors; Rizzy p95 was 1.77–1.96 ms at eight clients and 8.21–9.23 ms at
32, versus 38.03–54.56 and 145.19–172.13 ms for Fizzy. Rizzy's warm
per-user Home cache was active, and its 87,740-byte HTML page remained much
smaller than Fizzy's 216,774-byte page. The check covers event and selected
media semantics, not identical HTML or all interactions; these short local
read runs do not establish whole-app speed or maximum capacity. Reproduce
with `python3 bench/home_media_html.py`.

The [source-structure Home repeat](results/home-media-html-layout-98-2026-09-26.json)
used the same harness after Rizzy gained Fizzy's event container classes,
action icons, related-card hover attributes, nested avatar and description
row, and conditional excerpt markup. Rizzy's sampled page grew from 87,740 to
145,888 bytes; Fizzy's was 216,774 bytes. Both samples still showed the same
100 events and selected media counts. In two reversed-order five-second rounds,
Rizzy handled 7,370–7,842 requests/s at eight clients and 6,894–7,504 at 32,
versus Fizzy's 316–410 and 411, with zero HTTP errors. Its warm Home cache
remains active, and the page and application are still not one-for-one; these
numbers remain scoped read diagnostics.

The [day-grid Home repeat](results/home-media-html-day-grid-98-2026-09-26.json)
followed the shared-hour grid, centered header, local-day labels, and timezone
cookie changes. Fresh matched samples still showed 100 events, three columns,
and the same selected media counts; Rizzy's page was 148,727 bytes versus
Fizzy's 216,774. Two reversed-order five-second rounds had zero HTTP errors.
At eight clients, Rizzy served 7,116–7,679 requests/s versus Fizzy's 308–398;
at 32, 6,500–7,141 versus 399–403. The Python client, four assigned server
CPUs, four Puma workers, single Rust process, and warm Rust Home cache are the
same as the earlier run. Browser inspection confirmed the local-day cookie and
hour-row alignment; it also showed remaining card-size and footer differences.
This is still a short, read-only, one-host diagnostic, not a full parity or
maximum-capacity result.

The [post-storage-ledger scoped read run](results/embedded-ledger-read-100-4workers-8-32-5s-2026-09-26.json)
used fresh 100-card SQLite fixtures, four assigned app CPUs, four Fizzy Puma
workers, one Rust process, and a Rust load client. Two reversed-order
five-second rounds at 8 and 32 clients read board detail, card detail,
board-filtered cards, and board activities in equal shares. Sampled shapes,
card fields, titles, activity descriptions, and description HTML matched;
the first card page was 23,974 bytes in both apps. Every phase had zero HTTP
errors. Rizzy served 8,746–8,876 requests/s versus Fizzy's 371–440, with
Rizzy p95 of 1.69–5.19 ms versus 34.79–119.61 ms. This confirms that the
new attachment lookup did not erase Rizzy's advantage on these four routes;
the fixture contains no embeds, the phases are short, and it does not prove
whole-app or maximum-capacity performance.

The [matched-fixture 1,000-card MySQL JSON read run](results/mysql-read-matched-fixtures-1000-8-32-128-15s-2026-09-26.json)
uses fresh databases for two reversed-order rounds, four available app CPUs,
and a separate Rust load generator. Its four GET routes read the board, one
card, scoped cards, and scoped activities. Both targets use the same account
number and owner name and email. Before timing, the harness compares complete
sampled JSON values across all four routes after masking generated IDs,
timestamps, and server addresses; it also checks pagination. Afterward, it
checks unchanged card, event, and comment counts. At 8, 32, and 128 clients,
Rizzy served 3,251–4,117 requests/s versus Fizzy's 320–350, or 10.2–12.1×
per paired level. All twelve phases had zero errors. The serialized JSON
bodies remain about 1% smaller for Rizzy despite matching decoded values.
Full browser, side-effect, and unsampled response parity remains open. This
is one-host read throughput, not a whole-app or maximum-scale
result. Run `python3 bench/mysql_read_scale.py --mysql-port 33081` against a
local Oracle MySQL server to repeat it.

The [four-process read run](results/mysql-read-matched-fixtures-4process-1000-8-32-128-15s-2026-09-26.json) extends the same harness with balanced requests to four Rizzy processes and four Fizzy Puma workers on the same four server CPUs. Its [one-process Rizzy control](results/mysql-read-matched-fixtures-1process-1000-8-32-128-15s-2026-09-26.json) used the identical Rust binary hash. Both runs checked decoded sample JSON, all Rust replicas, unchanged 1,000-card/1,000-event counts, and zero errors in two reversed-order 15-second rounds at 8, 32, and 128 clients. Four-process Rizzy served 3,372–4,605 requests/s versus Fizzy's 290–312; its mean throughput gain over one Rizzy process was about 2% at 8 clients, 24% at 32, and 30% at 128. The extra processes increased memory, CPU use, and 128-client p95 latency. The result demonstrates local process throughput scaling for four JSON reads, not multi-host capacity or product-wide speed. Reproduce with `python3 bench/mysql_read_scale.py --mysql-port 33081 --rizzy-processes 4` and rerun with `--rizzy-processes 1` for the control.

The [CSP-enabled board HTML run](results/board-html-csp-2026-09-26.json)
rechecks a fresh 100-card board fixture after Rizzy began adding per-response
script nonces. Two reversed-order five-second rounds at 8 and 32 clients used
the same four available server CPUs. Rizzy handled 5,219–5,843 requests/s and
Fizzy 77–89, with zero HTTP errors and 15 visible cards in both samples. The
Rust page is about 50.5 KB versus 158 KB for Fizzy and lacks complete UI
parity, so this is a scoped diagnostic, not a product-wide speed or scale
claim. Run `python3 bench/board_html_csp.py` to repeat it with disposable
databases.

The [sustained mixed workload](results/sustained-mixed-100-2026-09-26.json) runs `python3 bench/sustained_mixed.py --seconds 60 --clients 32 --cards 100 --rounds 2 --server-cpus 8` against the pinned Fizzy revision and a release Rizzy build. The result records both source revisions and the release binary SHA-256. Each of two reversed-order rounds uses fresh disposable databases, eight CPU-affined production Puma workers for Fizzy and one CPU-affined Rizzy process, with the Python clients on other CPUs. Both accounts start with the same 100-card board. A unit reads board, card, filtered cards, and filtered activities JSON, then creates, edits, comments on, closes, and reads a new card: five reads and four writes. Before timing, the harness compares sampled response shapes, content, counts, and pagination headers; afterward, it checks card, event, and comment totals in each database.

With account update time included in board JSON ETags, a repeat at 32 clients measured 91.10 and 90.99 Rizzy units/s versus Fizzy's 28.03 and 25.61, a 3.25× and 3.55× ratio by round. The p95 unit times were 689.16 and 698.29 ms for Rizzy versus 1,538.74 and 1,783.69 ms for Fizzy. All four runs had zero HTTP errors and the expected final records. Fizzy's eight-process tree grew from about 1.0 GiB to 3.0–3.1 GiB proportional memory; Rizzy grew from about 24 MiB to 100–102 MiB. Rizzy used about 7.0 of its eight assigned CPU cores on average versus Fizzy's 4.6. The different process and feature models, incomplete HTML and side-effect parity, one-host generator, SQLite's single-writer limit, and one tested concurrency level prevent a whole-app performance or maximum-scale claim. The shorter five-second pilot was only a harness check. `tests/board_show_pagination_parity.py` separately compares the 20-card signed-in board HTML and filtered JSON pages and conditional responses on fresh databases.

The same harness then used 1,000 initial cards, 30 seconds per target, eight assigned server CPUs, and two reversed-order rounds at each concurrency level. The [64-client](results/sustained-mixed-1000-64-2026-09-26.json), [128-client](results/sustained-mixed-1000-128-2026-09-26.json), and [256-client](results/sustained-mixed-1000-256-2026-09-26.json) artifacts pin the Fizzy and Rizzy revisions and Rust binary hash. All twelve measured runs had zero HTTP errors and matching card, event, and comment count formulas after the run.

| Clients | Fizzy units/s, two rounds | Rizzy units/s, two rounds | Fizzy p95 unit time | Rizzy p95 unit time |
| ---: | ---: | ---: | ---: | ---: |
| 64 | 25.63–25.67 | 118.51–121.22 | 3.17–3.45 s | 1.01–1.06 s |
| 128 | 22.83–27.08 | 124.16–125.63 | 5.35–6.53 s | 1.90–1.92 s |
| 256 | 24.55–26.65 | 124.96–126.62 | 10.43–12.59 s | 3.68–3.71 s |

Rizzy sustained roughly 4.6–5.4 times the completed workflow rate on this fixture. Throughput changed little from 128 to 256 clients while latency rose, so this is near the local throughput plateau for both setups; neither reached an error-defined maximum. At 256 clients Fizzy's process tree ended at 3.0–3.1 GiB proportional memory and Rizzy at 98–99 MiB. Both servers had the same CPU affinity, but Fizzy used about 4.2 cores on average and Rizzy 6.4–6.6; this is equal CPU *availability*, not equal CPU consumption. The apps still differ in process model, HTML, and background side effects. This measures one API workflow on one host with SQLite, not whole-app parity, multi-host scaling, or production capacity.

After expanding Action Text sanitization and plain-text conversion, the [64-client repeat](results/sustained-mixed-1000-64-rich-text-2026-09-26.json) pinned the new Rust release binary and used the same 1,000-card fixture, CPU allocation, 30-second windows, and reversed order. Rizzy completed 124.27–124.59 units/s with 961–973 ms p95; Fizzy completed 19.79–23.62 units/s with 4.07–5.06 s p95. All four runs had zero HTTP errors and verified card, event, and comment totals. Fizzy's rate varied from the earlier 64-client artifact, so the repeat establishes that the current Rust build retains a lead on this workload; it does not isolate the rich-text change's performance effect.

The [30,000-board run before count optimization](results/board-index-json-30k-8cpus-2026-09-26.json) exposed a Rust regression: 27–35 Rizzy versus 107–110 Fizzy requests/s on the fourth 100-board JSON page. Rizzy's page lookup already used the recent-access index, but its total-count query joined all 30,000 accesses to boards on every request. A direct database probe measured that count at 15.15 ms median and the indexed page lookup at 0.12 ms median. Rizzy now counts its account-scoped access records through a covering `(user_id, account_id)` index, and startup repairs orphaned or wrongly scoped records. In the [repeat with the count index](results/board-index-json-30k-8cpus-count-index-2026-09-26.json), two reversed-order eight-second rounds measured 1,442–1,448 Rizzy versus 108–110 Fizzy requests/s at 64 clients, with p95 51 ms versus 674–746 ms and no HTTP errors. At eight clients the ranges were 1,554–1,570 versus 85–102 requests/s. Each server was restricted to the same eight logical CPUs; the load generator ran on other CPUs. The five-page order, counts, links, and sampled JSON shapes matched, while source and Rust accounts contained four and zero additional boards, respectively. The 100-board responses were 55.8 and 55.3 KB. This establishes a stronger local read-capacity result for one route and dataset; eight-second runs, direct-row fixtures, remaining HTML differences, writes, and broader product behavior still need verification.

The [30,000-board SQL query probe](results/board-index-query-30k-2026-09-26.json) measures forty warmed reads of a 100-board page at offset 55 in an in-memory SQLite fixture. The median fell from 7.6 ms to 0.04 ms after adding `accesses(user_id, accessed_at DESC)`; `EXPLAIN QUERY PLAN` stopped reporting a temporary sort. Run `python3 bench/boards_index_query.py` to repeat it. This isolates the page lookup and does not measure JSON rendering, HTTP throughput, or production capacity.

The [200-board JSON index run](results/board-index-json-200-2026-09-26.json) follows `tests/boards_index_order_parity.py`: direct-row fixtures put 200 distinct recent-access boards at the front of each account, and five pages plus a 100-board response shape are checked before timing page four. With the release Rust server and eight production Fizzy Puma workers, two ten-second reversed-order rounds measured 890–896 versus 562–599 requests/s at eight clients and 707–711 versus 621–622 at 64, with no HTTP errors. Rust's batched creator loading removes per-board user queries from this route. At 64 clients the Rust server averaged about 10 CPU cores versus Fizzy's eight, so the result is a route-specific local read comparison rather than equal-budget scale proof. The pages were 55.1 and 55.6 KB, and the two accounts had different baseline board counts.

The [anonymous public-board benchmark](public_board_html.py) creates matching published boards with Maybe, Not Now, named, and Done cards, checks the side-column counts and the same 15 Maybe titles, then times the board HTML and Maybe-column HTML separately. On the [1,000-card fixture](results/public-board-html-1000-2026-09-25.json), at 64 clients Rizzy served 7,550 versus Fizzy's 1,626 board requests/s (p95 15.9 versus 69.5 ms) and 2,301 versus 812 Maybe-column requests/s (p95 49.6 versus 161.1 ms). The [100-card fixture](results/public-board-html-100-2026-09-25.json) also completed without HTTP errors. These are two-second sequential local reads with a fresh release Rizzy account and Fizzy's existing account and Ruby worker pool. They exclude asset loads, client-side column loading, writes, CPU and memory usage. The 1,000-card board response was 11.1 KB for Rizzy versus 41.6 KB for Fizzy; the Maybe-column response was 7.9 versus 88.2 KB. The content and presentation are still different, so the measured ratios are diagnostics for these read paths, not proof of one-for-one speed or maximum sustainable scale.

After public preview articles gained tags, assignee metadata, stage controls, stamps, and image markup, the [100-card rerun](results/public-board-rich-html-100-2026-09-25.json) measured 3,393 Rizzy versus 924 Fizzy Maybe-column requests/s at 32 clients, with p95 16.0 versus 59.1 ms and no HTTP errors. Rizzy's stream response rose from 7.9 to 28.9 KB; Fizzy's was 88.2 KB. This was a one-second sequential sample and the HTML is still different, so it remains a route-level diagnostic rather than a capacity or full-parity result.

The [Not Now column run](results/public-not-now-html-100-2026-09-25.json) adds the first public page of postponed cards, checks the same 10 titles and dated stamp markup, and measures the new persisted record path. At 32 clients Rizzy served 2,663 versus Fizzy's 1,872 requests/s, with p95 13.1 versus 28.1 ms and zero HTTP errors. The responses were 21.9 versus 74.4 KB, and each target ran for one second at a time. These differences limit this to a local read diagnostic; CPU, memory, writes, sustained load, and full HTML equivalence remain unmeasured.

The [browser search script](search_html.py) creates matching card/comment hits in each app, verifies the title multiset and result counts over four pages, then measures the first and fourth HTML pages with signed-in cookies. The [current 100-card, 20-comment run](results/search-html-100-page-first-2026-09-25.json) returned 15 and 25 hits on those pages; at 32 clients it measured 3,623 versus 888 requests/s on page one and 2,952 versus 470 on page four (Rizzy versus Fizzy). A [1,000-card, 200-comment baseline](results/search-html-1000-2026-09-25.json) exposed a Rust query slowdown at 32 clients. After ranking only hit IDs before loading full row content, the [1,000-card rerun](results/search-html-1000-page-first-2026-09-25.json) measured 918 versus 737 requests/s on page one and 510 versus 443 on page four at 32 clients; its p95 was 44.5 versus 71.2 ms and 79.7 versus 111.9 ms. The intermediate [forced hit-first run](results/search-html-1000-hit-first-2026-09-25.json) was slower on the first page and was replaced. These runs used separate two- or three-second sequential samples on one host, a release Rust process with a fresh account, and Fizzy's existing account and Ruby worker pool. Fizzy's throughput varied between repetitions. Rizzy's page-one HTML was about 8.7 KB versus Fizzy's 56.4 KB, and its page-four HTML was about 43.4 KB versus Fizzy's 120.6 KB in the 1,000-card fixture. The result counts and sampled content match, but the pages and server work are not yet one-for-one, so these timings do not prove a product-wide speed or scale advantage.

After switching SQLite browser snippets to FTS5's 20-token `snippet` and `highlight`, the [1,000-card, 200-comment run](results/search-html-1000-sqlite-snippet-2026-09-27.json) again passed the four-page result count and title-set gate. At eight and 32 clients, Rizzy served 488 and 457 first-page requests/s versus Fizzy's 350 and 390; on page four it served 310 and 298 versus 222 and 233. The two-second phases had no HTTP errors. The first-page bodies remained about 9 KB for Rizzy and 56 KB for Fizzy, and the later number-lookup change does not affect the benchmark's nonnumeric query. These figures describe this route and host, not maximum scale or full application parity.

The [microsecond creation-time recheck](results/search-html-1000-microtime-2026-09-27.json) records the pinned Fizzy revision, release Rust binary hash, eight shared CPU IDs, eight Fizzy workers, and one Rust process. Its fresh 1,000-card, 200-comment fixture passed the four-page count and title-set gate. At eight and 32 clients, Rizzy served 495 and 460 first-page requests/s versus Fizzy's 360 and 384; on page four it served 314 and 301 versus 227 and 240, without errors. The client ran on the same CPUs as the servers and timed each target sequentially for two seconds. Rizzy's first-page HTML was about 9 KB versus Fizzy's 56 KB, so the figures are a route-level regression check, not an equal-work or maximum-scale result.

The [search form and pagination recheck](results/search-html-1000-source-form-2026-09-27.json) follows richer Rust form, result, timestamp, and automatic-pagination markup. It records the same setup and a new release binary hash. Fresh four-page result counts and title sets matched, and the two-second phases had zero errors. At eight and 32 clients, Rizzy served 462 and 442 first-page requests/s versus Fizzy's 341 and 340; on page four, 304 and 292 versus 224 and 236. Rizzy's first-page body increased to 17.9 KB but remained smaller than Fizzy's 56.3 KB. This is a scoped read comparison, not whole-app speed or maximum-scale proof.

`search_html.py --fresh` now starts both apps with disposable SQLite databases, pins the same CPU set for both servers and the client, and records the source revision and release Rust binary hash. `--rounds 2` reverses app order in the second round. After adding comment creator avatars, the [1,000-card, 200-comment two-round run](results/search-html-1000-avatar-fresh-2round-2026-09-27.json) matched the result count and title set on four pages and had zero HTTP errors in every two-second phase. At 32 clients, first-page Rizzy throughput was 468 requests/s in both rounds versus Fizzy's 353–393; fourth-page throughput was 272–293 versus 223–241. First-page p95 was 80–85 ms versus 131–148; fourth-page p95 was 147–153 versus 216–225. HTML body sizes were 18.6 versus 56.3 KB on page one and 77.5 versus 120.0 KB on page four. Run `cargo build --release` then `python3 bench/search_html.py --fresh --cards 1000 --seconds 2 --concurrency 8,32 --rounds 2` to repeat. This remains a short one-host read test with different presentation work, not a whole-app or maximum-scale result.

The [scoped JSON concurrency run](results/scoped-api-concurrency-2026-09-25.json) created 100 cards per app and checked matching board, card, and activity JSON shapes, card description HTML, first-page counts, and filtered-card totals before timing. It rotated through that board, one card, its filtered card list, and its filtered activity list at 1, 8, 32, 64, 128, and 256 clients. At 64 clients Rizzy served 2,398 requests/s with 48.7 ms p95 versus Fizzy's 733 requests/s with 191.6 ms p95; neither had HTTP errors at any measured level. With an exploratory 100 ms p95 cutoff, Rizzy's last passing level was 64 clients and Fizzy's was 32. The filtered first-page JSON sizes were 23.8 and 24.7 KB, respectively. These are single five-second sequential runs on one host with a Python client. Fizzy used an existing account and 32 Ruby workers while Rizzy used a fresh account and one release process; background account data and deployment shape differ, and the run contains no writes or full browser interactions. It shows this fixture's local read capacity, not maximum production scale or whole-app parity. The [account-wide mixed diagnostic](results/mixed-api-concurrency-2026-09-25.json) also included a board list with different background contents, so it should not be used for an equal-workload scale comparison. The harness now removes new fixture boards when it exits.

The [alternating scoped JSON repeat](results/scoped-api-alternating-2026-09-25.json) used fresh isolated accounts and the same 100-card fixture, with two ten-second rounds at 8, 32, and 64 clients. The harness reversed target order in round two and confirmed matching sampled board, card, and activity JSON shapes, description HTML, first-page counts, and totals. At 64 clients, Rizzy served 3,974 and 3,993 requests/s with 27.1 and 27.3 ms p95; Fizzy served 1,193 and 1,175 requests/s with 116.1 and 121.7 ms p95. All runs had zero HTTP errors. Fizzy ran 16 Puma workers and Rizzy used its default 12 SQLite connections on the same host. The filtered first-page JSON was 23.7 KB in Rizzy and 24.8 KB in Fizzy, so the response work is close in size but not identical. The Python load client, sequential measurements, ten-second windows, and read-only scope still prevent a claim of maximum sustainable throughput, production scaling, or full application parity. Use `--rounds 2` to repeat this ordering check.

Both scoped Python artifacts above predate the global request counter. Their per-worker endpoint assignment allowed different effective endpoint mixes, so use the balanced Rust-client results below for the four-path capacity comparison.

The [board HTML script](board_html.py) creates a 100-card board on each app, checks 15 visible Maybe cards and the add-card control, then reads the signed-in board page through browser cookies. The [two-second local run](results/board-html-2026-09-25.json) measured 2,142 Rizzy versus 366 Fizzy requests/s at 32 clients (p95 16.7 versus 161.2 ms) and 2,012 versus 311 at 128 (p95 68.0 versus 535.8 ms). Neither target returned HTTP errors. The Rust page was 46.7 KB versus Fizzy's 189.2 KB; its layout and behavior are still incomplete, so this is a diagnostic for the present first page, not evidence of one-for-one product speed or maximum sustainable scale. The client ran targets sequentially on one host and did not measure asset loads, CPU, memory, or interactions.

The [card HTML script](card_html.py) creates 100 cards on each app, adds the same two steps and one comment to the sampled card, verifies their rendered text, and reads the signed-in card page through browser cookies. The [two-second local run](results/card-html-2026-09-25.json) measured 6,959 Rizzy versus 724 Fizzy requests/s at 32 clients (p95 8.4 versus 87.0 ms) and 7,241 versus 659 at 128 (p95 29.4 versus 318.8 ms), with no HTTP errors. Fizzy's response was 70.3 KB and Rizzy's 22.5 KB; card controls, markup, and side effects still differ. This run measures neither front-end assets nor interaction latency, and the sequential short samples do not prove full product speed or maximum sustainable scale.

`card_html_fresh.py` starts disposable Fizzy and Rizzy servers and records their source revision, Rust binary hash, shared CPUs, and worker count. It uses the card HTML script's two-step, comment, tag, and reaction content gate. The [1,000-card interactive-shell run](results/card-html-1000-interactive-shell-2026-09-27.json) used eight shared CPUs, eight Fizzy workers, one Rust process, and two-second phases. At 32 clients, Rizzy served 5,979 requests/s versus Fizzy's 444; the HTML bodies were 98,680 and 73,878 bytes. These are sequential warm reads of one card with different markup, so they do not establish whole-app speed or maximum scale. Reproduce with `python3 bench/card_html_fresh.py --cards 1000 --seconds 2 --concurrency 8,32 --output bench/results/card-html-fresh.json` after building release Rizzy.

After adding the image control and its server lookup, the [repeat](results/card-html-with-image-control-2026-09-25.json) measured 7,331 versus 722 requests/s at 32 clients and 7,732 versus 685 at 128, with no HTTP errors. The small change from the prior run is within what these short local samples can show; it should not be attributed to the image control. The two HTML responses still differ substantially in size and behavior.

After adding the card tag picker, the script also placed the same tag on each sampled card and checked its text. The [tagged card run](results/card-html-with-tag-2026-09-25.json) measured 6,650 Rizzy versus 736 Fizzy requests/s at 32 clients (p95 8.4 versus 94.2 ms) and 7,253 versus 718 at 128 (p95 29.6 versus 269.0 ms). Fizzy's HTML was 70.6 KB and Rizzy's 27.3 KB. The samples lasted two seconds per target, and the apps still differ in markup and interactions; comparing this run with the earlier untagged samples does not isolate the cost of one added lookup.

The [reaction card run](results/card-html-with-reactions-2026-09-25.json) published the sampled card and added the same two steps, one comment, one tag, one card reaction, and one comment reaction to each app. It checked the saved reactions and all six rendered content markers before timing. At 32 clients, Rizzy served 5,362 requests/s versus Fizzy's 690 (p95 10.0 versus 93.5 ms); at 64, it served 6,131 versus 699 (p95 17.9 versus 175.8 ms). Both had zero HTTP errors. The HTML responses were 44.2 KB and 74.1 KB, and Rizzy's card UI and side effects remain incomplete. These five-second sequential local reads do not establish one-for-one page performance or sustainable product scale. The added publication and reactions also make this workload different from the earlier tagged-card run, so their throughput difference cannot be attributed to reactions alone.

The [notification HTML script](notifications_html.py) joins a member to each account, gives that member access to a new board, creates 100 cards and verifies 100 unread notifications and 100 rendered items before timing. Its [first run](results/notifications-html-100-2026-09-25.json) exposed Rizzy's per-item database reads: at 32 clients Fizzy served 806 requests/s and Rizzy 148, with p95 76.5 and 234.1 ms. After batching card, creator, column, event, mention, and comment lookups, a [repeat](results/notifications-html-100-batched-2026-09-25.json) reached 196 Rizzy requests/s at 32 clients versus Fizzy's 807. The [warm page-cache run](results/notifications-html-100-cached-2026-09-25.json) reached 6,856 versus 819 requests/s at 32 clients (p95 8.3 versus 74.1 ms) and 5,812 versus 689 at 64 (p95 21.1 versus 179.6 ms), with zero HTTP errors. Rizzy checks notification count, latest update time, and unread count before reusing a page; a two-process test verifies read/unread changes invalidate it. A later one-second expiry bounds stale details after unrelated edits and was not part of this run. The HTML responses were about 102.5 KB for Rizzy and 251.8 KB for Fizzy; the Rust presentation remains simpler. These sequential five-second samples favor repeated warm reads and do not measure cold-cache throughput, write-heavy notification traffic, CPU, memory, or maximum sustainable scale. The invite-based fixture consumes one join-code use per run, even though it removes the temporary member afterward.

With the one-second expiry, a [three-second rerun](results/notifications-html-100-expiring-cache-2026-09-25.json) measured 6,334 Rizzy versus 883 Fizzy requests/s at 32 clients (p95 8.1 versus 69.1 ms) and 5,837 versus 807 at 64 (p95 15.5 versus 134.7 ms). Concurrent refreshes caused a 153.7 ms Rust p99 at 64 clients. A per-user single-flight rebuild then gave [6,465 versus 778 requests/s at 64 clients](results/notifications-html-100-singleflight-2026-09-25.json), with p95 18.4 versus 145.5 ms and p99 26.1 versus 175.7 ms. Both targets had zero HTTP errors. These are separate short runs with different fixture users and different background account contents; the change between runs includes normal measurement variation. The current result is a warm-read measurement of a smaller Rust page, not proof of overall notification throughput or greater maximum scale.

The [all-cards browser grid script](cards_html.py) creates a 100-card board on each app, checks that the filtered first HTML page contains 15 previews, then measures browser-session reads. With `--rich`, it also adds a column, tag, completed step, user comment, boost, and assignee to each visible card and checks the tag and step progress in both responses. The [plain result](results/cards-html-2026-09-25.json) measured 2,116 Rizzy versus 652 Fizzy requests/s at 32 clients (p95 17.2 versus 92.6 ms); the [rich result](results/cards-html-rich-2026-09-25.json) measured 995 versus 642 requests/s (p95 36.0 versus 90.2 ms). After indexing the step lookup, the [rich high-concurrency run](results/cards-html-rich-high-concurrency-2026-09-25.json) measured 1,028 versus 666 requests/s at 64 clients and 1,026 versus 636 at 128; Rizzy's p95 was 69.4 versus 184.7 ms at 64 and 133.2 versus 285.3 ms at 128. Each server ran sequentially on the same host for two seconds per level, without errors. Fizzy's HTML was 169–194 KB and Rizzy's 29–35 KB because the Rust interface still omits presentation details, so these numbers show performance of the current pages, not equivalent product capacity or a proven scale advantage. The richer fixture also shows that preview metadata reduces Rizzy's 32-client throughput substantially; continue profiling before adding more detail.

After the grid was restyled against compiled Fizzy CSS, the [latest rich-grid run](results/cards-html-source-layout-2026-09-25.json) measured 1,037 Rizzy versus 658 Fizzy requests/s at 32 clients, 1,035 versus 614 at 64, and 1,033 versus 630 at 128. The 128-client p95 was 131.6 ms for Rizzy and 302.2 ms for Fizzy; neither server reported errors. The Rust page was 44 KB versus Fizzy's 194 KB, so this still does not establish full HTML parity or application-wide scale. The load client requests HTML only and does not time CSS, images, JavaScript, or interactions.

`compare.py` creates the same number of cards in a fresh board on each server. It checks board, card, and activity JSON shapes, description HTML, first page sizes, and total counts before measuring a read workload: board list, board detail, card detail, filtered card list, and board activity feed. Its current global request counter sends nearly equal request counts to every selected endpoint. Requests are authenticated with the same permission level. The script reports throughput, latency percentiles, and errors for each concurrency level. Runs before the activity mix was added used four endpoints. Earlier Python runs assigned each worker to one endpoint, so faster endpoints could contribute more requests and alter the effective workload mix. Their ratios are diagnostics and are superseded by the balanced runs below.

The [initial result](results/initial-2026-09-24.json) used the Fizzy source at `477c943e0506f109e5bc83ae9dadbe519732c045`, Ruby 3.4.8, Rails production, Puma's default 16 workers and 1 thread each, SQLite, and a Rust release build with 12 SQLite connections. Both ran on one 32 logical CPU AMD Ryzen AI Max+ 395 machine. The fixture had 100 cards per board and each first card page had 15 cards. The four sampled response shapes and description HTML matched; serialized first page size was approximately 24 KB for each.

The Rust build processed 1,467, 4,714, and 4,707 requests per second at concurrency 1, 8, and 32 respectively, versus 122, 1,244, and 1,653 for Fizzy. At concurrency 32, p95 was 12.4 ms for Rizzy and 43.1 ms for Fizzy. These are short five second diagnostic runs on a partial implementation. They do not establish a one-for-one speed or scale advantage. The apps still differ in many features, queries, side effects, caching, and deployment behavior. The results need repeated runs with equal resource budgets and full response and behavior parity before making a product claim.

After adding assignees to card responses and more API routes, the [current read run](results/current-2026-09-24.json) measured 4,306 requests per second for Rizzy versus 1,580 for Fizzy at concurrency 32, with p95 16.5 ms versus 45.6 ms. The four sampled response shapes still matched. This is also a five second diagnostic on a partial implementation.

The [higher-concurrency run](results/high-concurrency-2026-09-24.json) measured 4,327 versus 1,549 requests per second at 64 clients, with p95 38.4 versus 86.1 ms. At 128 clients, the figures were 4,280 versus 1,439 requests per second, with p95 77.9 versus 156.2 ms. For this specific read workload and a 50 ms p95 target, Rizzy met the target at 64 clients while Fizzy's last measured passing point was 32 clients. The Python generator may cap throughput, and these numbers cannot be extrapolated to overall application capacity.

After adding stored activity events and board/card timestamp updates, the [current read run](results/with-events-and-touches-2026-09-24.json) measured 4,378 versus 1,580 requests per second at 32 clients (p95 18.5 versus 46.0 ms) and 4,362 versus 1,386 at 64 clients (p95 39.4 versus 91.8 ms). The [current 32-client write run](results/write-with-touches-2026-09-24.json) measured 303 versus 102 card creates per second, both without HTTP errors or duplicate card numbers. Fizzy still performs search indexing, webhook dispatch, notifications, and jobs that Rizzy does not, so the write result is not a fully equivalent operation.

Adding the activity feed to one fifth of read requests exposed repeated database lookups in Rizzy. In the [first five-endpoint run](results/activity-mix-2026-09-24.json), Rizzy reached 1,413 requests/s with 70.1 ms p95 at 32 clients, versus Fizzy's 1,148 requests/s with 63.9 ms p95. After batch loading activity relationships, the [repeat](results/activity-mix-batched-2026-09-24.json) reached 2,120 requests/s with 43.2 ms p95 for Rizzy, versus 1,175 requests/s with 59.8 ms p95 for Fizzy. At 64 clients, Rizzy measured 1,752 requests/s and 110.1 ms p95 versus Fizzy's 1,110 requests/s and 115.6 ms p95. These are five second local runs on partial implementations; activity payload shape and fixture event counts were checked before timing.

After moving board access to explicit membership records, the [five-endpoint rerun](results/board-access-2026-09-25.json) measured 2,344 requests/s at 32 clients for Rizzy versus 1,087 for Fizzy, with p95 40.0 ms versus 69.5 ms. At 64 clients the run measured 2,641 versus 860 requests/s, with p95 81.4 versus 158.9 ms. Both served 100-card fixtures with matching board, card, and activity response shapes, five equal read endpoints, and no reported HTTP errors. These single five-second runs remain sensitive to caching, load-generator limits, and sequential target order. The applications still differ in many features and side effects.

After adding webhook management and a delivery worker, the [five-endpoint read rerun](results/webhooks-read-2026-09-25.json) measured 2,112 versus 1,098 requests/s at 32 clients, with p95 45.5 versus 65.2 ms. At 64 clients it measured 2,532 versus 1,073 requests/s, with p95 83.1 versus 122.3 ms. The workload had no subscribed webhooks, so this run does not measure delivery throughput or equal write side effects. It checked board, card, and activity shapes and reported no HTTP errors.

The [FTS5 search run](results/fts-search-2026-09-25.json) created 100 cards per target with the same unique search term. Before batching card response reads, Rizzy served 954 search requests/s versus 691 for Fizzy at 32 clients; p95 was 37.7 versus 95.2 ms. The [batching rerun](results/fts-search-batched-2026-09-25.json) measured 2,052 versus 722 requests/s at 32 clients, p95 17.7 versus 89.8 ms. At 64 clients it measured 2,056 versus 689 requests/s, p95 33.8 versus 163.1 ms. The script checked 100 hits, a 15-card first page, sampled JSON shape, and HTTP errors before timing. These are separate five-second diagnostic runs with new fixtures, so the change between the two Rizzy measurements also includes run-to-run noise.

After indexing visible plain text, the [500-card formatted-search run](results/search-formatted-500-2026-09-25.json) placed the same unique term inside a bold paragraph on every card and verified that a hidden HTML class name returned no results. It checked first-page JSON shape, plain description, rendered HTML, total count, and HTTP errors. At 32 clients Rizzy served 1,678 requests/s versus Fizzy's 601 (p95 23.3 versus 110.3 ms); at 64 clients it served 1,698 versus 603 (p95 42.4 versus 195.7 ms). Both had zero HTTP errors through 128 clients. With a 50 ms p95 target, Rizzy's last measured passing point was 64 clients versus Fizzy's 16, four times the concurrent clients for this specific search read. Rizzy exceeded the target at 128 clients; Fizzy exceeded it at 32. The serialized first pages were 25.7 KB for Rizzy and 26.5 KB for Fizzy. These are sequential five-second runs with a Python client on already populated accounts; client saturation and unequal background data remain possible, and the result does not establish full product scale.

The [filter read run](results/filters-2026-09-25.json) used 100 cards per target, closed 20, and alternated a term filter with a today-and-closer filter. Both returned the expected counts and matching sampled JSON shapes. At 32 clients, Rizzy served 2,373 requests/s with 15.4 ms p95 versus Fizzy's 670 with 95.5 ms p95. At 64 clients, Rizzy served 2,445 requests/s with 28.8 ms p95 versus Fizzy's 636 with 174.6 ms p95. Both reported zero HTTP errors. The first pages were about 24.5 KB. This is a single five-second local run per concurrency level, with sequential targets and a Python load generator. It demonstrates only these filter paths on this fixture; full feature parity and larger controlled scale runs remain required.

After matching Fizzy's bearer-token format and read/write authorization, the [repeat filter run](results/auth-format-2026-09-25.json) used the same 100-card fixture design and checked first-page shape and counts. At 32 clients Rizzy served 2,282 requests/s with 16.1 ms p95 versus Fizzy's 661 with 97.7 ms p95. At 64 clients Rizzy served 2,301 requests/s with 30.4 ms p95 versus Fizzy's 624 with 189.9 ms p95. Both had zero HTTP errors. These are five-second local measurements of two routes, not proof of product-wide speed or maximum sustainable scale.

After adding `.json` route suffix handling and column position actions, the [five-endpoint read run](results/json-routing-five-endpoint-2026-09-25.json) measured 3,349 requests/s at 32 clients for Rizzy versus 1,047 for Fizzy, with p95 16.1 versus 70.1 ms. At 64 clients it measured 3,472 versus 1,106 requests/s, with p95 27.7 versus 124.3 ms. The 100-card fixture matched sampled board, card, and activity JSON shapes, description HTML, and first-page count; neither server returned HTTP errors. These are single five-second sequential measurements on partial implementations, so they do not establish maximum capacity or product-wide advantage.

After adding browser draft creation and publication, the [five-endpoint read rerun](results/drafts-read-2026-09-25.json) measured 3,475 versus 1,172 requests/s at 32 clients, with p95 15.9 versus 59.5 ms. At 64 clients it measured 3,365 versus 1,102 requests/s, with p95 28.7 versus 117.2 ms. The sampled JSON shapes, description HTML, and page counts matched, and both targets reported zero HTTP errors. This is still a short sequential run on partial implementations.

After adding card image storage and image URLs to card responses, the [read rerun](results/card-images-read-2026-09-25.json) measured 4,009 versus 1,163 requests/s at 32 clients, with p95 15.2 versus 61.6 ms. At 64 clients it measured 3,911 versus 1,070 requests/s, with p95 27.9 versus 124.3 ms. The 100-card fixtures contained no images, so this measures the added lookup cost on the five existing read endpoints, not image serving throughput. Sampled JSON shapes, description HTML, and page counts matched; both had zero HTTP errors. This is a single five-second sequential run on partial implementations, and the difference from the previous run includes normal measurement variation.

The [avatar thumbnail run](results/avatar-thumbnail-2026-09-25.json) uploaded the same 806×926 PNG to each server, warmed its 256×256 thumbnail, then timed unauthenticated GETs through both redirects and the final file response. At 32 clients Rizzy served 1,785 requests/s versus Fizzy's 996 (p95 25.6 versus 45.1 ms). At 64 clients it served 1,791 versus 976 (p95 51.6 versus 90.4 ms). Both reported zero HTTP errors. Rizzy's thumbnail was 38,294 bytes and Fizzy's was 70,367, so transferred bytes and encoder work were not equal. These five-second sequential samples do not establish a like-for-like image-processing or maximum-scale advantage.

After adding common Action Text rendering, the [five-endpoint plain-text rerun](results/rich-text-read-2026-09-25.json) measured 3,825 versus 1,168 requests/s at 32 clients (p95 15.5 versus 60.0 ms) and 3,760 versus 1,050 at 64 clients (p95 28.6 versus 126.6 ms). The sample response shapes, description HTML, and page counts matched, with no HTTP errors. The fixture descriptions were plain text, so this mainly checks the overhead of the new rendering path.

The [formatted-description run](results/rich-text-formatted-2026-09-25.json) used 100 cards per server with the same paragraph, bold text, link, and list. It verified first-page plain text and HTML equality before timing card detail and filtered card-list reads. At 32 clients Rizzy served 3,315 requests/s versus Fizzy's 1,281 (p95 16.2 versus 59.8 ms); at 64 clients it served 3,367 versus 1,045 (p95 30.0 versus 123.9 ms). First-page payloads were 25.6 KB and 26.2 KB respectively. These are sequential five-second diagnostics for two routes, and do not cover rich-text attachments or full product scale.

After adding passkeys, push subscription storage, and timed notification emails, the [five-endpoint read rerun](results/compare-notification-2026-09-25.json) used release Rizzy and production Fizzy with 100 cards per board. Board, card, and activity JSON shapes, description HTML, first-page counts, and total counts matched; serialized card-page samples were 24,375 and 24,645 bytes. At 1, 8, and 32 clients, Rizzy served 1,450, 3,211, and 3,648 requests/s versus Fizzy's 94, 718, and 1,071. At 32 clients p95 was 15.1 ms versus 66.2 ms, with no reported HTTP errors. The read fixture did not trigger notification emails; target runs were sequential and only one five-second sample was taken at each level. Different response bytes, background work, and incomplete UI parity still prevent an overall speed or scale claim.

The [64- and 128-client continuation](results/compare-notification-high-2026-09-25.json) measured 3,596 and 3,785 Rizzy requests/s versus 1,145 and 1,079 for Fizzy. At 64 clients p95 was 28.7 ms for Rizzy and 110.3 ms for Fizzy; at 128 it was 55.4 and 223.4 ms. Neither server reported HTTP errors. With a 50 ms p95 target, Rizzy's last measured passing level was 64 clients and Fizzy's was 8 for these samples; untested concurrency levels may also pass. This remains a Python-generated, short, sequential read workload, not a maximum sustainable load measurement.

The [image-embed read run](results/embeds-2026-09-25.json) created 20 cards per server, each with a distinct uploaded moon.jpg embed. It checked sampled card JSON shape, plain text, normalized HTML, attachment flags, and first-page count before alternating card detail and filtered card-list reads. At 32 clients Rizzy served 3,471 requests/s versus Fizzy's 1,323 (p95 17.2 versus 58.8 ms); at 64 clients it served 3,265 versus 1,089 (p95 33.9 versus 122.7 ms). Both had zero HTTP errors. First-page payloads were 43.5 KB for Rizzy and 54.9 KB for Fizzy because the signed IDs and representation URLs have different lengths. The preview encoders also produce different bytes. This is a short sequential read diagnostic, not a like-for-like transfer or maximum-scale result.

The [200-card write check](results/drafts-write-2026-09-25.json) and [2,000-card follow-up](results/drafts-write-2000-2026-09-25.json) used 32 concurrent clients per target. In the longer fixed-count run, Rizzy completed 2,000 creates in 0.57 seconds (3,498/s, 11.1 ms p95) and Fizzy in 19.4 seconds (103/s, 560.3 ms p95), both with zero HTTP errors and unique card numbers. Rizzy's database contained all 2,000 corresponding publication events, creator watches, and search entries. The targets still differ in background work and unimplemented features; the Rust sample is under one second and does not establish sustained write capacity. A duration-based run with equal resource budgets, CPU and memory measurements, and fully matched side effects is still needed.

A local tuning probe of the five-endpoint workload measured 8, 12, and 24 SQLite connections. The 12-connection default was fastest in that probe; `RIZZY_DB_POOL_SIZE` allows a different value from 1 to 64 when deployment measurements justify it.

The harness supports `--client rust` after `cargo build --release --bin rizzy_load`. The independent Tokio client reads every response body and records per-endpoint request counts and bytes. `FIZZY_PID` and `RIZZY_PID` enable Linux process-tree sampling for CPU time, proportional set size, and process I/O. Its credentials remain in environment variables. Both clients still run each target sequentially on the same host. A different load-generator host, matched deployment budgets, full response parity, writes, and longer runs remain necessary for production-scale claims.

The [30-second capacity sweep](results/scoped-api-rust-client-capacity-2026-09-25.json) and [two-order 20-second repeat](results/scoped-api-rust-client-repeated-2026-09-25.json) used fresh isolated accounts and a balanced four-path 100-card JSON fixture. The repeat checked equal board names, card state and text, first-page title order, activity actions and descriptions, description HTML, JSON shapes, counts, and totals before timing. Per-endpoint completed request counts differed by at most one in each run; no timed requests failed. At 128 clients, Rizzy served 2,851–2,951 requests/s with 61.9–64.1 ms p95 in the repeat; Fizzy served 1,038–1,052 with 197.2–213.2 ms p95. With a 100 ms p95 threshold and zero errors, Rizzy's last passing **tested** level was 128 clients and Fizzy's was 32; both failed the threshold at 256 clients, and Fizzy failed it at 64. Rizzy averaged roughly 10 server CPU cores at 128 clients versus Fizzy's 16 Puma-worker cores. Warm process-tree proportional memory was approximately 63 MiB for Rizzy and 5.8–5.9 GiB for Fizzy in these deployments. The load client consumed about 0.1 CPU core, below server use. The sampled filtered-card first page was 23.9 KB versus 24.7 KB (Rizzy versus Fizzy); other response bodies also differed in size. Process I/O counters were near zero on these warm reads. The mixed response bytes, partial application parity, single-host sequential runs, different worker and connection counts, and 20–30 second windows limit this to a local read-capacity result, not maximum production scale or a whole-app speed claim.

The [board query probe](results/board-column-index-2026-09-25.json) isolates the Rust app's board reads on 30,000 synthetic cards in one in-memory SQLite board. With the previous board-active index, the first-page named-column query needed a temporary sort and measured about 1.6 ms median across 60 warm reads. With `cards_board_column_active`, SQLite used the board and column keys directly and measured about 0.014 ms median. Startup migration on an existing Rizzy database created the index, and `EXPLAIN QUERY PLAN` confirmed it was selected. The eager initial-board query ranked every card before returning 15 per column and measured about 42 ms median. The revised initial query counts columns and fetches only the first 15 Maybe cards, measuring about 3 ms median; other columns are fetched on demand. These are in-memory SQLite query timings, excluding HTML rendering and HTTP, and are not a Fizzy comparison or an end-to-end scale result.

The account-export benchmark used fresh production Fizzy and release Rizzy accounts with 500 matching cards and 500 comments each. The [first run](results/account-export-500-2026-09-25.json) exposed Rizzy's one-second worker poll: its three request-to-ready times were 0.21, 0.97, and 0.99 seconds versus Fizzy's 0.48–0.88 seconds. It also found a missing system user in Rizzy's seeded account. After backfilling that user and adding an in-process worker wakeup, both archives contained 3,507 entries, including equal counts for cards, comments, events, watches, and Action Text records. The [next three](results/account-export-500-notify-2026-09-25.json) and [three repeat](results/account-export-500-notify-repeat-2026-09-25.json) runs gave Rizzy a median ready time of 0.066 seconds versus Fizzy's 0.704 seconds, about 10.7 times shorter for this fixture. Polling remains for cross-process recovery. The Fizzy ZIP was 1.38 MB and the Rizzy ZIP about 1.24 MB; archive metadata, serialization, and some fields still differ. These were sequential local runs with different worker counts and incomplete product parity, so they do not establish overall export throughput or maximum scale.

The [user-export index probe](results/user-export-comment-index-2026-09-25.json) copied a 2,000-card Rizzy database twice and added one comment to each card. Without an index on `comments(card_id,created_at)`, the 2,000 comment lookups used table scans and took 83 ms in an in-process SQLite probe; with the index, they used indexed lookups and took 5 ms. Three completed ZIP exports with the same 2,000 JSON entries took 2.92–3.14 seconds before and 0.85–1.84 seconds after. This compares two local Rizzy debug builds and includes the export worker's polling delay, so it does not measure Fizzy or isolate server CPU time.

The browser [activity timeline benchmark](results/events-html-adaptive-2026-09-25.json) created 100 published cards and events per target, verified 100 visible events and three columns, and requested each filtered HTML timeline using a browser session. With release Rizzy and production Fizzy, the adaptive day query served 1,012 versus 61 requests/s at one client, 807 versus 500 at eight, and 666 versus 781 at 32. At 32 clients its p95 was 57.4 ms versus Fizzy's 81.1 ms, but its throughput was lower. The [first six-query version](results/events-html-2026-09-25.json) served 584 requests/s at 32 clients; a [window-function attempt](results/events-html-ranked-2026-09-25.json) regressed to 465 and was replaced. A [32-connection probe](results/events-html-adaptive-pool32-2026-09-25.json) served 566 versus 787 requests/s at 32 clients, worse than the default 12 connections. These two- to three-second sequential diagnostics do not establish greater maximum scale. Fizzy's page was about 242 KB versus Rizzy's 47 KB because Rizzy still has simpler markup, so the HTML workloads are not yet equivalent.

The activity timeline now returns source-style conditional responses. The [first full-response run](results/events-html-etag-full-2026-09-25.json) measured 636 Rizzy versus 735 Fizzy requests/s at 32 clients; Rizzy's extra validator query did not remove its full-page throughput gap. The [matching ETag run](results/events-html-etag-conditional-2026-09-25.json) returned 304 on every timed request and measured 5,157 versus 2,109 requests/s at 64 clients, with p95 22.6 versus 49.6 ms. A one-second, per-user HTML cache with single-flight rendering then produced a [warm full-response run](results/events-html-etag-cached-full-2026-09-25.json) of 6,980 versus 981 requests/s at 64 clients. The [final release build repeat](results/events-html-etag-cached-final-2026-09-25.json) measured 6,909 versus 1,052 requests/s at 64 clients, with p95 17.1 versus 114.7 ms and no HTTP errors. All runs verified 100 visible events and three columns, but Rizzy's page was about 47 KB versus Fizzy's 248 KB. They used separate three-second sequential samples, an existing Fizzy account, a fresh Rizzy account, and different worker counts; their differences are diagnostics for these reads, not one-for-one page or sustained scale proof. The benchmark harness now removes its fixture boards and can measure conditional requests with `--conditional`.

The [matching board ETag run](results/board-html-conditional-2026-09-25.json) used a 100-card fixture on each app, checked 15 visible first-page cards, then verified both returned empty 304 responses for matching validators. At 32 clients, Rizzy served 1,957 versus Fizzy's 1,161 requests/s, with p95 18.3 versus 43.4 ms; at 64 clients it served 1,986 versus 1,123 requests/s, with p95 35.0 versus 75.9 ms. The release Rust server used its default connection pool and Fizzy ran with eight Puma workers. Each sample lasted three seconds, the accounts had different existing contents, and the 200-response pages were 47 KB versus 156 KB. The identical empty 304 bodies make this a useful conditional-read comparison, but it does not establish equivalent page behavior or sustained application scale.

The subsequent [same-host Rust before/after run](results/board-conditional-fastpath-2026-09-26.json) uses `board_conditional_fastpath.py` with separate 100-card databases and release binaries at `c52402c` and the candidate change. A dedicated SQLite connection now lets Rizzy return an unchanged board's 304 before querying and rendering its content; a commit from another process invalidates that cached validator. At eight clients, conditional throughput moved from 2,379 to 4,691 requests/s (p95 5.03 to 2.83 ms); at 32 it moved from 1,924 to 4,635 requests/s (p95 18.63 to 12.4 ms). Full-response throughput stayed close at 2,392 versus 2,441 and 1,893 versus 1,870 requests/s. Both builds returned the same 46,890-byte page and no HTTP errors. These sequential three-second runs isolate this Rust optimization; they are not a new Fizzy comparison or sustained capacity measurement.

## Write contention

`column_delete.py` compares an authenticated column delete after inserting equal
card counts into each app's SQLite database. It times only the request and
checks that every card's update time changes, activity time stays fixed,
deleted-column cards become untriaged, and the other column's timestamp stays
fixed. The [pre-optimization run](results/column-delete-sqlite-before-bulk-touch-2026-09-26.json)
measured a 33.3 ms median for Rizzy versus 17.6 ms for Fizzy at 10,000 cards.
The [guarded, split-update run](results/column-delete-sqlite-guard-split-2026-09-26.json)
reduced Rizzy to 21.1 ms versus Fizzy's 16.0 ms; at 1,000 cards Rizzy took
2.9 versus 15.1 ms. A [30,000-card run](results/column-delete-sqlite-guard-split-30000-2026-09-26.json)
measured 70.0 versus 36.4 ms. These are three single-request trials per size
on the same two CPUs, with two Fizzy workers and one Rust process. Cards were
cloned from API-created records directly in each database; fixture insertion
and card-creation side effects were outside the timed request. The larger
results identify a remaining Rizzy write bottleneck, not greater scale.

The [SQLite one-pass trial](results/column-delete-sqlite-single-pass-2026-09-27.json)
combined unlinking and timestamp updates in a single `UPDATE`, matching the
approach used by Rizzy's MySQL adapter. The column lifecycle parity test passed,
but three fresh paired trials measured 24.8 ms median for Rizzy versus 14.9 ms
for Fizzy at 10,000 cards, and 94.0 versus 32.9 ms at 30,000. This was worse
than the earlier SQLite split-update results, so the one-pass change was
reverted. The wide ordered column index includes `column_id`; assigning that
field on every row is a likely cause, but this trial does not isolate its cost.
A [same-day split-update recheck](results/column-delete-sqlite-split-recheck-2026-09-27.json)
measured Rizzy medians of 22.2 and 74.8 ms at 10,000 and 30,000 cards,
versus 24.8 and 94.0 ms for the one-pass trial. Its artifact records the
release binary SHA-256 and Git revision; both trials used three fresh paired
fixtures per size and the same two CPUs. The large delete still trails Fizzy.

The [index trade-off probe](results/column-index-tradeoff-2026-09-26.json)
uses `column_index_tradeoff.py` on three disposable copies of one 30,000-card
Rizzy fixture. The current wide column index gave 0.019 ms median first-page
reads, 0.255 ms counts, and 35.4 ms to clear 10,000 column links. Replacing
it with `(board_id,column_id)` improved that write to 19.2 ms but raised the
first-page read to 2.37 ms and count to 1.95 ms. A partial ordered index kept
the read fast but did not materially improve the write. These are direct SQL
measurements, not HTTP timings; the wide index remains because named-column
reads use it.

The [expanded index probe](results/column-index-tradeoff-maybe-pair-2026-09-26.json)
also measures the untriaged Maybe column on the same 30,000-card fixture. A
partial copy of the wide index that excludes null `column_id` values lowered
the 10,000-card unlink step from 34.6 to 20.6 ms, but raised the Maybe first-page
read from 0.022 to 2.411 ms. Adding a compact Maybe index restored that read
to 0.027 ms, while unlink rose to 32.6 ms and the Maybe count took 2.054 ms
versus 0.275 ms with the current index. All figures are medians of three
disposable database copies with 100 warmed reads per copy. Neither partial
layout preserves both read paths and materially improves deletion, so the
current index remains in place.

For MySQL, `mysql_column_delete.py` times the authenticated DELETE on fresh
1,000- and 10,000-card fixtures and verifies every card's timestamp and
placement afterward. The [row-epoch comparison](results/mysql-column-delete-epoch-1000-10000-2026-09-26.json)
measured a 10,000-card median of 144.7 ms for Rizzy versus 62.9 ms for Fizzy;
at 1,000 cards, the medians were 16.0 and 26.1 ms. A separate
[five-pair Rust binary A/B run](results/mysql-column-delete-epoch-ab-2026-09-26.json)
used `mysql_column_delete_ab.py`, fresh databases, the same two CPUs, and
alternating request order. The row-epoch build improved the 10,000-card median
from 182.5 to 149.0 ms and the 1,000-card median from 19.5 to 16.3 ms;
all five pairs improved at both sizes. These are single-request local timings,
not sustained write throughput. The large-column delete remains slower than
Fizzy, and the overall application speed and scale goals remain open.

The next MySQL change combines column unlinking and card timestamp updates
into one card-table pass. A [direct SQL probe](results/mysql-column-delete-single-pass-sql-probe-2026-09-26.json)
measured 152.1 ms median for that pass and surrounding transaction versus
195.7 ms for the two-pass row-epoch variant on 10,000 cards. The
[five-pair API A/B run](results/mysql-column-delete-single-pass-ab-2026-09-26.json)
measured 114.3 ms median for the new binary versus 145.7 ms for the previous
one at 10,000 cards, with all five pairs faster. At 1,000 cards, the repeat
was mixed and its new median was slower (18.0 versus 16.2 ms), so that size
does not support an improvement claim. The clean-revision
[Fizzy comparison](results/mysql-column-delete-single-pass-vs-fizzy-10000-2026-09-26.json)
measured 113.4 ms for Rizzy versus 70.3 ms for Fizzy at 10,000 cards.
The MySQL core test checked the new single epoch increment, card placement,
card and board timestamps, and the untouched second column. The SQLite
column lifecycle comparison remained equal to Fizzy. Large-column deletion
is faster than the previous Rust build but still trails Fizzy on this fixture.

The [MySQL index-layout probe](results/mysql-column-index-layouts-2026-09-26.json)
tested the same 10,000-card deletion with the wide ordered column index and
two smaller alternatives. Their delete times changed little, while the
40-query named and Maybe first-page read batches went from roughly 2 ms with
the wide index to about 330 ms with either smaller index: MySQL selected a
board index and filesorted the result. The wide index remains. A separate
[forced-index probe](results/mysql-column-index-force-2026-09-26.json)
showed that MySQL's unhinted named-column count used a slower plan; 40 counts
took 297 ms median versus 22 ms when forced onto the existing column index.
The [authenticated binary A/B](results/mysql-column-count-hint-ab-2026-09-26.json)
then measured three fresh 10,000-card trials of 40 named-column first-page
requests per build. The median of trial medians fell from 8.664 to 1.475 ms,
with matching response totals. The board route used an unchanged count query
but also varied between builds, so its timings are a control for local run
noise, not an effect credited to the named-column hint. These are sequential
single-client reads on one MySQL host; concurrent capacity and full app parity
still need measurement.

The [five-step card workflow](card_workflow.py) creates a published card, edits its title and description, adds a comment, closes it, and reads its final state. It compares JSON response shapes in a preflight, then checks unique numbers, closed-card totals, four activity events per card, and a three-comment sample after each run. Fixture boards are deleted afterward. On [two reversed-order 500-workflow rounds](results/card-workflow-500-2026-09-26.json), at eight clients Rizzy completed 418–457 workflows/s versus Fizzy's 28.5–32.1; at 32 clients it completed 432–442 versus 28.5–28.6. Every run had zero HTTP errors and the expected 500 cards and 2,000 activities. Rizzy's p95 full-workflow latency was 19.7–21.5 ms at eight clients and 77.5–82.7 ms at 32; Fizzy's was 339–360 ms and 1,400–1,408 ms, respectively. A [100-workflow pilot](results/card-workflow-2026-09-26.json) passed the same checks, but its Rust runs were too short for a useful sustained-rate estimate.

A two-process Rust run exposed a remaining SQLite lock error when a title change inserted its system comment. The edit, comment, closure, event, and system-comment transactions now use `BEGIN IMMEDIATE` to reserve the writer across processes. `tests/cluster_card_workflow.py` runs 600 workflows through two Rust processes, checking 600 closed cards, 2,400 events, and 1,800 comments; ten fresh repeats passed. The [post-fix 500-workflow comparison](results/card-workflow-500-immediate-2026-09-26.json) measured 420–450 Rizzy versus 27.5–29.4 Fizzy workflows/s at eight clients and 412–438 versus 28.6–30.4 at 32, with zero HTTP errors and the expected record counts. Rust's p95 workflow latency was 20.8–21.7 ms at eight clients and 80.9–85.8 ms at 32. These local runs do not show that adding Rust processes increases SQLite write capacity; SQLite still serializes writers.

The [Rust cluster capacity run](results/cluster-card-workflow-2026-09-26.json) tests that limit directly. It uses a fresh SQLite database for each sample, alternates one, two, and four release processes over two rounds, and checks all 1,000 closed cards, 4,000 events, and 3,000 comments in each sample. At eight clients, one process completed 401–416 workflows/s, two completed 349–360, and four completed 315–320. At 32 clients, one completed 395–420, two completed 383–397, and four completed 362–369. Every sample had zero HTTP errors. The 32-client p95 workflow latency rose from 85.9–88.9 ms on one process to 232.6–263.3 ms on four. These results contradict a claim that Rizzy's SQLite write path scales by adding processes; a multi-writer storage implementation and sustained mixed-workload measurement are still required for the user's scale goal. Reproduce with `python3 bench/cluster_card_workflow.py --workflows 1000 --concurrency 8,32 --processes 1,2,4 --rounds 2`.

These are sequential local runs on SQLite with the same five API operations and empty benchmark accounts at the start of each run. Fizzy used eight production Puma workers and an existing database containing other accounts; Rizzy used one release process and a fresh database. Fizzy's queue worker was disabled, while HTTP requests still performed source queue inserts and callbacks. Response shapes, selected values, and event/comment counts matched, but all callback effects, rendered HTML, and background deliveries were not compared. The Rust 500-workflow samples lasted about one second, and neither target was driven to a measured saturation limit or tested across hosts. This is route-level write evidence, not a whole-app speed or maximum-scale claim. Set the usual `FIZZY_*` and `RIZZY_*` URL, account, and token variables, then run `python3 bench/card_workflow.py --workflows 500 --concurrency 8,32 --rounds 2`; optional `FIZZY_PID` and `RIZZY_PID` add Linux process-tree resource samples.

The [first card creation run](results/write-before-queue-2026-09-24.json) showed that Rizzy lost throughput as concurrent SQLite writers contended for the database lock. A Tokio mutex now queues the card number allocation and insert transaction in one process. The [repeat with 500 card writes per level](results/write-500-2026-09-24.json) produced the following diagnostic results:

| Concurrent clients | Fizzy writes/s | Rizzy writes/s | Fizzy p95 | Rizzy p95 |
| ---: | ---: | ---: | ---: | ---: |
| 8 | 99.2 | 316.0 | 131.2 ms | 32.1 ms |
| 32 | 104.2 | 316.9 | 552.3 ms | 116.9 ms |
| 64 | 104.8 | 317.6 | 840.1 ms | 217.0 ms |

Each run had zero HTTP errors and unique card numbers. Fizzy performs additional event, search, notification, and job work on each write, which Rizzy has not fully replicated. These numbers show the current card creation path under contention, not full application write parity. The local mutex queues writes within one Rizzy process; the atomic SQLite account-row update allocates numbers across processes. `tests/cluster_writes.py` checks 100 concurrent creates through two processes for errors and duplicate or missing numbers. SQLite still serializes writers, so this check does not establish clustered throughput.

The [current write run](results/write-current-2026-09-24.json), after adding creator watches and richer card responses, measured 314 Rizzy writes per second versus 107 Fizzy writes per second at 32 clients (200 cards each). Both had zero errors and unique card numbers. The same side effect and clustered deployment limits still apply.

After adding initial notification records, the [32-client card-create rerun](results/notifications-write-2026-09-25.json) measured 288 Rizzy writes/s versus 99 for Fizzy (200 cards per target). Both had no HTTP errors and unique card numbers; p95 was 119 ms for Rizzy versus 528 ms for Fizzy. The fixture has only the creating user on each board, so it exercises event creation and notification recipient checks but not a populated notification fanout. The applications still perform different background work, and this single run does not establish equivalent write capacity.

With FTS5 indexing enabled, the [32-client write rerun](results/fts-write-2026-09-25.json) measured 300 card creates/s for Rizzy versus 103 for Fizzy (200 each). Both had zero HTTP errors and unique card numbers; p95 was 125.9 ms for Rizzy versus 531.8 ms for Fizzy. This fixture still has no notification fanout or subscribed webhook, so it is not a full side-effect parity benchmark.

In a two-process Rizzy probe, 500 card creates were split across two servers sharing one SQLite file. All 500 succeeded with unique account-wide card numbers in 2.17 seconds (about 230 writes/s). That is slower than the single-process write run because SQLite still admits one writer at a time. The result confirms this small clustered correctness case but does not demonstrate increased write capacity from adding processes.

With opaque User attachment IDs and mention parsing enabled, the [validated 200-card check](results/write-user-attachables-2026-09-25.json) measured 2,337 Rizzy creates/s versus 102 Fizzy creates/s at 32 clients. The Rizzy sample lasted only 0.09 seconds, so it is a correctness and latency probe rather than a sustained throughput measurement. The [three-second fixed-duration run](results/write-duration-user-attachables-2026-09-25.json) then measured 3,045 versus 104 creates/s at 32 clients (p95 12.2 versus 513.9 ms), and 3,020 versus 100 at 64 clients (p95 24.9 versus 902.8 ms). Both had zero HTTP errors and unique numbers. Their newly created board card counts and publication-event counts matched successful writes: 9,162 versus 337 at 32 clients, and 9,133 versus 359 at 64. These boards contained no User attachments, webhooks, or notification fanout; the apps still differ in background work and unimplemented behavior. Targets ran sequentially on one host with a Python client, and this run did not measure CPU, memory, or database I/O. It does not prove equivalent sustained capacity or a product-wide speed advantage.

The [100-step index read run](results/steps-index-100-2026-09-27.json) follows a paired conditional-response check, then measures full JSON reads on fresh equal-content fixtures. Eight Fizzy Puma workers and one Rust process shared the same eight assigned CPU cores; a separate Rust client drove two reversed-order three-second rounds at eight and 32 clients. Response bodies were 9,067 bytes in both apps and all phases had zero HTTP errors. Rizzy served 2,841–2,905 requests/s versus Fizzy's 584–688, with lower p95 latency. This is a short, single-host read comparison, not product-wide speed or maximum capacity. Reproduce with `python3 bench/steps_index.py --steps 100 --seconds 3 --clients 8,32` after building the Rizzy and load-client release binaries.
