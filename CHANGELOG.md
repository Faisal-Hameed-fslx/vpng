# Changelog

All notable changes to VPNG are recorded here. The format follows Keep a
Changelog; versions follow semantic versioning, with the format version tracked
separately as major.minor in the header.

## [Unreleased]

### Added

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
* The test suite is **329 passed / 0 failed** (was 181).

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
