# Comparative load runs

`compare.py` creates the same number of cards in a fresh board on each server. It checks board, card, and activity JSON shapes, description HTML, first page sizes, and total counts before measuring a read workload: board list, board detail, card detail, filtered card list, and board activity feed. Each endpoint receives one fifth of the requests. Requests are authenticated with the same permission level. The script reports throughput, latency percentiles, and errors for each concurrency level. Runs before the activity mix was added used four endpoints, one quarter each.

The [initial result](results/initial-2026-09-24.json) used the Fizzy source at `477c943e0506f109e5bc83ae9dadbe519732c045`, Ruby 3.4.8, Rails production, Puma's default 16 workers and 1 thread each, SQLite, and a Rust release build with 12 SQLite connections. Both ran on one 32 logical CPU AMD Ryzen AI Max+ 395 machine. The fixture had 100 cards per board and each first card page had 15 cards. The four sampled response shapes and description HTML matched; serialized first page size was approximately 24 KB for each.

The Rust build processed 1,467, 4,714, and 4,707 requests per second at concurrency 1, 8, and 32 respectively, versus 122, 1,244, and 1,653 for Fizzy. At concurrency 32, p95 was 12.4 ms for Rizzy and 43.1 ms for Fizzy. These are short five second diagnostic runs on a partial implementation. They do not establish a one-for-one speed or scale advantage. The apps still differ in many features, queries, side effects, caching, and deployment behavior. The results need repeated runs with equal resource budgets and full response and behavior parity before making a product claim.

After adding assignees to card responses and more API routes, the [current read run](results/current-2026-09-24.json) measured 4,306 requests per second for Rizzy versus 1,580 for Fizzy at concurrency 32, with p95 16.5 ms versus 45.6 ms. The four sampled response shapes still matched. This is also a five second diagnostic on a partial implementation.

The [higher-concurrency run](results/high-concurrency-2026-09-24.json) measured 4,327 versus 1,549 requests per second at 64 clients, with p95 38.4 versus 86.1 ms. At 128 clients, the figures were 4,280 versus 1,439 requests per second, with p95 77.9 versus 156.2 ms. For this specific read workload and a 50 ms p95 target, Rizzy met the target at 64 clients while Fizzy's last measured passing point was 32 clients. The Python generator may cap throughput, and these numbers cannot be extrapolated to overall application capacity.

After adding stored activity events and board/card timestamp updates, the [current read run](results/with-events-and-touches-2026-09-24.json) measured 4,378 versus 1,580 requests per second at 32 clients (p95 18.5 versus 46.0 ms) and 4,362 versus 1,386 at 64 clients (p95 39.4 versus 91.8 ms). The [current 32-client write run](results/write-with-touches-2026-09-24.json) measured 303 versus 102 card creates per second, both without HTTP errors or duplicate card numbers. Fizzy still performs search indexing, webhook dispatch, notifications, and jobs that Rizzy does not, so the write result is not a fully equivalent operation.

Adding the activity feed to one fifth of read requests exposed repeated database lookups in Rizzy. In the [first five-endpoint run](results/activity-mix-2026-09-24.json), Rizzy reached 1,413 requests/s with 70.1 ms p95 at 32 clients, versus Fizzy's 1,148 requests/s with 63.9 ms p95. After batch loading activity relationships, the [repeat](results/activity-mix-batched-2026-09-24.json) reached 2,120 requests/s with 43.2 ms p95 for Rizzy, versus 1,175 requests/s with 59.8 ms p95 for Fizzy. At 64 clients, Rizzy measured 1,752 requests/s and 110.1 ms p95 versus Fizzy's 1,110 requests/s and 115.6 ms p95. These are five second local runs on partial implementations; activity payload shape and fixture event counts were checked before timing.

A local tuning probe of the five-endpoint workload measured 8, 12, and 24 SQLite connections. The 12-connection default was fastest in that probe; `RIZZY_DB_POOL_SIZE` allows a different value from 1 to 64 when deployment measurements justify it.

The benchmark client uses Python threads and runs each target sequentially; it can become a bottleneck at high concurrency. Further scale work should use a separate load generator, report CPU, proportional memory, database I/O and error rate, and define a latency service level objective before comparing maximum sustainable throughput.

## Write contention

The [first card creation run](results/write-before-queue-2026-09-24.json) showed that Rizzy lost throughput as concurrent SQLite writers contended for the database lock. A Tokio mutex now queues the card number allocation and insert transaction in one process. The [repeat with 500 card writes per level](results/write-500-2026-09-24.json) produced the following diagnostic results:

| Concurrent clients | Fizzy writes/s | Rizzy writes/s | Fizzy p95 | Rizzy p95 |
| ---: | ---: | ---: | ---: | ---: |
| 8 | 99.2 | 316.0 | 131.2 ms | 32.1 ms |
| 32 | 104.2 | 316.9 | 552.3 ms | 116.9 ms |
| 64 | 104.8 | 317.6 | 840.1 ms | 217.0 ms |

Each run had zero HTTP errors and unique card numbers. Fizzy performs additional event, search, notification, and job work on each write, which Rizzy has not fully replicated. These numbers show the current card creation path under contention, not full application write parity. The single process mutex also does not coordinate multiple Rizzy processes; clustered deployment needs a database coordinated allocation strategy.

The [current write run](results/write-current-2026-09-24.json), after adding creator watches and richer card responses, measured 314 Rizzy writes per second versus 107 Fizzy writes per second at 32 clients (200 cards each). Both had zero errors and unique card numbers. The same side effect and clustered deployment limits still apply.

In a two-process Rizzy probe, 500 card creates were split across two servers sharing one SQLite file. All 500 succeeded with unique account-wide card numbers in 2.17 seconds (about 230 writes/s). That is slower than the single-process write run because SQLite still admits one writer at a time. The result confirms this small clustered correctness case but does not demonstrate increased write capacity from adding processes.
