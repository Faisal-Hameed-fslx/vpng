# Changelog

All notable changes to VPNG are recorded here. The format follows Keep a
Changelog; versions follow semantic versioning, with the format version tracked
separately as major.minor in the header.

## [Unreleased]

### Added

* **The preview pyramid and the adaptive partition, in one format revision
  (Phase B, P6 and P7).** The container's minor version moves **0.1 → 0.2**
  and the layout does not move with it: the main header is still 64 bytes
  with the same fields at the same offsets, the directory entry is still 40
  bytes, and the file is still header, directory, payloads.
  * **What v0.2 changes is two rules.** The directory entry's `flags` field
    carries the **level index** in its low byte (0 is the original, 1 and up
    are derived previews); the upper byte is reserved and must be zero, and a
    decoder that finds it set rejects the file with `UnsupportedFeature`. And
    a level's tiles may now have **different sizes**: they are an *exact
    partition* of the level's rectangle — every pixel covered once — checked
    by a sweep-line validator before a file is written, so a gap or an
    overlap cannot reach a file.
  * **The default path is unchanged.** `Header::new` still writes **v0.1**, so
    the golden fixtures and every existing file stay byte-identical. The
    v0.1 file this milestone produced has the SHA-256
    `d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf` —
    **the P5.2 tile-256 baseline's digest**. The pyramid and the adaptive
    partition are opt-in (`--pyramid`, `--adaptive`).
  * **P6 — the preview pyramid.** The encoder builds derived levels with an
    integer-only 2 × 2 box average (round-half-up, no floating point, alpha
    averaged like the other channels) and stops when the next level would
    not be strictly smaller in both dimensions. The measured result on the
    corpus's own 15,360 × 8,640 RGB photograph (398,131,200 canonical
    bytes), five runs per case: the whole picture is available in
    **197.94 ms** at 1/64 scale against **13062.32 ms** at full resolution —
    **66× sooner** — and the cost is stated: **+32.7% bytes**
    (164,873,298 against 124,197,035) and **+44.2% encode time**
    (26809.86 ms against 18590.46 ms). The encoder's reported ratio uses the
    *original* pixels, so a pyramid can never flatter it. The pyramid encode
    peaks at **547,844,096 B** of resident set and **541,421,345 B** of heap
    — the picture and its previews have to be held, and the report says so
    with the memory scope attached. This is **not** the 64 MB
    localized-edit target, which is a separate claim.
  * **P7 — adaptive tiling.** A planner picks tile sizes from the picture's
    own content: a coarse binary cut along the longer axis, splits on
    multiples of half the nominal tile, and a busiest-first greedy pass
    scored by the mean absolute neighbour difference sampled on a 64 × 64
    lattice. Measured over the **whole 750-image corpus** on the same
    canonical pixels, the result is unfavourable and kept: adaptive tiling is
    a **compression wash** — 452,762,594 bytes against the uniform grid's
    453,767,460 (−0.22%), with 330 images smaller, 241 larger and 179
    identical. It **hurts UI screenshots** (69 of 100 grow). What it does
    deliver is **half the tiles at the median** (4 → 2), a smaller directory
    and fewer requests to serve a region. All **750 of 750** adaptive files
    decode exactly and pass `vpng check`.
  * **The container's readers all learned about levels.** `vpng-decoder`
    exposes `Document::levels()`, `level()`, `base_level()` and
    `decode_level()`; `vpng-source` reports a level list; `vpng-cli` gained
    `--pyramid`, `--pyramid-levels N`, `--adaptive`, `--adaptive-threshold N`
    and `--level N` for `decode` and `extract-tile`; `vpng-view` plans
    viewports over a partition; and the editing engine commits a pyramid
    file by **dropping the previews** (reported as `preview_levels_dropped`)
    while keeping every untouched tile payload byte for byte.
  * **New documentation.** `docs/VPNG_SPEC_0_2.md` (the revision),
    `docs/decisions/ADR-0014-levels-and-partitions.md` (which supersedes
    ADR-0005 for v0.2), `docs/P6_P7_REPORT.md` (the report, with the twelve
    questions and the acceptance gates), `benchmarks/p6_preview_results.json`,
    `benchmarks/p7_adaptive_results.json` and `benchmarks/p6_memory_results.json`
    (the raw data), and
    `check_preview()` and `check_adaptive()` in
    `benchmarks/verify_reports.py`, which bind every quoted figure to it.
  * **The 64 MB localized-edit target was re-measured**, not assumed, after
    the partition refactor: the 12-case memory matrix is **12 PASS / 0 FAIL /
    0 BLOCKED**, and the worst case — a 16384 × 16384 RGBA16 image, 2 GiB of
    canonical pixels — edits with **13,770,752 bytes** of resident-set growth
    against the 67,108,864-byte budget, **20.5%** of it. Every case rewrites
    one tile and copies the rest byte for byte.
  * **The test suite is 418 passed / 0 failed** (was 368). The v0.2 revision
    added 50: 10 in `crates/vpng-testkit/tests/v2_formats.rs`, 6 in
    `crates/vpng-cli/tests/v2_features.rs`, and the rest in the crates the
    revision touched — the decoder's level derivation, the tiling
    partition's exact-cover validator, the encoder's pyramid and adaptive
    planner, and the editing engine's pyramid commit.
* **The 750-image corpus, and the distribution it makes measurable
  (Phase A, Step 3).** The nine-image seed is replaced by a real corpus:
  **750 images** in the five planned categories — photographs 300,
  screenshots 100, pixel art 100, scans 100, textures 150 — of which **746
  are real images** fetched from five public repositories at pinned commits
  and **4** are the project's own deterministic synthetic seed images.
  * **The corpus is a manifest of digests.** `benchmarks/corpus_manifest.json`
    (version 2) records, per image: the SHA-256 of its canonical pixels, the
    SHA-256 of the file it came from, the repository, the ref, the **commit**
    and the path inside it, the dimensions, the pixel format, the licence and
    the attribution. The image bytes are not redistributed — they stay under
    the git-ignored `corpus-data/` tree.
  * **The selection is deterministic and the skips are counted.** Candidates
    are ordered by the SHA-256 of their path, and the builder records what it
    rejected: 49 images that failed the pixel-art test (a long edge of at
    most 512 pixels and at most 64 distinct colours, alpha excluded), 23 it
    could not decode and 4 duplicates.
  * **Every image was verified, and every file checked.** All 750 images were
    re-canonicalised and compared with the manifest: **0 digest mismatches**.
    Every encoded file passed `vpng check`: **0 failures**.
  * **The headline result is unfavourable, and it is kept.** `auto` writes
    **0.415×** the canonical bytes; PNG writes **0.379×**, WebP lossless
    **0.242×** and JPEG XL lossless **0.219×**. **VPNG is behind PNG on every
    one of the five categories** — photographs 0.602 against 0.591,
    screenshots 0.063 against 0.031, scans 0.346 against 0.277, pixel art
    0.165 against 0.140, textures 0.378 against 0.367. The seed hinted at
    this; 750 images measure it.
  * **The 10 MB target succeeds on 748 of 750 images (99.7%)** — photographs
    299/300, screenshots 100/100, pixel art 100/100, scans 100/100, textures
    149/150. Both failures are named: the 16K photograph at **124,197,035**
    bytes and one 2,048 × 2,048 texture at **11,397,940** bytes.
  * **Auto selection never lost to the best forced codec** on any of the 750
    images, and was strictly smaller on **109** of them (1,541,810 bytes in
    total). The palette codec refused 556 images — more than 256 distinct
    pixel values — with a typed refusal rather than a silent fallback.
  * **Eighteen images expand**: their file is larger than their pixels, the
    worst a 1 × 1 texture at 37.0×, because the container's fixed overhead
    is larger than the image. The corpus records it rather than hiding it.
  * New: `benchmarks/corpus/build_corpus.py` (the builder),
    `benchmarks/corpus/sources.py` (the source registry),
    `benchmarks/render_corpus_tables.py` (the tables, rendered from the raw
    data), `docs/P5_CORPUS_REPORT.md` (the report, with the twelve
    questions), `benchmarks/p5_corpus_results.json` (the raw data), and
    `check_corpus()` in `benchmarks/verify_reports.py`, which binds every
    quoted figure to it.
* **The multi-run photographic benchmark and the latency matrix
  (`crates/vpng-validation`).** The measurement foundation: every case is
  repeated, and the report gives the **median** and the **p95**, not a
  single figure.
  * **The 16K photographic case ran** — it did not have to be reported
    BLOCKED. The corpus's own 15,360 × 8,640 RGB photograph (398,131,200
    canonical bytes) was encoded **120 times** and viewed repeatedly on one
    host, in one process: three tile sizes × four worker counts × ten runs.
  * **The headline result is that a single run cannot be trusted.** The
    run-to-run spread is **18.5% to 52.4% of the median**. That is why
    every earlier benchmark in this repository — all of them single runs —
    could not tell two configurations apart.
  * **More workers are faster and do not change the bytes.** At tile size
    256 the encode goes from **78793.67 ms to 12811.94 ms (6.15×)** at
    eight workers, and each tile size has **one SHA-256 across all forty
    of its runs**. The scaling is sub-linear and the report says so.
  * **The tile size, not the thread count, dominates interactive latency.**
    A pan at tile size 1024 costs **125.87 ms** against **21.44 ms** at
    256, because a pan across a boundary must decode a whole tile — 3,145,728
    pixels against 65,536.
  * **The parallel pool runs out of work at a large tile size.** At 1024 the
    viewport needs two tiles, so four workers take 135.85 ms and eight take
    **158.89 ms** — the bigger pool is slower. At 256 the same viewport
    needs 20 tiles and eight workers give **4.36×** over the sequential
    baseline's 141.16 ms.
  * **One worker and a pool is slower than no pool.** 205.60 ms against the
    sequential path's 141.16 ms at tile size 256: the pool's machinery
    costs more than it saves when there is nothing to parallelise.
  * **The photograph.** The corpus's native 16K image (15360 × 8640 RGB,
    398,131,200 canonical bytes) is converted to canonical pixels **once**
    by `benchmarks/corpus/canonical_cache.py`, cached under a key derived
    from the source file's SHA-256, with a sidecar that records both
    digests so a stale cache is detected rather than trusted. The
    benchmark reads that file tile by tile through a `PixelSource`, so the
    image is never in memory.
  * **The matrix.** The tile size (256, 512, 1024) and the worker count
    (1, 2, 4, 8) are both swept, ten runs per case, against the sequential
    decoder measured in the same run.
  * **The percentiles are defined, not assumed.** The median is the middle
    sample, or the mean of the two middle ones when the count is even; the
    p95 is the **nearest-rank** percentile; the standard deviation is the
    sample one. The rules are written into the raw JSON beside the numbers
    and pinned by unit tests.
  * **The bytes are checked.** Every encode records the SHA-256 of the file
    it wrote. A configuration that changed a byte would be reported as a
    defect, not as a faster result.
  * **The tables are generated, not transcribed.**
    `benchmarks/render_bench_tables.py` renders the report's tables
    straight from the raw JSON, and `check_bench()` in
    `benchmarks/verify_reports.py` checks that every figure the report
    contains is the figure the data holds.
  * **The repetition rule is now the repository's rule.** `docs/BENCHMARKS.md`
    §Method states it, with the reason: the same one-worker 16K encode
    measured 176,812 ms in one run and 128,963 ms in the next, and a
    single run could not tell those two apart.

* **The command line's `encode` streams and parallelises.** `vpng encode`
  used to build the whole `.vpng` file in memory and then write it; it now
  uses the streaming writer, so the output is written straight to disk
  through a temporary name that is renamed over the target. `--workers N`
  exposes the parallel encoder (default: the logical CPU count). **The
  worker count changes the speed and not the bytes**, and three new
  integration tests in `crates/vpng-cli/tests/encode_workers.rs` check it
  from the command line's own side.

* **The parallel encoder and the streaming writer (`crates/vpng-encoder`).**
  The encoder now scales, and the streaming writer is a library feature
  rather than a benchmark helper.
  * **The encoder's per-tile work is independent.** `select_payload` sees
    only the tile's own pixels and geometry, so the same tile always
    produces the same payload. That is what makes parallelism safe.
  * **One worker and N workers produce the same file, byte for byte.**
    The serial encoder, the parallel encoder and the streaming writer all
    build the container through **one** assembly function, so they cannot
    disagree about offsets, checksums or the header. The 16K benchmark
    records the SHA-256 of every file it writes: all four worker counts
    produced the same digest.
  * **A pixel source, not a whole image.** The new `PixelSource` trait
    lets a caller hand the encoder one tile at a time, so a 16K image can
    be encoded without ever being in memory.
  * **The streaming writer is parallel.** The workers encode chunks of
    tiles and the coordinator consumes them in tile order, so the payloads
    are laid out exactly as the serial encoder lays them out. A bounded
    window of chunks in flight keeps the memory bound, whatever the image
    size.
  * **The file is written atomically.** The streaming writer writes to a
    temporary name in the target's own directory and renames it over the
    target, so a reader never sees a half-written image and a failed
    encode leaves no partial file behind.
  * 6 new integration tests over 144 encode cases: every pixel format ×
    six sizes × three tile geometries, comparing the batch encoder, the
    parallel encoder and the streamed file byte for byte; the worker count
    and the chunk size varied from 1 to 16; a generated source that never
    holds an image; and a size-limit failure and a failing source, both of
    which must leave no file behind.

* **The parallel-decode coverage matrix.**
  `crates/vpng-view/tests/parallel_matrix.rs` checks **144 cases** — all
  eight v0.1 pixel formats × six representative sizes × three tile
  geometries (16, 64, 256) — and asserts that every tile the pool produces
  is byte-identical to the sequential decoder's. The promise the engine
  rests on is now checked across the whole format, not just one file.

* **P5 is closed — the 16K acceptance case ran.** The raw data is
  `benchmarks/p5_view_results.json` (16K) and
  `benchmarks/p5_view_results_small.json` (2K), and the report is
  `docs/P5_REPORT.md`, with every quoted figure checked against the raw
  JSON by `benchmarks/verify_reports.py`.
  * **A 16384 × 16384 RGBA8 image** — 1 GiB of canonical pixels, a
    140,431,266-byte file with 4,096 tiles — was written, opened, panned
    across, edited and saved, on this host, in one process. It did not
    have to be reported BLOCKED.
  * **The first viewport takes 63.56 ms**, against **118.96 ms** for the
    sequential baseline measured in the same run: **1.87× faster**.
  * **Only the viewport's tiles are decoded**: 16 of 4,096, counted. The
    parallel pixels are byte-identical to the sequential decoder's for all
    16 of them.
  * **There is no read amplification.** The pool's workers read 540,792
    bytes of payload; the header and the directory are read once; the
    baseline reads 704,700 bytes in total — exactly 163,908 + 540,792.
  * **Memory stays inside the target**: the whole run peaks at 47.9 MB of
    resident set and 43.8 MB of heap, against the 64 MB target.
  * **A pan costs 26.53 ms mean** with prefetch at radius 2, against
    34.66 ms without it. The cancellation rate is 0.6222; one decoded
    result was discarded because the user panned away, and **83 were kept**
    because the new viewport wanted the same tiles.
  * **Prefetch is measured, not promised.** The radius sweep shows it
    helping at 16K (best radius 2, a 17.1% reduction in panned latency)
    and **not** helping at 2K (best radius 0). Both runs are kept.
  * **The P4 property is unchanged**: a 64×64 edit rewrites 1 tile of
    4,096, the other 4,095 payloads keep their bytes exactly, and the
    pixels read back exactly.

* **The predictive prefetch and the viewport plan (`crates/vpng-view`).**
  The part that decides *which* tiles to ask for, not just in what order.
  * **The viewport.** A rectangle of the image, the tiles it covers, and
    their distance from its centre. The nearest tile is decoded first.
  * **The prefetch ring.** The tiles just outside the visible rectangle
    are the ones that come next if the user pans a little. The ring's
    width is a parameter, not a constant: the benchmark measures radii 0
    to 3 and reports them all, including the one that lost.
  * **The direction of travel is preferred.** When the user pans right,
    the tiles to the right are wanted before the tiles to the left. The
    preference is folded into the distance the scheduler sorts by, so it
    is one number and not a second ordering rule that could disagree with
    the first.
  * **The ring is clipped to the image**, and a prefetch is never allowed
    to push a visible tile out of the bounded queue.
  * **A tile that is being decoded for an old viewport is kept when the
    new viewport wants it too.** The pixels do not depend on the viewport,
    so a result that lands after a pan is kept rather than thrown away —
    and the keep is counted separately from the discard.
  * **The visible demand is counted apart from the speculative one.** A
    prefetch that misses is a guess that has not come true, not a cache
    failure, so the two hit rates are reported apart.
  * 13 new tests, including the ring's exact membership, the clipping,
    the direction preference and the queue's behaviour under pressure.

* **The P5 acceptance benchmark (`vpng-view-bench`).** One run produces
  one report: the file is written (or reused) and the four scenarios run
  in the same process, so every figure describes the same machine and the
  same file.
  * **The first viewport**, against the sequential baseline measured in
    the same run.
  * **The pan**: the panned latency, the cancellation rate, and the
    results that were discarded or recovered.
  * **The prefetch**, as a sweep over the ring's radius, with the hit
    rates for the visible and the speculative demand apart.
  * **The localized edit**: that saving still rewrites only the tiles it
    touches, that the untouched payloads are byte-identical, and that the
    pixels come back exactly.
  * **The memory figures say what they include**, next to every number.
  * The pixels the pool produced are compared, byte for byte, with the
    pixels the ordinary decoder produces for the same tiles.

* **The parallel worker pool (`crates/vpng-view`).** The parallel engine:
  a fixed set of worker threads that take requests from the scheduler,
  decode them, and publish the results into the bounded cache.
  * **The lock is never held across a decode.** The workers share one
    mutex and hold it only while they take a request or publish a result,
    so the decode — the expensive part — happens with the lock released.
  * **One decode per tile.** A tile that is being decoded is *in flight*,
    and a request for it does not start a second decode; the caller is
    told that the work is already under way.
  * **A cached tile is not decoded again.** The request path looks the
    tile up first and counts the hit or the miss, so the cache hit rate
    measures how much of the demand the cache satisfied.
  * **A stale result is dropped, not shown.** The generation handle is
    given to every decoder, so a worker can see that the viewport moved
    while its tile was being decoded; the pixels are thrown away and
    counted separately from a failure.
  * **A failed decode does not stop the pool.** The failure is recorded
    against the tile, the worker carries on, and the caller is told when
    it waits for that tile.
  * **The pixels are the same pixels.** The pool changes *when* a tile is
    decoded, never *what* it decodes: a test decodes a 512×384 RGBA8 file
    through four workers and through the sequential decoder, and asserts
    that all 48 tiles are byte-identical.
  * 8 new tests, including the byte-identity check, the in-flight
    deduplication, the cache-hit path, the discarded-stale path and the
    surviving-failure path.

* **A shared tile cache (`crates/vpng-cache`).** The bounded, byte-budgeted
  LRU cache moves out of `vpng-editing` into a crate of its own, because
  two engines need it: the editor holds the tiles a session has touched,
  and the viewer holds the tiles a viewport has shown. Both must be able
  to state exactly how much decoded memory they hold, and there is one
  implementation of the bound. `vpng_editing::TileCache` still resolves
  to the same type, so no caller changes.

* **The feature backlog and the release plan.** The capabilities that were
  proposed for the project — a preview pyramid, parallel decoding, a
  streaming reader, Python bindings, adaptive tiling, a graphical editor,
  a WebAssembly package, HTTP Range streaming, layers and masks, and GPU
  texture blocks — are now attached to phases in `docs/ROADMAP_STATUS.md`
  §2.2 and in the master roadmap's new §21, with the state of each and
  what each one needs. The separate feature list is gone; its contents
  live in the roadmap, where they can be tracked.
  * **A release plan from v0.2 to v0.23** (`docs/ROADMAP_STATUS.md` §9),
    in which **compression work appears in every release**: a codec, a
    measurement, a tuning pass, or a format revision that opens a new
    compression axis. Compression is a standing line of work rather than
    one project deferred to the end.
  * A release version is a product version: the container's own version
    (header bytes 8–9) moves only when the layout changes, and the plan
    says which releases move it.
  * No dates are promised and no version is claimed. Every row is a plan;
    the only versions that are more than a plan are the ones in the phase
    gates, and those say what was executed.
  * The invariants hold in every row: exact lossless reconstruction,
    deterministic output, compatibility with the existing golden fixtures,
    no silent binary-layout change, and no lossy fallback.

* **Streaming reader (`crates/vpng-source`).** The file half of the editing
  engine's `FileSource`, extracted into its own crate so that the
  interactive viewer and the editor share one implementation of the rules
  that decide which files are acceptable. `TileReader` opens a file, parses
  the container once, and reads a tile's payload only when one is asked for.
  * **It counts what it reads.** `bytes_read` and `payloads_read` report the
    I/O volume exactly, so a benchmark can state it rather than estimate it.
  * **The metadata is separate from the handle.** `ContainerMeta` is what
    the header and the directory say: immutable, cheap to share. `fork()`
    opens a second handle on the same file with the metadata already parsed
    — the arrangement a parallel decoder needs, because a file position is
    mutable state and a shared position would serialise the workers.
  * `FileSource` now delegates to it, so there is one implementation of the
    file-access rules rather than two. The editing engine's behaviour is
    unchanged: its 51 tests pass untouched.
  * 11 tests, including one that runs the reader and the in-memory decoder
    over 700 truncations and single-byte corruptions and asserts that they
    accept and reject exactly the same files.
* **P5 design (`docs/P5_DESIGN.md`).** The design for the streaming and
  parallel decoding milestone, written before any of the code: the seven
  components, the acceptance test as measurable criteria, and the open
  decisions that had to be answered first.

* **Interactive scheduler (`crates/vpng-view`).** The bounded request queue
  that decides *which* tiles to decode and *when*.
  * **Priority.** A request is dispatched in the order `(class, distance,
    sequence)`: a visible tile before a prefetched one, a prefetched one
    before a background one; then the nearer tile; then the arrival order,
    so nothing is starved.
  * **A bounded queue.** The queue holds at most *N* requests. A full queue
    refuses a prefetch or background request and does not block the caller.
    A *visible* request is never refused: it displaces the least urgent
    request in the queue, because the tile the user is looking at matters
    more than the tile we guessed they might want.
  * **Cancellation by viewport generation.** Every request carries the
    viewport generation it was issued under. A pan advances the generation
    and every queued request from an older generation is dropped before
    anything else is dispatched. A request already running is allowed to
    finish — stopping a decode half-way would throw the work away — but its
    result is discarded if its generation is stale, and that is counted
    separately from a failure.
  * **One request per tile.** A tile that is already queued is not queued
    again; if the new request is more urgent, the queued one is *promoted*
    rather than duplicated.
  * **Counters, not estimates.** Every submission, promotion, refusal,
    dispatch, cancellation and discarded result is counted, so a report can
    state what happened.
  * 17 tests, including the ordering and cancellation rules checked
    exactly.

### Fixed

* **The file-backed source now validates a file exactly the way the
  in-memory decoder does.** Two checks were missing from `FileSource`:
  * a file too short to hold a header was reported as an **I/O error**
    (a short read) rather than as a corrupt file — the in-memory decoder
    says "corrupt file", and the two now agree;
  * the directory was not checked against the file's length, and a tile's
    payload was not checked to begin after the end of the directory. Both
    are the decoder's rules, and both are now the file-backed source's too.
  * The rule that made this visible is pinned by a test: the reader and the
    in-memory decoder are run over 700 truncations and single-byte
    corruptions of the same file and must agree on every one of them.

* **`Header::parse` accepted a header that `Header::to_bytes` refused to
  serialize.** The parser did not check the image dimensions, while
  `Header::new` and `Header::to_bytes` both required them to be non-zero.
  The parser and the serializer therefore disagreed about the same bytes: one
  implementation would accept a file the other refused to write.
  * Found by the **coverage-guided** fuzzer in CI on Linux (2026-11-18, run
    44266490454) after 2,328 executions of the `header` target. The input is a
    74-byte header that is well formed in every respect except that its width
    is zero — correct magic, version, header size and channel count, a valid
    CRC32C, and zeroed reserved bytes.
  * The full-file path was never affected: `Document::parse` called
    `header.dimensions()` itself and rejected the file, which is why the
    defect survived the entire test suite and only the fuzzer reached it.
  * **The fix:** `Header::parse` now applies the same non-zero dimension rule
    as the other two entry points, with the check placed *after* every other
    header rule so that the error a caller sees for a malformed file does not
    change — a zero dimension is still `InvalidArgument` with the message
    "image dimensions must be non-zero", exactly as it was through
    `Document::parse`.
  * **The binary format is unchanged.** No byte, offset, size or version
    moved, and the golden fixtures are byte-identical.
  * **The regression is pinned:** the exact crash artifact libFuzzer wrote
    (`crash-32233a96f431ba8ee472ae5d927905192ef267aa`) is stored as a
    literal in `crates/vpng-fuzzing/tests/fuzz_regressions.rs` and replayed on
    every `cargo test` run.
  * **The fix is re-verified by the fuzzer.** The CI run after the fix
    (44274770109) executed all seven coverage-guided targets again —
    78,850,985 executions, 0 findings — with the `header` target's coverage
    up from 130 to 184 edges, the extra branch being the new dimension
    check.

### Added

* **Editing engine (`crates/vpng-editing`).** A tile-backed editing session
  with region reads and writes, dirty-tile tracking, transactions, and
  tile-level undo/redo. The engine is generic over a `TileSource` trait, so
  the same code serves a file already in memory (`DocumentSource`) and a file
  far too large to hold in memory (`FileSource`, which reads one payload at a
  time). A 64×64 edit on a 2048×2048 image decodes **1** tile of 64 and
  dirties 1; on a 16384×16384 image it still decodes **1** tile of 1,024.
  * A **bounded byte-budget LRU cache** of decoded tiles. A tile larger than
    the whole budget is rejected rather than cached, so the budget stays
    honest.
  * A **bounded byte-budget undo history**. The peak is recorded after
    trimming, so the reported peak is the high-water mark of the bytes the
    history actually retains and can never exceed the budget. An undo that
    restores a tile to the pixels the file stores marks it clean again, so
    it is not rewritten by the next save.
  * A **copy-on-write save**: the dirty tiles are re-encoded through the
    encoder's own `select_payload`, every other payload is copied **byte for
    byte**, and the new file replaces the old one through a temporary file
    and a single atomic rename. The original file is never written to.
  * **Fault injection** (`FailurePoint::AfterBytes`, `FailurePoint::BeforeRename`)
    so the failure path is tested rather than asserted. Both leave the
    original byte-identical and leave no partial file behind.
  * 51 unit tests. The binary format is unchanged.
  * **A successful commit is a new baseline.** Undo and redo cannot cross a
    commit: both history stacks are cleared, the edit counter restarts, the
    open transaction ends, and the dirty set is reconciled with the committed
    file. A *failed* commit changes nothing — the dirty tiles, the history,
    and the transaction all survive, so the user can retry or undo.
  * A rewritten tile's directory entry carries `predictor_id: 0` and
    `flags: 0` — the metadata describes the payload that was just written,
    not the tile it replaced. An untouched tile keeps its own metadata.
* **Research-only codecs (`crates/vpng-codecs/src/experimental/`).** Four
  context-adaptive residual models (CARC A–D, ids `0x80..=0x83`), a byte-level
  LZ77 dictionary codec (`0x90`), and three hybrids (`0x91..=0x93`). Each has
  an independent encoder and decoder, a documented bitstream, a pinned golden
  hex vector, exact reconstruction, bounded memory, no cross-tile dependency,
  8- and 16-bit support, malformed-stream handling, and integer-only
  arithmetic in the bitstream path. 52 new tests.
  * **The research ids cannot reach a file.** They are not in
    `IMPLEMENTED_CODEC_IDS`, `get_codec` cannot resolve them, the encoder
    cannot select them, and the decoder rejects a file that names one with
    `ErrorKind::UnsupportedFeature`. The stable ids 0–3 are unchanged and
    4–6 remain reserved.
* **Fuzzing (`crates/vpng-fuzzing`, `fuzz/`).** Seven targets — header,
  directory, container, tile payload, experimental codec, region, editing —
  with two drivers over the same logic: a deterministic mutation harness that
  runs on the stable toolchain and in CI, and `cargo-fuzz` wrappers for
  coverage-guided fuzzing on nightly.
* **Memory validation (`crates/vpng-validation`, `benchmarks/memory/`).** A
  tracking global allocator and a Windows `K32GetProcessMemoryInfo` counter
  give two independent memory figures — the allocator's own high-water mark
  and the operating system's resident-set figures. A tile-by-tile streaming
  writer produces an ordinary VPNG file without ever holding the whole
  image, and a driver runs one validation case per process.
* **CI (`.github/workflows/ci.yml`).** Nine jobs: `fmt`, `clippy`, `build`,
  `test`, `golden` (the format's fixtures on their own), `fuzz-smoke`,
  `fuzz` (nightly, coverage-guided), `release`, `python`.
* **Research harness (`crates/vpng-research`, `benchmarks/corpus/run_research.py`).**
  Eleven codecs measured on identical pixels and identical tile layouts,
  with the baselines taken from a shared module so the two corpus runners
  cannot drift. Raw results in `benchmarks/p3_5_raw_results.json` (per-image,
  per-tile, per-category, per-resolution, aggregate and worst-case) and
  `benchmarks/p3_5_comparison.csv`.
* Four milestone reports: `docs/P3_5_COMPRESSION_RESEARCH_REPORT.md`,
  `docs/P4_EDITING_ENGINE_REPORT.md`, `docs/P4_MEMORY_VALIDATION.md`,
  `docs/P4_COMPATIBILITY_REPORT.md`.
* **The `default_codec` wording is resolved.** Header byte 27 is
  **informational**: a reader must not use it to decode a tile and must not
  reject a file because of its value; the per-tile directory entry's `codec
  id` is the authority (ADR-0003). Specification §2.3 states it, §4.1 marks
  the directory entry as authoritative, and §5 repeats it. The byte, its
  offset, its size and the format version are unchanged, so no conforming
  file changes meaning and the golden fixtures are untouched. 7 new
  conformance tests in `crates/vpng-testkit/tests/default_codec.rs`.

### Measured

* **16K memory matrix: 12 PASS, 0 FAIL, 0 BLOCKED.** 4096², 8192² and
  16384² × {gray8, rgb8, rgba8, rgba16}. The 16384×16384 RGBA16 case — 2 GiB
  of canonical pixels — writes with an 8.8 MB heap peak and edits with an
  11.7 MB resident-set growth and a 12.5 MB heap peak, against the 64 MB
  target. The memory figures are flat across the resolutions: the engine's
  memory is a function of the tile size and the budgets, not of the image's
  size.
* **Editing locality.** A 64×64 edit decodes 1 tile of 64, dirties 1
  (1.5625% of the image), reuses 63 of 64 payloads byte for byte, and commits
  in 47.6 ms.
* **Compression research.** Across the whole 9-image corpus (416,522,696
  canonical bytes): WebP lossless 0.2575×, **CARC C 0.3062×**, the stable
  predictor codec 0.3205×, PNG 0.3445×, the predictor→LZ77→entropy hybrid
  0.4350×, LZ77 0.8009×, Raw 1.0002×. On the 15360×8640 photograph, CARC C
  reaches 0.2995× against PNG's 0.3384× and WebP's 0.2549×.
* **Fuzzing: 200,000 iterations × 7 targets = 1,400,000 executions, 0
  findings**, 3.9 s in the release profile. The *deterministic* harness found
  no library defect; the *coverage-guided* fuzzer, run in CI on Linux, found
  one (see **Fixed** below).
* The test suite is **368 passed / 0 failed** (was 181).

### Known limitations

* **No experimental codec is promoted.** CARC C is the strongest research
  result, but it is 18.9% worse than WebP lossless across the corpus, it
  expands incompressible data, and a promotion needs a specification update,
  a version bump, a migration policy and compatibility tests. The stable set
  stays at ids 0–3.
* The compression study is a **9-image seed**, not the 750-image corpus, so
  no distribution, per-resolution, or 10 MB success claim follows from it.
* **JPEG XL lossless is now a measured baseline** (`cjxl` v0.13.0, `-d 0`,
  verified lossless by a `cjxl`→`djxl` round trip): 91,196,450 bytes,
  0.2189× across the corpus, 85,982,362 bytes (0.2160×) on the 15360×8640
  photograph. It is the strongest of the three baselines — 15.0% smaller
  than WebP lossless — and CARC C is 39.8% worse than it. No encoder-setting
  sweep was run, so no claim is made that `cjxl -d 0` is the best JPEG XL
  can do.
* **The WebP baseline on the 15360×8640 photograph needed a second attempt.**
  Pillow's encoder failed once with `can't allocate picture frame` under host
  memory pressure and completed on a repeat, giving 0.2549×. The figure is
  measured, but a baseline that fails intermittently is not one a product
  should depend on.
* **Coverage-guided fuzzing cannot run on this host.** `cargo-fuzz` v0.13.2
  installs and runs, the pinned `nightly-2026-10-01` toolchain builds, and
  the seven targets are now correctly wired (`cargo fuzz list` sees all
  seven); three *platform* limits stop the local run on
  `x86_64-pc-windows-gnu`: the AddressSanitizer runtime is not supported
  for that target, LLVM's libFuzzer Windows build is MSVC-only, and rustc's
  sanitizer-coverage module is ELF-only (it emits the `__sancov_cntrs` /
  `__sancov_pcs` sections and the weak `__start___sancov_*` anchors on ELF
  targets only, so a COFF target cannot link the fuzz target at all). The
  coverage-guided fuzzer **did** run in CI on Linux — 132,429,711 executions
  across the seven targets, 0 findings in the first run (44264162940), a
  second run (44266490454) that found the header defect recorded under
  **Fixed** above, and a third run (44274770109) that re-verified the fix
  (78,850,985 executions, 0 findings). The deterministic harness — 200,000
  iterations × 7 targets = 1,400,000 executions — is the local evidence.
* **No graphical editor.** The engine is a library and a benchmark.
* The editing history is in memory only, temporary files left by a killed
  process are not swept, and there is no file locking.
* The spec's wording for header byte 27 (`default_codec`) said "codec used
  for all tiles in v0.1", while the implementation treats it as informational
  and the per-tile directory entry as authoritative. **Resolved on
  2026-11-18**: specification §2.3 now states that the byte is
  informational, §4.1 marks the directory entry's `codec id` as
  authoritative, and §5 repeats it. The byte, its offset, its size and the
  format version are unchanged, so no file changes meaning; the behaviour is
  pinned by `crates/vpng-testkit/tests/default_codec.rs`.
* VPNG still loses to PNG, WebP lossless and JPEG XL lossless on photographs and
  paintings: the dictionary (id 4), predictor+dictionary (id 5), and
  context-adaptive entropy (id 6) codecs do not exist yet.
* The predictor payload is **not canonical**: any well-formed stream decodes to
  the same pixels, and the encoder's particular per-row choice is pinned by
  golden vectors rather than by the format.
* Predictor selection is greedy per row, and encoding costs roughly two to
  three times what the same coder would cost without the per-row measurement.

### Added (previous)

* **Spatial predictor + entropy codec (id 3, ADR-0011).** A tile is stored as a
  per-row predictor plan plus an adaptive arithmetic-coded list of signed
  residuals. Seven predictors on whole samples — none, left, up, upper-left,
  average, Paeth, and gradient (LOCO-I) — with out-of-tile neighbours treated
  as `0`, so prediction resets at every tile boundary and no tile depends on
  another. Residuals are `(sample - prediction) mod 2^(8B)` folded to
  `[-2^(8B-1), 2^(8B-1) - 1]`, so reconstruction is exact and no floating
  point is involved. The entropy coder is an adaptive binary arithmetic coder
  (11-bit probabilities, shift-5 adaptation) whose contexts are per sign and
  per magnitude plane. The encoder picks a predictor **per row** by measuring
  each candidate with the live coder's probability model, using a baked-in
  fixed-point `log2` table so the choice is identical on every host. Byte-level
  golden vectors, a whole-file golden fixture, and conformance/property tests,
  with `--codec predictor` added to the CLI.
* ADR-0011 (`docs/decisions/ADR-0011-predictor-entropy.md`) records the
  predictor set, the residual folding, the entropy coder, and the per-row
  selection.
* The seed-corpus runner now measures the predictor codec alongside Raw,
  Palette, and Run-length, and the large-image stress harness now encodes a
  forced-predictor file next to the Raw and auto files.
* The bounded mutation sweep now covers a Predictor file as well, so every
  implemented codec is swept for truncations, single-byte substitutions, and
  seeded multi-byte corruptions.

### Measured (previous)

Re-measured on the 9-image seed corpus (2026-11-15, same host and corpus as
the 2026-11-14 run; see `docs/BENCHMARKS.md` for the full table):

* The large photo (15360×8640 rgb8) drops from **0.774×** to **0.312×** of its
  canonical bytes, and the cat photograph from 0.830× to 0.509×.
* Auto selection was again no worse than the best forced codec on all 9
  images, and it now beats forced predictor on the large photo by 2,588 bytes
  by mixing in Palette for two tiles.
* The noise image still falls back to Raw at 1.000× even though a forced
  predictor run expands to 1.016×.

### Known limitations (previous)

* VPNG still loses to PNG and WebP lossless on photographs and paintings: the
  dictionary (id 4), predictor+dictionary (id 5), and context-adaptive entropy
  (id 6) codecs do not exist yet.
* The predictor payload is **not canonical**: any well-formed stream decodes to
  the same pixels, and the encoder's particular per-row choice is pinned by
  golden vectors rather than by the format.
* Predictor selection is greedy per row, and encoding costs roughly two to
  three times what the same coder would cost without the per-row measurement.

## [0.1.0] — 2026-11-13

First implementation phase. The workspace **builds and passes its tests**:
`cargo build --workspace` exits 0 and `cargo test --workspace` reports
148 passed / 0 failed on `stable-x86_64-pc-windows-gnu` (rustc 1.100.0). See
`docs/ROADMAP_STATUS.md` for the recorded evidence.

### Added

* Format specification `docs/VPNG_SPEC_0_1.md`: 64-byte header, 40-byte tile
  directory entries, payload layout, invariants, limits, exit codes.
* Architecture decision records ADR-0001 through ADR-0010 resolving the roadmap's
  open questions (header checksum coverage, directory layout, directory
  authority, codec ids, grid derivation, size units, CRC32C, 16-bit support,
  palette payload layout, run-length token format).
* `docs/REPOSITORY_AUDIT.md`, `docs/ROADMAP_STATUS.md`, `docs/THREAT_MODEL.md`,
  `docs/BITSTREAM.md`, `docs/CODEC_DESIGN.md`, `docs/EDITING_MODEL.md`,
  `docs/MEMORY_MODEL.md`, `docs/BENCHMARKS.md`.
* Rust workspace with eight crates:
  * `vpng-core` — pixel formats, geometry, checked arithmetic, errors, limits.
  * `vpng-container` — header, directory, CRC32C, SHA-256 canonical digest.
  * `vpng-codecs` — `TileCodec` trait, the Raw, Palette, and Run-length codecs,
    and the per-tile codec registry; ids 3–6 reserved.
  * `vpng-tiling` — `TileGrid`, exact partition, region-to-tile mapping.
  * `vpng-encoder` — canonical pixels to a complete `.vpng` file, per-tile
    smallest-payload codec selection, exact size accounting, `SizeLimitExceeded`.
  * `vpng-decoder` — validated `Document`, full/tile/region decode.
  * `vpng-testkit` — deterministic PRNG and image generators.
  * `vpng-cli` — `encode`, `decode`, `info`, `check`, `verify`, `decode-region`,
    `extract-tile`, with stable exit codes and `--json`.
* **Palette codec (id 1, ADR-0009).** Per-tile palette of exact canonical pixel
  bytes, indices packed MSB-first at the smallest valid width, canonical form
  enforced by the decoder. Falls back to another codec when a tile has more than
  256 distinct colours. Byte-level golden vectors, a whole-file golden fixture,
  and property tests, with `--codec auto|raw|palette` on the CLI.
* **Run-length codec (id 2, ADR-0010).** Runs counted in whole pixels, with
  self-delimiting tokens: literal runs of 1–128 symbols and repeat runs of 2–128,
  no escape bytes. The encoder is deterministic greedy; the decoder accepts any
  well-formed stream and rejects truncated, overrunning, or length-1 repeat
  tokens. Byte-level golden vectors, a whole-file golden fixture, and property
  tests, with `--codec rle` added to the CLI.
* **Per-tile codec selection.** `EncodeOptions` defaults to `CODEC_AUTO`: each
  tile gets the smallest payload any implemented codec offers, ties going to the
  lowest id, with Raw as the guaranteed fallback. `with_codec(id)` forces one
  codec; the concrete choice is recorded in each directory entry.
* Layered integrity: `vpng info` (header + directory, O(1)), `vpng check` (every
  tile payload CRC32C, no pixel decode), `vpng verify` (exact pixels against a
  reference). `info` now prints an explicit note that payloads were not
  verified, in both text and JSON forms.
* Conformance tests: exhaustive round-trip over all v0.1 formats and sizes,
  region-vs-crop equality, tile-vs-image equality, determinism, transparent-RGB
  preservation, malformed-input mutation and truncation sweeps, golden files.
* **Bounded mutation sweep** (`crates/vpng-testkit/tests/mutation_sweep.rs`):
  every truncation and every single-byte substitution (three values per offset)
  on one small Raw, Palette, and Run-length file, plus 2,000 seeded multi-byte
  corruptions per file. Every outcome is a typed error or a valid decode; no
  panic. This is a deterministic sweep, **not** coverage-guided fuzzing.
* **Large-image stress harness** (`benchmarks/stress/run_stress.ps1`) and a
  **seed-corpus runner** (`benchmarks/corpus/seed.py`,
  `benchmarks/corpus/run_corpus.py`). Measured results, with host and corpus
  named, are recorded in `docs/BENCHMARKS.md`.
* `benchmarks/corpus_manifest.json`, now populated with a 9-image seed corpus
  (photographs, digital paintings, procedural, noise, screenshots) carrying
  provenance, license, and canonical SHA-256, and a Python experiment skeleton.

### Known limitations

* Raw, Palette, and Run-length only; the remaining baseline codecs (ids 3–6) are
  reserved. Raw ratio is 1.0 by construction; Palette and Run-length shrink
  friendly tiles, but on the 9-image seed corpus VPNG still loses to PNG and
  WebP lossless on photographs and paintings, because no predictor, dictionary,
  or entropy coder exists yet.
* Uniform fixed-size tiles only.
* No tile cache, no parallel decode, no streaming reader.
* No editing engine, previews, or Python bindings yet.
* `vpng info` reads only the header and directory, so it is **not** an integrity
  check; use `vpng check` to validate a file on its own, and `vpng verify` to
  compare it against the original image.
* The seed corpus is 9 images, **not** the 750-image target, so it supports no
  distribution, per-resolution, or 10 MB success claim.
* 16K is measured only for gray8 (16384×16384) and a 15360×8640 rgb8 photo; the
  16384×16384 RGBA8 case (1 GiB of pixels) was skipped for insufficient free
  physical memory and waits on streaming/allocation work.
* No coverage-guided fuzzing yet; the mutation sweep is deterministic and bounded.
