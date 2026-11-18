# P5 — Interactive decoding: design

**Status:** proposal (2026-11-18). Nothing here is implemented yet; this
document exists to make the P5 decisions explicit before any code is written.

**Objective.** Open a 16K × 16K image, navigate it, edit a small region and
save it, without ever holding the whole image in memory. P5 is about
**interactive performance** — the latency a person feels while panning and
zooming — not about making a batch decoder faster.

---

## 1. What the format already gives us

P5 needs **no format change**. This is worth stating first, because it
removes the largest risk from the milestone.

* The header carries `directory_offset` and `tile_count`, so the directory
  is a single fixed region that can be read without touching a payload
  (specification §1, §4).
* Every directory entry carries `payload_offset`, `compressed_length` and
  `uncompressed_length`, so any tile's payload can be addressed directly
  (specification §4.1). Payloads are not required to be contiguous, so
  random access does not depend on the writer's layout.
* Tiles are independent: no tile's payload depends on another tile's, so
  tiles can be decoded in any order, on any number of threads, and in
  parallel with anything else (specification §5, ADR-0005).
* `vpng_decoder::Document::tile_payload` and `decode_tile` already decode a
  single tile, and `vpng_editing::FileSource` already reads one payload at a
  time from disk. The random-access machinery exists; it is simply
  sequential and uncached today.

The consequence is that P5 is an **engine** milestone: the bytes on disk
stay exactly as they are, and the golden fixtures do not move.

## 2. What exists today, and what P5 adds

| Capability | Today | P5 |
|---|---|---|
| Random access to one tile | yes (`FileSource`, `Document::tile_payload`) | unchanged, shared by the reader |
| Region decode | yes, sequential (`decode_region`) | parallel, priority-ordered |
| Decoded-tile cache | yes, bounded byte-budget LRU (`TileCache`) | shared, single-flight, hit accounting |
| Concurrent decode | **no** | bounded worker pool |
| Request scheduling | **no** | priority queue: viewport first, prefetch behind it |
| Prefetch | **no** | predictive, bounded, cancellable |
| Duplicate decode prevention | **no** | in-flight deduplication (single flight) |
| Cancellation | **no** | obsolete viewport requests are dropped |
| Instrumentation | partial (cache counters, timings) | latency, throughput, hit rate, queue depth, RSS |

## 3. The components

### 3.1 Streaming reader

A `TileReader` over a `File`: read the header and the directory once, then
read only the payloads of the tiles asked for. It owns no pixel memory; it
hands out compressed payload bytes. This is `FileSource`'s file half,
extracted so that the decoder and the editor share it.

* **Bound:** the directory is `tile_count * 40 + 4` bytes — for 16K at the
  512² tile size the P4 matrix used, that is 1,024 entries, 40,964 bytes
  (40 KiB); at 1024² tiles it is 256 entries, 10,244 bytes. Negligible, but
  it must be stated in the memory report rather than assumed.
* **Invariant:** the reader never reads a payload it was not asked for, and
  never reads the same payload twice (the reader keeps the bytes it has
  already fetched only if the cache asks it to).

### 3.2 Parallel tile decoder

A bounded worker pool (default: `min(available_parallelism, 8)` workers,
overridable). Each worker takes a tile index, asks the reader for the
payload, decodes it through the same `get_codec(entry.codec_id)` path the
sequential decoder uses, and publishes the decoded tile.

* **Correctness first:** the decoded bytes of a tile must be identical to
  what the sequential decoder produces, byte for byte. There is one decode
  path; parallelism only decides *who* runs it and *when*.
* **No shared mutable state:** a tile is decoded from its payload and its
  directory entry alone. The only shared state is the cache and the
  scheduler, both of which are explicitly synchronised.

### 3.3 Bounded scheduler with viewport priority

Requests are not equal. A tile under the user's viewport must be decoded
now; a prefetched neighbour can wait; an obsolete request must be dropped.

* A **priority queue** keyed by `(request class, distance from the viewport
  centre, sequence number)`. Request classes: `Visible` (in the viewport),
  `Prefetch` (adjacent, predicted), `Background`.
* **Bounded depth:** the queue holds at most *N* outstanding requests
  (*N* = a small multiple of the worker count). A full queue refuses a
  `Background` request and does not block the caller.
* **Cancellation:** a new viewport invalidates the previous one. Every
  request carries the viewport generation it was issued under; when the
  generation advances, queued requests from older generations are dropped
  before they are dispatched. A request already running is allowed to
  finish — killing a decode mid-way would waste the work — but its result is
  discarded if its generation is stale.
* **Fairness:** a viewport request always outranks a prefetch request, and a
  prefetch request never delays a visible tile.

### 3.4 Predictive prefetch

When a viewport is requested, the scheduler also queues the tiles most
likely to be needed next.

* **Direction of travel:** the last viewport movement gives a direction; the
  tiles in that direction are preferred over the tiles behind it.
* **Neighbourhood:** the ring of tiles immediately around the viewport.
* **Bounded and cancellable:** prefetch is a *request class*, not a special
  path, so it obeys the same queue bound and the same cancellation rules.
  If the user pans the other way, the stale prefetches are dropped.
* **Policy is a decision, not a constant:** the first implementation should
  make the policy explicit and measurable (ring radius, direction weight)
  so that the benchmark can show whether it helps or hurts. A prefetch that
  does not raise the cache hit rate is a prefetch that should be removed.

### 3.5 Cache coordination

The existing `TileCache` (bounded byte-budget LRU) becomes the single owner
of decoded tiles.

* **Single flight:** two requests for the same tile that arrive while it is
  being decoded wait for the one decode, not two. This is the "avoid
  duplicate decoding" requirement, and it is also a memory requirement: the
  tile exists once, not once per waiter.
* **One budget, shared:** the cache's byte budget covers decoded tiles; the
  scheduler's queue bound covers pending requests; the workers' buffers are
  counted separately. The memory report states all three.
* **Eviction stays LRU**, with one addition: an evicted tile that a waiter
  is still waiting for is not re-decoded for that waiter (the waiter is
  handed the tile it was waiting for).

### 3.6 Cancellation

Cancellation is a first-class concept, not an error path.

* A cancelled request returns a **distinct, non-error** result
  (`Cancelled`), because "the user panned away" is not a failure.
* A cancelled request that was never dispatched costs nothing.
* A cancelled request that was running costs one decode, and its result is
  dropped.
* The instrumentation counts cancellations separately from failures, so a
  report cannot mistake "the user panned away" for "the decoder is broken".

### 3.7 Instrumentation

Every measurement in the P5 report must come from a counter, not an
estimate. The record for a run:

| Metric | Definition |
|---|---|
| First-viewport latency | wall time from "open the file" to "the first viewport's pixels are available" |
| Viewport latency | wall time from "request a viewport" to "that viewport's pixels are available" |
| Tile throughput | tiles decoded per second, measured over a decode phase |
| Cache hit rate | hits / (hits + misses) |
| Queue depth | outstanding requests, sampled |
| Prefetch effectiveness | prefetched tiles that were later used / prefetched tiles issued |
| Cancellation rate | cancelled requests / issued requests |
| Peak RSS | process resident set, sampled |
| Heap peak | tracked allocation peak |
| Decoded bytes held | the cache's byte high-water mark |

The report states, for every memory figure, **what is included and what is
excluded** — the same rule the P4 memory report follows.

## 4. The acceptance test, as measurable criteria

The critical P5 acceptance test, spelled out:

1. Write a 16384 × 16384 RGBA8 VPNG image (1 GiB of canonical pixels)
   without decoding it whole.
2. Open it and request a small viewport.
   * **Criterion:** the full image is never allocated. The report shows the
     heap peak and the RSS peak, and both must stay inside the 64 MB CPU
     working-memory target (the same target P4 uses).
   * **Criterion:** only the tiles the viewport needs are decoded. The
     instrumented count must equal the viewport's tile count, not more.
3. Pan rapidly across the image.
   * **Criterion:** obsolete requests are cancelled, and the cancellation
     rate is reported. The report must not claim a latency improvement
     without the panned-latency measurement.
4. Prefetch likely next tiles.
   * **Criterion:** the prefetch policy's parameters are reported, and the
     cache hit rate with prefetch is compared against the hit rate without
     it. If prefetch does not help, the report says so.
5. Edit a small region, save and reopen.
   * **Criterion:** the pixels are exact, and **the payloads outside the
     edited region are byte-identical** to the pre-edit file. This is the
   same copy-on-write property P4 established, and P5 must not weaken it.
6. Report: first-viewport latency, tile throughput, cache hit rate, peak
   RSS and memory-budget compliance.

**The 16K rule applies.** If the 16K case cannot be run on the host, the
result is **BLOCKED**, not **PASS**, and the report says which case was
blocked and why. No 16K claim may be inferred from a smaller image.

**No speed claims without a baseline.** A latency figure is only meaningful
against something. The baseline is the **sequential** path that exists today
(`FileSource` + `decode_region`, one tile at a time, no cache). Every
comparison in the P5 report is against that baseline, measured on the same
host in the same run.

## 5. Open decisions

These are the questions P5 needs answered before the code is written. They
are listed because each one changes the design, not because they are hard.

1. **Worker count.** `min(available_parallelism, 8)`, or fixed? A fixed
   count is reproducible; the host-dependent one is faster on big machines
   and noisier in benchmarks.
   **Decided (stage 3):** the pool takes the count from its caller and
   never guesses. The benchmark chooses it, records it in the raw data and
   reports it; the recommendation for a caller is
   `min(available_parallelism, 8)`. A benchmark that does not state its
   worker count is not reproducible, so the number is part of the result,
   not part of the environment.
2. **Prefetch policy.** Ring radius and direction weighting. The first
   implementation should expose them as parameters and measure them.
   **Still open** — this is stage 4.
3. **The cache budget for viewing.** P4's cache budget is an editing
   budget. A viewing cache may want a different one, and the two may
   coexist (the editing engine's dirty tiles must never be evicted by a
   prefetch).
   **Decided (stage 3):** the cache moved out of `vpng-editing` into
   `crates/vpng-cache`, so there is one implementation of the bound and
   each engine holds its **own instance** with its own budget. The
   editing engine's dirty tiles cannot be evicted by the viewer's
   prefetch, because they are in different caches.
4. **Where the engine lives.** A new crate (`vpng-view`) beside
   `vpng-editing`, or a module inside `vpng-editing`. A separate crate keeps
   the editing engine's API stable and lets the viewer be tested on its own.
   **Decided (stage 2):** `crates/vpng-view`.
5. **Threading library.** `std::thread` only, or a small dependency. The
   workspace currently has no async runtime and no thread-pool dependency;
   adding one is a real decision for a format library.
   **Decided (stage 3):** `std::thread` only, with a `Mutex` and a
   `Condvar`. A format library should not drag an async runtime into its
   dependency graph, and the work here is CPU-bound and short.

## 6. A build order

The user's standing preference is module-by-module edits with a compile
check between each, and one heavy job at a time on this host.

1. **The reader.** Extract the file half of `FileSource` into a `TileReader`
   with an explicit "bytes read" counter. Test: the reader reads exactly the
   payloads it is asked for.
   **Done** — `crates/vpng-source`, 11 tests.
2. **The scheduler.** The priority queue and cancellation, with no threads
   yet — a single-threaded driver that consumes the queue in order. Test:
   ordering and cancellation are exact.
   **Done** — `crates/vpng-view/src/scheduler.rs` and `driver.rs`, 17 tests.
3. **The parallel decoder.** The worker pool on top of the scheduler. Test:
   the decoded bytes are identical to the sequential decoder's, for every
   format and tile geometry.
   **Done** — `crates/vpng-view/src/pool.rs` and `file_decoder.rs`, 8 tests.
   The byte-identity test covers a 512×384 RGBA8 file with 48 tiles; the
   *every* format and tile geometry part is not yet covered and is
   recorded as such.
4. **Prefetch.** The policy, the parameters, the measurement.
   **Not started.**
5. **The benchmark.** The acceptance test as a runnable program, producing
   the raw JSON, with the sequential baseline measured in the same run.
   **Not started.**
6. **The report.** `docs/P5_REPORT.md`, with every figure bound to the raw
   data by a check in `benchmarks/verify_reports.py`, exactly as P3.5 and
   P4 did.
   **Not started.**

Each step ends with a compile check and a test run before the next begins.
