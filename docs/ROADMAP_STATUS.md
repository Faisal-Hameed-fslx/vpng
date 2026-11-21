# Roadmap status

**Updated:** 2026-11-18
**Legend:**

* `PASS` — the required test **exists and was executed**, and the observed result
  satisfied the gate. The evidence is recorded below.
* `FAIL` — executed and did not satisfy the gate.
* `BLOCKED` — code exists but could not be executed.
* `NOT STARTED` — no implementation yet.

## 1. Build environment (evidence)

| Item | Observed |
|------|----------|
| `rustc --version` | `rustc 1.100.0 (0ef1bc386 2026-11-09)` |
| `cargo --version` | `cargo 1.100.0 (3acc79c0c 2026-11-05)` |
| Default host | `x86_64-pc-windows-gnu` |
| Active toolchain | overridden by `E:\VPNG\rust-toolchain.toml` |
| `cargo build --workspace` | **exit 0**, `Finished dev profile ... in 3.85s` |
| `cargo test --workspace --all-targets` | **exit 0**, `418 passed; 0 failed` |
| `cargo fmt --all --check` | **exit 0**, no diffs |
| `cargo clippy --workspace --all-targets` | **exit 0**, zero warnings |

No Visual Studio is required: the GNU host ships `rust-lld` and a
`self-contained` linker set.

## 2. Phase gates

> **Two numberings are in this file, and they collide.** This table uses
> the **original** master-roadmap numbering, in which **P5 = preview and
> multiresolution** and **P6 = 16K and memory engineering**. §2.1 below
> uses the **forward-plan** numbering, in which **P5 = streaming and
> parallel decoding** and **P6 = the preview pyramid**. The old numbering
> stays authoritative for work already done; the forward plan renumbers
> the remaining work. Where this table says "P5" it means *preview*, and
> where §2.1 says "P5" it means *streaming and parallel decoding*. The
> forward-plan names are always qualified ("forward P5").

| Phase | Gate summary | Status | Evidence |
|-------|--------------|--------|----------|
| P0 — repository audit and specification | unambiguous field sizes; executable serialization tests; reproducible build env | **PASS** | `vpng-container` 19 tests pass (header is exactly 64 bytes, directory entry exactly 40 bytes, round-trips exact, checksum detects a flipped byte). Build environment recorded in §1 and `docs/REPOSITORY_AUDIT.md`. |
| P1 — minimal valid container | exact round-trip all formats; golden files; tile-only decode; malformed tests | **PASS** | `roundtrip.rs` 7 pass, `golden.rs` 5 pass, `malformed.rs` 12 pass, `vpng-decoder` 8 pass. Structural integrity checking (`vpng check`) is implemented and tested. See §3 for the CLI end-to-end run. |
| P2 — baseline codecs | palette, RLE, predictor, LZ77, entropy | **PARTIAL** | Palette (id 1) implemented with byte-level golden vectors, whole-file golden fixture, and property tests (§3.1, A06); RLE (id 2) implemented with byte-level golden vectors, a whole-file golden fixture, and conformance tests (§3.2, A07); Predictor+entropy (id 3) implemented with seven spatial predictors, per-row measured-cost selection, an adaptive arithmetic coder, byte-level golden vectors, a whole-file golden fixture, and conformance/property tests (§3.3, A08). LZ77 (id 4), predictor+dictionary (id 5), and context-adaptive entropy (id 6) remain reserved and are rejected on decode. Cost-based per-tile selection with a Raw fallback is implemented and tested. |
| P3 — adaptive tiling, region decode | region == crop; coverage validation; bounded cache; parallel decode | **PARTIAL** | Fixed-grid region decode and exact-coverage validation pass (`region_decode_equals_full_decode_crop`). A **bounded byte-budget LRU tile cache** now exists and is tested (`the_cache_evicts_when_the_budget_is_exceeded`; A14). The **parallel decode engine** now exists and is measured: `crates/vpng-view` decodes a 16K viewport in 63.56 ms against the sequential baseline's 118.96 ms, and its tiles are byte-identical to the sequential decoder's (`docs/P5_REPORT.md` §3.1, Q11). The parallel half of this gate is closed by measurement. **Variable-size (adaptive) tiling is now implemented and measured** in the v0.2 format revision: a level's tiles are an exact partition of the level's rectangle, validated by a sweep-line check, and the whole 750-image corpus was encoded both ways on the same canonical pixels — 452,762,594 bytes against the uniform grid's 453,767,460 (−0.22%), with 750 of 750 exact and 0 check failures. Evidence: `docs/P6_P7_REPORT.md`, `benchmarks/p7_adaptive_results.json`. |
| P4 — editing engine | localized edit; payload reuse; crash safety; undo/redo | **PASS** | The engine is implemented in `crates/vpng-editing` (51 tests). A localized edit decodes 1 tile of 64 and modifies 1; 63 of 64 payloads are preserved byte for byte and 1 is rewritten; BEGIN/COMMIT/ABORT, atomic copy-on-write save, and two injected failure points all preserve the original; undo/redo restore the pixels exactly. A successful commit is a **new baseline**: undo and redo cannot cross it, while a failed commit preserves the whole editing state. The design decisions it resolved are ADR-0012 (atomic replacement, §18.9/§18.10) and ADR-0013 (cache eviction, §18.13). Evidence: `docs/P4_EDITING_ENGINE_REPORT.md`, `benchmarks/p4_editing_results.json`. |
| P5 — preview and multiresolution | thumbnail, pyramid | **PARTIAL** | The preview pyramid is **forward P6** (§2.1) and it **ran** (it is not BLOCKED): the corpus's own 15,360 × 8,640 RGB photograph (398,131,200 canonical bytes) was encoded five times with and five times without the pyramid. A derived level is explicitly marked as derived in the directory entry's `flags`, the original level is exact, and the time to the first preview is measured — the whole picture in **197.94 ms** at 1/64 scale against **13062.32 ms** for the full-resolution picture, at a cost of **+32.7% bytes** and **+44.2% encode time**. The format revision is **v0.1 → v0.2** and it did not move the 64-byte header or the 40-byte directory entry. A standalone **thumbnail** is not implemented: the smallest pyramid level is the smallest preview, and the gate's separate thumbnail row is still open. Evidence: `docs/P6_P7_REPORT.md`, `docs/VPNG_SPEC_0_2.md`, `benchmarks/p6_preview_results.json`. |
| P6 — 16K and memory engineering | 16K stress; streaming; budgets | **PARTIAL** | A tile-by-tile streaming writer and a 12-case memory matrix now exist: 4096², 8192² and 16384² × {gray8, rgb8, rgba8, rgba16}, **12 PASS / 0 FAIL / 0 BLOCKED**. The 16384×16384 RGBA16 case (2 GiB of canonical pixels) writes in an 8.8 MB heap peak and edits with a 13.2 MB resident-set growth, against a 64 MB target. A bounded cache and a bounded history are implemented. The parallel engine is now measured on a 16K workload: a full viewing run peaks at 47.9 MB of resident set and 43.8 MB of heap against the 64 MB target (`docs/P5_REPORT.md` §3.5). The **streaming encoder** and the **parallel encoder** are both done: `vpng encode` writes through the streaming writer with `--workers N`, one worker and N workers produce the same file byte for byte (144 encode cases), and the streaming writer holds a 1 GiB image to **19,296,256 B** of resident set. **Still open:** a streaming *input* decoder — `vpng encode` still reads the whole input PNG into memory before it encodes. Evidence: `docs/P4_MEMORY_VALIDATION.md`, `docs/P5_ENCODER_REPORT.md`, `benchmarks/p4_memory_results.json`, `benchmarks/p5_view_results.json`, `benchmarks/p5_encode_results.json`. |
| P7 — compression research | controlled experiments | **PARTIAL** | Eleven codecs were measured on identical pixels and identical tile layouts: the four stable codecs, four context-adaptive residual models (CARC A–D), an LZ77 dictionary codec and three hybrids, with pinned golden vectors for each. Every image was also measured against PNG, WebP lossless and JPEG XL lossless, so every column of the comparison shares one denominator. The results are in `benchmarks/p3_5_raw_results.json` (per-image, per-tile, per-category, per-resolution, aggregate and worst-case) and `benchmarks/p3_5_comparison.csv`. The **750-image corpus now exists** (`benchmarks/corpus_manifest.json`, version 2) and the study has **not** been re-run on it: the eleven-codec comparison is still the nine-image seed, and the research codecs are still unpromoted. Evidence: `docs/P3_5_COMPRESSION_RESEARCH_REPORT.md`, `docs/P5_CORPUS_REPORT.md`. |
| P8 — 10 MB target evaluation | exact accounting; honest failure | **PARTIAL** | Exact size accounting passes (`size_accounting_is_exact`); refusal is honest and typed (`--max-size 1024` → exit 6 reporting the smallest lossless size 240,148). The **750-image corpus now exists and the per-category success distribution is measured**: 748 of 750 images (99.7%) land under the decimal 10,000,000-byte ceiling — photographs 299/300, screenshots 100/100, pixel art 100/100, scans 100/100, textures 149/150 — and both failures are named (the 16K photograph at 124,197,035 bytes, one 2,048 × 2,048 texture at 11,397,940 bytes). Evidence: `docs/P5_CORPUS_REPORT.md`, `benchmarks/p5_corpus_results.json`. |
| P9 — Python API | bindings around Rust | **NOT STARTED** | CLI-driver tooling only. |
| P10 — hardening and v1.0 | fuzzing; freeze | **PARTIAL** | A deterministic mutation harness over 7 targets runs in CI (200,000 iterations × 7 targets = 1,400,000 executions, 0 findings), and the 7 `cargo-fuzz` targets ran **coverage-guided in CI on Linux** (run 44264162940: 132,429,711 executions, 0 findings). A second run (44266490454) **found a real defect** — `Header::parse` accepted a zero-dimension header that `Header::to_bytes` refused to serialize — which is fixed and pinned by a regression test, and a third run (44274770109) re-verified the fix (78,850,985 executions, 0 findings). The coverage-guided fuzzer cannot run on this Windows-GNU host — the toolchain and `cargo-fuzz` install and run, but the AddressSanitizer runtime, LLVM's MSVC-only libFuzzer Windows build and rustc's ELF-only coverage module each stop it there — and the format is not frozen. |

### 2.1 The forward plan (2026-11-18) and how it maps onto the phases above

The phase table in §2 is the record of work as it was planned and delivered.
The forward plan renumbers the remaining work, so this table maps the new
names onto the old ones — the old numbering stays authoritative for what has
already been done.

| Forward plan | Objective | Existing phases it draws on | State |
|---|---|---|---|
| P3.5 + P4 — compression and editing | the two gates just closed | P3.5 (recorded as P7) and P4 | **verified; committed** (`c821f25`) |
| P4.1 — specification and compatibility closure | resolve `default_codec`; verify the Linux fuzzing CI path | P10 | **CLOSED** (2026-11-18): the specification clarification is committed (`b692227`) and the Linux coverage-guided fuzz job ran three times — clean, then a real defect that is fixed and pinned, then a clean re-verification (runs 44264162940, 44266490454, 44274770109) |
| P5 — streaming and parallel decoding | open a 16K image and navigate it without a full-image allocation | P6 (streaming, parallel limits) and the parallel-decode part of P3 | **CLOSED** (2026-11-19): all six stages are done. The 16K RGBA8 acceptance case **ran** (it is not BLOCKED): the first viewport takes 63.56 ms against the sequential baseline's 118.96 ms, the whole run peaks at 47.9 MB of resident set against the 64 MB target, and a 64×64 edit rewrites 1 tile of 4,096. Prefetch is measured, not assumed: it helps at 16K (best radius 2) and does not help at 2K, and both runs are kept. Evidence: `docs/P5_REPORT.md`, `benchmarks/p5_view_results.json`, `benchmarks/p5_view_results_small.json` |
| P5.1 — the parallel and streaming encoder | make the encoder fast enough that every later benchmark is affordable, and prove that making it fast does not change the file | P6 (the encoder's half) and the v0.18/v0.19 release rows | **CLOSED** (2026-11-20): the 16K RGBA8 acceptance case **ran** (it is not BLOCKED). The encoder's per-tile work is independent, so the parallel driver encodes chunks of tiles on worker threads while a coordinator consumes them **in tile order**; the serial, parallel and streaming paths all assemble the container through one function. Eight workers cut the 16K encode from **128962.99 ms to 30174.82 ms (4.27×)**, and **all four worker counts wrote the same file** — one SHA-256, `9cc5a3e266b53553…`. The streaming writer holds the whole run to **19,296,256 B** of resident set on a 1 GiB image, against the 64 MB target. A `PixelSource` trait means the image is never in memory. Evidence: `docs/P5_ENCODER_REPORT.md`, `benchmarks/p5_encode_results.json` |
| P5.2 — the multi-run photographic baseline | make every later comparison trustworthy: a repeated, photographic, matrixed measurement to compare P6 and P7 against | the v0.20 and v0.8 corpus rows, and P6/P7's baselines | **CLOSED** (2026-11-20): the 16K photographic case **ran** (it is not BLOCKED). The corpus's own 15,360 × 8,640 RGB photograph (398,131,200 canonical bytes) was encoded **120 times** — three tile sizes × four worker counts × ten runs — and viewed repeatedly in one process. Every case reports a **median** and a **nearest-rank p95**, against a sequential baseline measured in the same run, with the rules written into the raw JSON and pinned by unit tests. The finding that matters: the run-to-run spread is **18.5% to 52.4% of the median**, so a single run cannot be trusted. Eight workers cut the encode 6.15× (78793.67 ms → 12811.94 ms) and **every worker count wrote the same file** — one SHA-256 per tile size across all forty of its runs. The tile size dominates interactive latency: a pan costs 125.87 ms at 1024 against 21.44 ms at 256. Evidence: `docs/P5_BENCH_REPORT.md`, `benchmarks/p5_bench_results.json` |
| P5.3 — the 750-image corpus | replace the nine-image seed with a real, reproducible corpus, and measure the per-category distribution and the 10 MB success rate on it | P8, and the v0.8/v0.20 corpus rows | **CLOSED** (2026-11-21): the corpus **exists** — 750 images, every category filled (photographs 300, screenshots 100, pixel art 100, scans 100, textures 150), 746 of them real, fetched from five public repositories at pinned commits and recorded by a SHA-256 manifest (`benchmarks/corpus_manifest.json`, version 2). Every image's canonical digest was re-verified against the manifest (**0 mismatches**) and every file passed `vpng check` (**0 failures**). The distribution is measured: `auto` writes **0.415×** the canonical bytes against PNG's **0.379×**, WebP lossless's **0.242×** and JPEG XL lossless's **0.219×**, so **VPNG is behind PNG on every category** — the unfavourable result the corpus was built to find. The 10 MB target succeeds on **748 of 750 images (99.7%)**, with both failures named. Evidence: `docs/P5_CORPUS_REPORT.md`, `benchmarks/p5_corpus_results.json` |
| P6 — multiresolution preview pyramid | fast zooming, progressive presentation | P5 | **CLOSED** (2026-11-21): the pyramid is in the container and measured. The format revision **v0.1 → v0.2** writes the level index in the directory entry's `flags` (the low byte), leaves the upper byte reserved, and keeps the 64-byte header and the 40-byte directory entry exactly where they were. The 16K photographic case **ran** (it is not BLOCKED): the corpus's own 15,360 × 8,640 RGB photograph (398,131,200 canonical bytes) was encoded five times with and five times without the pyramid. The whole picture arrives **66× sooner at 1/64 scale** — 197.94 ms for the smallest level against 13062.32 ms for the full-resolution picture — and the cost is stated: **+32.7% bytes** (164,873,298 against 124,197,035) and **+44.2% encode time** (26809.86 ms against 18590.46 ms). The default path's bytes did not move: the v0.1 file's digest is the P5.2 tile-256 baseline's, `d029a7ebc1e0347cb…`. Evidence: `docs/P6_P7_REPORT.md`, `docs/VPNG_SPEC_0_2.md`, `benchmarks/p6_preview_results.json` |
| P7 — adaptive tiling | variable tile dimensions and spatially adaptive layouts | P3 | **CLOSED** (2026-11-21): the variable-size partition is in the container and measured. A level's tiles are now an **exact partition** of the level's rectangle — every pixel covered once — validated by a sweep-line check before a file is written, with the partition representation shared by the encoder, the decoder, the viewer and the editing engine. The **whole 750-image corpus** was encoded both ways on the same canonical pixels: adaptive tiling is a **compression wash** — 452,762,594 bytes against the uniform grid's 453,767,460 (−0.22%), 330 images smaller, 241 larger, 179 identical — and it **hurts UI screenshots** (69 of 100 grow). What it does deliver is **half the tiles at the median** (4 → 2), a smaller directory and fewer requests to serve a region. All 750 adaptive files decode exactly and pass `vpng check`. Evidence: `docs/P6_P7_REPORT.md`, `docs/decisions/ADR-0014-levels-and-partitions.md`, `benchmarks/p7_adaptive_results.json` |
| P8 — production validation | the 750-image corpus, multiple platforms, 16K workloads | P8 | PARTIAL (the corpus exists and is measured; other platforms and 16K workloads are not) |
| P9 — Python bindings and integration | expose the stable engine to other applications | P9 | NOT STARTED |
| P10 — format freeze and release | compatibility guarantees, security audit, v1.0 | P10 | PARTIAL (the format is not frozen) |
| P11 — graphical viewer and editor | a desktop UI over `vpng-view` and `vpng-editing` | P9 (the binding surface) and P5 | NOT STARTED |
| P12 — WebAssembly package | the decoder, the viewer and the editor in a browser | P9 | NOT STARTED |
| P13 — network streaming | read the header, the directory and the tiles over HTTP Range requests | P5 | NOT STARTED |
| P14 — layers and masks | several named tile grids in one file | P7 (the partition representation) | NOT STARTED — needs a container format revision |
| P15 — GPU-ready payloads | BC7/ASTC tile payloads for game-engine asset pipelines | P2 (the codec table) | NOT STARTED — needs a container format revision |

### 2.2 The feature backlog (the capabilities proposed for the project)

This table is the roadmap's answer to the capabilities proposed in the
project's feature list. Every one of them is now attached to a phase, so
that none of them is a wish that lives outside the plan. The "source"
column records where the proposal came from, so the origin of each item is
not lost.

| Capability | Source of the proposal | Phase | State | What it needs |
|---|---|---|---|---|
| Multiresolution preview pyramid (mipmaps) | already in the roadmap | **P6** | DONE | The pyramid is in the container: more than one grid of tiles, the level a reader should fetch marked in the directory entry's `flags`, and a rule that picks the level. The format revision is **v0.1 → v0.2** — the header and the directory entry keep their layout, the golden fixtures do not move, and the default path still writes v0.1 bytes. Measured on the corpus's own 16K photograph: the whole picture in 197.94 ms at 1/64 scale against 13062.32 ms at full resolution, for +32.7% bytes and +44.2% encode time. Evidence: `docs/P6_P7_REPORT.md`, `docs/VPNG_SPEC_0_2.md`. |
| Parallel tile decoding | already in the roadmap | **P5** | **DONE** | All six stages are done: the streaming reader (`vpng-source`), the bounded scheduler, the parallel worker pool, the predictive prefetch, and the acceptance benchmark. The 16K RGBA8 acceptance case **ran**: the first viewport takes 63.56 ms against the sequential baseline's 118.96 ms, only 16 of 4,096 tiles are decoded, and the whole run peaks at 47.9 MB of resident set against the 64 MB target. A 144-case matrix asserts every parallel tile is byte-identical to the sequential decoder's. Prefetch is measured, not assumed — it helps at 16K and does not help at 2K, and both runs are kept. Evidence: `docs/P5_REPORT.md`, `benchmarks/p5_view_results.json`, `benchmarks/p5_view_results_small.json`. |
| Streaming reader and viewport priority | already in the roadmap | **P5** | DONE for local files | `crates/vpng-source` and the scheduler in `crates/vpng-view`. The proposal also suggested HTTP Range streaming, which is tracked separately as **P13**. |
| Python bindings | already in the roadmap | **P9** | NOT STARTED | Native bindings returning NumPy arrays over the same engine, not a re-implementation of it. |
| Adaptive tiling and advanced codecs | already in the roadmap | **P7** and **P2** | PARTIAL — the tiling is DONE, the codecs are not | **Adaptive tiling is done** in the v0.2 format revision: variable tile sizes, an exact-cover partition validated before a file is written, and the whole 750-image corpus measured both ways on the same canonical pixels (−0.22% bytes, half the tiles at the median, 750 of 750 exact). **The reserved stable codec ids 4–6 are still reserved**, and a research codec is promoted to a stable id only after a specification update, a version bump, a migration policy and compatibility tests, in that order. The research ids `0x80..=0xFF` cannot reach a file; a decoder rejects one with `UnsupportedFeature`. |
| Graphical UI editor | proposed; not previously in the roadmap | **P11** | NOT STARTED | A desktop front end over `vpng-view` and `vpng-editing`. The engines exist; the window does not. No graphical editor exists in this repository today. |
| WebAssembly (WASM) package | proposed; not previously in the roadmap | **P12** | NOT STARTED | A `wasm32` build of the decoder, the viewer and the editing engine. The constraint is the same as everywhere else: the browser must produce the same pixels, and the file format must not change to suit the browser. |
| HTTP Range-request cloud streaming | proposed; not previously in the roadmap | **P13** | NOT STARTED | A reader that fetches the 64-byte header, the directory, and then tile payloads by byte range from an object store. The format already supports it: every tile payload has its own offset and length, so no format change is needed. |
| Multi-layer / mask compositing | proposed; not previously in the roadmap | **P14** | NOT STARTED | Several named tile grids in one file. This **changes the container layout** and needs a format revision: a version bump, a migration policy and compatibility tests, with the v0.1 files still readable unchanged. |
| GPU texture block support (BC7/ASTC) | proposed; not previously in the roadmap | **P15** | NOT STARTED | Optional GPU-ready tile payloads. This **changes the codec table** and needs a format revision on the same terms as P14. It is also the one item on this list that is not lossless in the same sense as the rest — a BC7 payload is a lossy GPU block format — so it must be a separate, explicitly named payload kind and never a silent substitute for a lossless tile. |

Two rules govern every row above:

* **A format-changing item is never a silent change.** P14 and P15 change
  what a file contains. Each requires a documented format revision — a
  version bump, a migration policy, and compatibility tests that prove the
  older files still read — before it may reach a file.
* **An item may not be claimed before it is measured.** Every row stays
  NOT STARTED until it has a test that executes, in the same way the phase
  gates do.

The guiding objective for P5 is **interactive performance**, not raw
throughput: opening an enormous image, navigating it smoothly, editing a
tiny region and saving it safely — all without loading the whole image into
memory.

---

## 3. End-to-end CLI evidence (300x200 RGBA8, tile 64)Fixture: deterministic raw RGBA8 with **non-zero RGB under fully transparent
pixels**, generated by `python` and encoded via `--input-format raw`.

| # | Command | Exit | Observed |
|---|---------|-----:|----------|
| 1 | `vpng encode src.raw out.vpng --input-format raw --width 300 --height 200 --format rgba8 --tile-size 64` | 0 | 300x200 RGBA8, 20 tiles, 240,868 bytes, ratio 0.996x, sha256 `6f415a11…b3f348` |
| 2 | `vpng info out.vpng --json` | 0 | `major 0, minor 1, 300x200, RGBA8, tiles 20, tile 64x64, directory_offset 64` |
| 3 | `vpng verify src.raw out.vpng --input-format raw …` | 0 | `PASS` — decoded digest equals source digest |
| 4 | `vpng decode out.vpng restored.png` | 0 | writes 300x200 RGBA8 PNG |
| 5 | `vpng encode restored.png out2.vpng --tile-size 256` | 0 | 2 tiles, sha256 **identical** to step 1 |
| 6 | `vpng verify restored.png out2.vpng` | 0 | `PASS` |
| 7 | `vpng decode-region out.vpng crop.png --x 100 --y 50 --width 40 --height 30` | 0 | 40x30 region |
| 8 | `vpng extract-tile out.vpng --tile-x 1 --tile-y 0 --output tile.png` | 0 | tile at 64+0, 64x64 |
| 9 | `vpng encode src.raw tiny.vpng … --max-size 1024` | **6** | `size limit exceeded: smallest lossless output was 240148 bytes, limit is 1024 bytes` |
| 10 | `vpng info restored.png` (not a VPNG file) | **4** | `corrupt file: bad magic bytes; not a VPNG file` |
| 11 | `vpng encode src.raw` (missing output) | **2** | `invalid argument: missing output path` |
| 12 | `vpng decode corrupt.vpng` (one payload byte flipped) | **4** | `corrupt file: tile 19 payload checksum mismatch: stored 0x750f7361, computed 0xd8722030` |
| 13 | `vpng check out.vpng` (intact file) | 0 | `OK` — 20 tiles, all payload CRC32C verified |
| 14 | `vpng check corrupt.vpng` (one payload byte flipped) | **4** | `corrupt file: tile 19 payload checksum mismatch: stored 0x750f7361, computed 0xd8722030` |
| 15 | `vpng info corrupt.vpng` (payload corrupted) | 0 | prints the header/directory summary plus the explicit "payloads were NOT verified" note |

**Key result:** the canonical SHA-256 survives
`raw → vpng → png → vpng` unchanged, and Google's transparent-RGB information is
preserved verbatim.

### 3.1 Palette codec end-to-end evidence (2-colour gray8, tile 64)

Fixture: a 128×128 gray8 checkerboard of two values (`0x10`, `0xEF`) in 8-pixel
squares, so each 64×64 tile has exactly two colours — the palette's ideal case.
Encoded via `--input-format raw`.

| # | Command | Exit | Observed |
|---|---------|-----:|----------|
| 16 | `vpng encode src.raw auto.vpng --input-format raw --width 128 --height 128 --format gray8 --tile-size 64` | 0 | auto **picks Palette**: 4 tiles, 2,292 bytes, ratio 7.148x, 0.140 B/px, sha256 `74beb8ad...d10f99b` |
| 17 | `vpng encode src.raw raw.vpng ... --codec raw` | 0 | 4 tiles, 16,612 bytes; the auto file is **0.138x** of Raw |
| 18 | `vpng encode src.raw pal.vpng ... --codec palette` | 0 | identical directory and payloads to the auto file; only the informational header `default_codec` differs (auto writes 0, forced writes 1) |
| 19 | `vpng verify src.raw auto.vpng ...` | 0 | `PASS` — decoded digest equals source digest |
| 20 | `vpng decode auto.vpng restored.png` | 0 | writes 128×128 gray8 PNG |
| 21 | `vpng check auto.vpng` | 0 | `OK` — 4 tiles, 2,064 payload bytes verified with CRC32C |

The directory of the auto file records `codec_id = 1` for all four tiles; the
auto and forced-palette files are byte-identical from offset 64 to end of file.

### 3.2 Run-length codec end-to-end evidence (flat gray8, tile 64)

Fixture: a 128×128 gray8 image with every pixel set to `0x55` — the run-length
codec's ideal case. Encoded via `--input-format raw`.

| # | Command | Exit | Observed |
|---|---------|-----:|----------|
| 22 | `vpng encode src.raw auto.vpng --input-format raw --width 128 --height 128 --format gray8 --tile-size 64` | 0 | auto **picks Run-length**: 4 tiles, 484 bytes, ratio 33.851x, 0.030 B/px, sha256 `14e7013b...b699fe76` |
| 23 | `vpng encode src.raw raw.vpng ... --codec raw` | 0 | 4 tiles, 16,612 bytes; the auto file is **0.029x** of Raw |
| 24 | `vpng encode src.raw pal.vpng ... --codec palette` | 0 | 4 tiles, 2,288 bytes; auto is **0.212x** of Palette |
| 25 | `vpng encode src.raw rle.vpng ... --codec rle` | 0 | 484 bytes; identical to the auto file from offset 64 to end of file |
| 26 | `vpng verify src.raw auto.vpng ...` | 0 | `PASS` — decoded digest equals source digest |
| 27 | `vpng decode auto.vpng restored.png` | 0 | writes 128×128 gray8 PNG |
| 28 | `vpng check auto.vpng` | 0 | `OK` — 4 tiles, 256 payload bytes verified with CRC32C |

The directory records `codec_id = 2` for all four tiles. The auto and
forced-RLE files differ only in the informational header `default_codec` byte
(auto writes 0, forced writes 2); the rest of the file is byte-identical.

### 3.3 Predictor + entropy codec end-to-end evidence (diagonal ramp, gray8, tile 64)

Fixture: a 256×256 gray8 image whose pixel value is `(x*2 + y*3) mod 256` — a
smooth diagonal ramp, so the spatial predictors have something to predict and
the entropy coder has a small, repetitive residual stream to code. Encoded via
`--input-format raw` with the **release** build.

| # | Command | Exit | Observed |
|---|---------|-----:|----------|
| 29 | `vpng encode src.raw auto.vpng --input-format raw --width 256 --height 256 --format gray8 --tile-size 64` | 0 | auto **picks Predictor**: 16 tiles, 5,176 bytes, ratio 12.662x, 0.079 B/px, canonical sha256 `2605a6c1…21075f5c` |
| 30 | `vpng encode src.raw raw.vpng … --codec raw` | 0 | 16 tiles, 66,244 bytes; the auto file is **0.078x** of Raw |
| 31 | `vpng encode src.raw pred.vpng … --codec predictor` | 0 | 16 tiles, 5,176 bytes; **byte-identical to the auto file from offset 64 to end of file** |
| 32 | `vpng encode src.raw rle.vpng … --codec rle` | 0 | 16 tiles, 66,756 bytes; auto is **0.078x** of Run-length |
| 33 | `vpng verify src.raw auto.vpng …` | 0 | `PASS` — decoded digest equals source digest |
| 34 | `vpng decode auto.vpng restored.png` | 0 | writes 256×256 Gray8 PNG |
| 35 | `vpng check auto.vpng` | 0 | `OK` — 16 tiles, 4,468 payload bytes verified with CRC32C |
| 36 | `vpng info auto.vpng --json` | 0 | `default_codec 0`, `tile_count 16`, `tile 64x64`, `payloads_verified false` |
| 37 | byte comparison, auto vs forced predictor | — | identical from offset 64; only header byte 27 (`default_codec`: 0 vs 3) differs, exactly as with Palette and RLE |
| 38 | `vpng decode-region auto.vpng crop.png --x 100 --y 50 --width 40 --height 30` | 0 | 40x30 region, sha256 `d28c9e5c…eda4cc3b614` |
| 39 | `vpng extract-tile auto.vpng --tile-x 1 --tile-y 0 --output tile.png` | 0 | tile at 64+0, 64x64 |

The directory records `codec_id = 3` for all 16 tiles. The auto and
forced-predictor files differ only in the informational header `default_codec`
byte (auto writes 0, forced writes 3), the same relationship the Palette and
Run-length phases recorded.

### 3.4 Large-image stress and mutation evidence (2026-11-15, with Predictor)

Driver: `benchmarks/stress/run_stress.ps1` (release build, tile 256, 12 logical
CPUs, 7,916 MB RAM). Full tables are in `docs/BENCHMARKS.md`; the key rows:

| Image | Codec | Output bytes | Encode ms | Verify ms | Peak RSS MB |
|---|---|---:|---:|---:|---:|
| 15360×8640 rgb8 (`1006677.jpg`) | raw | 398,212,868 | 2,205 | 2,077 | 1,142.6 |
| 15360×8640 rgb8 (`1006677.jpg`) | predictor | 124,199,623 (0.312×) | 54,823 | 14,906 | 882.0 |
| 15360×8640 rgb8 (`1006677.jpg`) | auto | 124,197,035 (0.312×) | 56,789 | 14,998 | 882.0 |
| 16384×16384 gray8 | raw | 268,599,364 | 1,512 | 1,090 | 772.0 |
| 16384×16384 gray8 | auto | 8,024,132 (0.030×) | 26,078 | 10,779 | 523.3 |
| 8192×8192 rgba8 | auto | 7,564,524 (0.028×) | 22,520 | 8,017 | 523.6 |

Every row: `verify` exit 0 (exact pixels) and `check` exit 0 (all payload CRCs).
A 512×512 region decode took 416–553 ms. The 16K RGBA8 case was **skipped**, not
failed: it needs roughly 3.4 GiB of live buffers and free physical memory never
exceeded 1,445 MB. The harness records that reason in its JSON report.

The predictor codec changes these numbers decisively. Before it landed (run of
2026-11-14, same host and driver) the photo's auto file was 308,132,486 bytes
(0.774×) and both synthetic cases fell back to Raw for every tile. The cost is
visible in the encode column: every row is measured under all seven predictors
before one is kept, so a predictor encode takes 20–55 s where Raw takes 1.4–2.2
s. Decode and peak RSS are both *better* than the Raw path, because the payload
is smaller.

Mutation evidence: `crates/vpng-testkit/tests/mutation_sweep.rs` runs every
truncation and every single-byte substitution (three values per offset) on one
small Raw, Palette, RLE, and Predictor file, plus 2,000 seeded multi-byte
corruptions per file. All outcomes are typed errors or valid decodes; no panic
occurred. This is a deterministic sweep, **not** coverage-guided fuzzing (see
A22).

### 3.5 Finding: `info` is not an integrity check — resolved by `check`

Step 15 (`info` on a payload-corrupted file) returns **exit 0**. This is
expected rather than a defect: `info` parses only the header and directory and
never reads tile payloads, which is what makes it O(1). To make that explicit,
`info` now prints a footer stating that payloads were **not** verified and
pointing at `vpng check`; in `--json` mode it reports `"payloads_verified":false`.

The gap — "is this file internally intact, with no reference image?" — is now
filled by **`vpng check <file>`**, which CRCs every tile payload, decodes no
pixels, and allocates no pixel buffer. It exits 0 when intact (step 13) and 4 on
the first bad payload CRC (step 14). `decode`/`verify` continue to catch the same
corruption while also doing their own work.

`check` is an addition beyond the roadmap's §12.1 command list, which assigns no
command to the "Integrity checks" capability (roadmap line 110) and defines
`vpng verify` as a comparison against an *original image* (line 1059). `info`
keeps its roadmap-specified meaning as an O(1) metadata command.

Integrity is therefore layered and each command states its own scope:

| Command | Reads | Detects | Cost |
|---------|-------|---------|------|
| `vpng info` | header + directory | structural violations, header/directory checksum | O(1) in tile count — **not** an integrity check |
| `vpng check` | every payload | all of the above **plus** every tile CRC32C | O(file size), no pixel buffer |
| `vpng verify` | candidate + original image | decode-accurate **plus** exact pixel equality against a reference | O(file size) plus a decode |

## 4. Acceptance matrix

| ID | Requirement | Evidence required | Status | Evidence |
|----|-------------|-------------------|--------|----------|
| A01 | Exact RGB8 round-trip | tests + hashes | **PASS** | `gradient_roundtrips_exactly_everywhere` etc. cover rgb8 |
| A02 | Exact RGBA8 round-trip | tests + hashes | **PASS** | same suites, rgba8; CLI §3 sha256 |
| A03 | Exact 16-bit round-trip | tests + hashes | **PASS** | same suites include gray16/rgb16/rgba16/graya16; `golden_gradient_300x180_rgb16_tile64` |
| A04 | Transparent RGB preservation | dedicated test | **PASS** | `transparent_pixel_channels_are_preserved` |
| A05 | Raw fallback | incompressible test | **PASS** | `noise_roundtrips_exactly_everywhere` (uniform random) and `incompressible_noise_falls_back_to_raw` / `auto_selection_falls_back_to_raw_for_noise` |
| A06 | Palette codec | golden vectors | **PASS** | byte-level vectors in `vpng-codecs/src/palette.rs` (1/2/3/16/256/>256 colours); golden fixture `palette_checker_64x64_rgba8_tile32.vpng`; property tests `vpng-testkit/tests/palette.rs` 5 pass; CLI §3.1 |
| A07 | RLE codec | golden vectors | **PASS** | byte-level vectors in `vpng-codecs/src/rle.rs` (literal/repeat boundaries, run splitting, multi-byte symbols, malformed streams); golden fixture `rle_mixed_64x64_gray8_tile64.vpng`; conformance tests `vpng-testkit/tests/rle.rs` 6 pass; CLI §3.2 |
| A08 | Prediction codec | property tests | **PASS** | seven predictors (none/left/up/upper-left/average/Paeth/gradient) forced in turn, reversible modular residual folding, per-row measured-cost selection; byte-level vectors in `vpng-codecs/src/predictor.rs`; golden fixture `predictor_ramp_64x64_gray8_tile64.vpng`; conformance and property tests `vpng-testkit/tests/predictor.rs` 10 pass; CLI §3.3 |
| A09 | Dictionary codec | property tests | **PARTIAL** | A research-only LZ77 dictionary codec exists with 22 tests, a pinned golden hex vector, four matching strategies, malformed-stream handling and 8/16-bit support (`crates/vpng-codecs/src/experimental/lz77.rs`). It is **not** assignable to the stable id 4: the research ids live in `0x80..=0xFF`, `get_codec` cannot reach them, and the decoder rejects a file that names one. Stable id 4 remains reserved. |
| A10 | Entropy coder | golden vectors | **PASS** | The arithmetic coder behind stable codec 3 is implemented with a pinned golden vector and per-row measured predictor selection (A08). A research-only context-adaptive entropy stage with 5 symbol groups exists for the hybrids (`crates/vpng-codecs/src/experimental/entropy.rs`), and CARC A–D add 256 integer probability blocks per tile. Stable id 6 remains reserved. |
| A11 | Tile-only decode | instrumented test | **PASS** | `single_tile_decode_matches_full_image`, `region_does_not_decode_unrelated_tiles` |
| A12 | Region decode | crop equality test | **PASS** | `region_decode_equals_full_decode_crop` (5 rects × 8 formats) |
| A13 | Parallel decode | determinism test | **PASS** | The parallel engine never changes *what* it decodes, only *when*. `crates/vpng-view/tests/parallel_matrix.rs` checks **144 cases** — all eight v0.1 pixel formats × six representative sizes × three tile geometries (16, 64, 256) — and asserts that every tile the pool produces is byte-identical to the sequential decoder's. The unit test `the_pool_produces_the_same_pixels_as_the_sequential_decoder` does the same for a 512 × 384 file with 48 tiles through four workers, and the acceptance benchmark compares the first viewport's 16 tiles again at 16K. The pool also has one decode per tile (`AlreadyInFlight`), and a tile whose decode lands after a pan is kept when the new viewport still wants it (83 such results in the 16K run). Evidence: `docs/P5_REPORT.md` §3.1 and Q11. |
| A14 | Bounded cache | allocation test | **PASS** | `TileCache` is a strict LRU with a **byte** budget (`with_cache_bytes`), not a tile count. `the_cache_evicts_when_the_budget_is_exceeded`; the benchmark's cache scenario fills a 524,288-byte budget exactly with 2 evictions and never exceeds it. A tile larger than the whole budget is rejected rather than cached. |
| A15 | Localized edit | instrumented test | **PASS** | `a_region_read_does_not_decode_unrelated_tiles`, `a_region_write_changes_only_the_tiles_it_intersects`, `the_engine_never_holds_a_full_image_buffer`. Measured: a 64×64 edit on 2048×2048 decodes 1 tile of 64 and dirties 1 (1.5625% of the image); on 16384² it decodes 1 tile of 1,024. |
| A16 | Undo/redo | exact hash tests | **PASS** | `undo_and_redo_restore_the_pixels_exactly`, `undo_makes_a_tile_clean_again`, `undo_with_no_history_reports_that_there_is_nothing_to_do`, `a_new_edit_invalidates_the_redo_chain`. The benchmark confirms `undo_restores_pixels` and `redo_restores_pixels` with a byte-exact region comparison. |
| A17 | Crash-safe commit | fault injection | **PASS** | The save never writes to the original: it writes a temporary file in the same directory, `sync_all`s it, and renames it over the target. `FailurePoint::AfterBytes` and `FailurePoint::BeforeRename` are injected in `a_failed_write_leaves_the_original_file_untouched` and `a_failure_before_the_rename_leaves_the_original_untouched`; in both cases the original is byte-identical, still parses, still passes its integrity check, and no temporary file is left behind. |
| A18 | Preview separation | format and API tests | NOT STARTED | |
| A19 | 16K stress | recorded run | **PASS** | A 12-case matrix — 4096², 8192², 16384² × {gray8, rgb8, rgba8, rgba16} — **12 PASS / 0 FAIL / 0 BLOCKED** (`benchmarks/p4_memory_results.json`). Every case writes the file, verifies it, edits it, commits it and verifies the result. The 16384×16384 RGBA16 case (2,147,483,648 canonical bytes) writes with an 8.83 MB heap peak and edits with an 11.69 MB resident-set growth and a 12.52 MB heap peak, against the 64 MB target. A general 16K **performance** claim is not made; the write times are 2–461 s and are dominated by the predictor codec's arithmetic coding. |
| A20 | 10 MB enforcement | boundary tests | **PASS** | `size_limit_is_reported_not_violated`; CLI step 9 exit 6 |
| A21 | Corruption detection | mutation tests | **PASS** | `malformed.rs` 12 tests; `vpng-decoder` integrity tests; CLI step 12 (`decode`) and step 14 (`check`) exit 4 |
| A22 | Fuzz robustness | fuzz report | **PARTIAL** | `malformed.rs` 12 tests, `mutation_sweep.rs` 4 tests (every truncation and every single-byte substitution, plus 2,000 seeded multi-byte corruptions per file, with no panic), and a deterministic mutation harness over 7 targets: **200,000 iterations × 7 targets = 1,400,000 executions, 0 findings** (`benchmarks/results/fuzz-mutation-report.json`). The 7 `cargo-fuzz` targets ran **coverage-guided in CI on Linux** (run 44264162940, 2026-11-18): **132,429,711 executions across the seven targets, 0 findings**, with AddressSanitizer and libFuzzer's coverage feedback. **A second run found a real defect** (run 44266490454): `Header::parse` accepted a zero-dimension header that `Header::to_bytes` refused to serialize — the parser and the serializer disagreed about the same 74 bytes. It is **fixed** (`Header::parse` now applies the same non-zero dimension rule as `Header::new` and `Header::to_bytes`) and **pinned**: the exact crash artifact is replayed on every test run in `crates/vpng-fuzzing/tests/fuzz_regressions.rs`. A **third run re-verified the fix** (44274770109): all seven targets, **78,850,985 executions, 0 findings**, with the `header` target's coverage up from 130 to 184 edges — the extra branch is the new dimension check. **Coverage-guided fuzzing cannot run on this Windows-GNU host** — the toolchain and `cargo-fuzz` v0.13.2 install and run, and `cargo fuzz list` sees all seven targets, but three platform limits stop the local run on `x86_64-pc-windows-gnu` (ASan is unsupported for the target, LLVM's libFuzzer Windows build is MSVC-only, and rustc's sanitizer-coverage module is ELF-only) — so the local evidence is the deterministic harness only. |
| A23 | Python API | integration tests | NOT STARTED | |
| A24 | v1.0 compatibility | golden corpus | NOT STARTED | |

## 5. Test inventory (executed)

Command: `cargo test --workspace --all-targets -j 2`, release and debug
profiles, `x86_64-pc-windows-gnu`.

| Suite | Tests | Result |
|-------|------:|--------|
| `vpng-cache` unit | 11 | ok |
| `vpng-cli` unit | 11 | ok |
| `vpng-cli` `tests/encode_workers.rs` | 3 | ok |
| `vpng-cli` `tests/v2_features.rs` | 6 | ok |
| `vpng-codecs` unit | 110 | ok |
| `vpng-container` unit | 21 | ok |
| `vpng-core` unit | 19 | ok |
| `vpng-decoder` unit | 14 | ok |
| `vpng-editing` unit | 41 | ok |
| `vpng-encoder` unit | 24 | ok |
| `vpng-encoder` `tests/parallel_identity.rs` | 6 | ok (144 encode cases) |
| `vpng-fuzzing` unit | 0 | ok |
| `vpng-fuzzing` bin `vpng-fuzz-mutate` | 0 | ok |
| `vpng-fuzzing` `tests/fuzz_smoke.rs` | 3 | ok |
| `vpng-fuzzing` `tests/fuzz_regressions.rs` | 3 | ok |
| `vpng-source` unit | 11 | ok |
| `vpng-view` unit | 38 | ok |
| `vpng-view` `tests/parallel_matrix.rs` | 1 | ok (144 cases) |
| `vpng-research` bin | 0 | ok |
| `vpng-testkit` unit | 2 | ok |
| `tests/default_codec.rs` | 7 | ok |
| `tests/golden.rs` | 5 | ok |
| `tests/malformed.rs` | 12 | ok |
| `tests/mutation_sweep.rs` | 4 | ok |
| `tests/palette.rs` | 5 | ok |
| `tests/predictor.rs` | 10 | ok |
| `tests/rle.rs` | 6 | ok |
| `tests/roundtrip.rs` | 7 | ok |
| `tests/v2_formats.rs` | 10 | ok (the v0.2 format revision) |
| `vpng-tiling` unit | 18 | ok |
| `vpng-validation` unit | 8 | ok |
| `vpng-validation` `tests/mem_tracking.rs` | 2 | ok |
| `vpng-validation` bins | 0 | ok |
| Doc-tests | 0 | ok |
| **Total** | **418** | **418 passed, 0 failed** |

The 115 new tests are: 52 codec tests (22 LZ77, 14 CARC, 12 hybrids, plus
the registry and entropy-stage tests), 51 editing tests, 3 fuzzing tests,
2 memory-accounting tests and 7 `default_codec` conformance tests.

Since that count was taken, ten more have been added: 6 in
`crates/vpng-validation/src/photo_bench.rs` (the median, the nearest-rank
p95, the sample standard deviation, the order-independence of the summary,
and the raw-file tile reader), 3 in
`crates/vpng-cli/tests/encode_workers.rs` (the worker count does not change
the file; the command line's file reads back; `--workers 0` is rejected),
and 1 in `crates/vpng-validation/tests/mem_tracking.rs`. The allocator's
counter test moved out of the library's test binary into its own
integration-test binary: the counters are process-wide, so the test raced
with every other test in the same process and failed roughly nine times in
ten.

**The v0.2 format revision added 50 more** (368 → 418): 10 in
`crates/vpng-testkit/tests/v2_formats.rs` (the default path is still v0.1;
the pyramid holds the original and the previews; a derived level never
replaces the original; the adaptive partition is an exact cover; an adaptive
region decodes equal to the crop; the worker count and the streaming writer
produce the same v0.2 file; the pyramid round-trips; a v0.1 file claiming a
pyramid is rejected; a reserved flag bit is rejected; the entry flags carry
the level), 6 in `crates/vpng-cli/tests/v2_features.rs` (the CLI's new
surface), and the rest in the crates the revision touched — the decoder's
level derivation, the tiling partition's exact-cover validator, the encoder's
pyramid and adaptive planner, and the editing engine's pyramid commit.

## 6. Defects found and fixed this phase

| # | Symptom | Root cause | Fix |
|---|---------|-----------|-----|
| 1 | `error[E0277]: dyn TileCodec doesn't implement Debug` (2 sites) | `Result::unwrap_err` requires `T: Debug`; the Ok type is `&dyn TileCodec` | used `.err().unwrap()` |
| 2 | `error[E0308]: expected Rect, found Result<Rect, Error>` | missing `.unwrap()` on `Rect::new` in a test | added `.unwrap()` |
| 3 | `region_maps_to_intersecting_tiles_only` failed (`1 != 4`) | **test expectation wrong, not the code**: rect `70..80` lies entirely inside the tile covering `64..128` | moved the rect to `60..70` so it genuinely straddles the boundary |
| 4 | Python `encode(json=True)` emitted `--json True` | booleans were serialized as value options | treated `bool` as a switch flag |
| 5 | `clippy::bool_comparison` warning | `x == false` in a test assertion | `!x` |
| 6 | `clippy::manual_div_ceil` warning | hand-rolled ceiling division | `u64::div_ceil` |
| 7 | `cargo fmt --all --check` reported diffs across 20+ files | source was hand-formatted, never run through rustfmt | applied `cargo fmt --all`; re-check now exits 0 |
| 8 | `error: this file contains an unclosed delimiter` in `vpng-decoder/src/lib.rs` | an edit anchor overlapped the tail of `tile_decode_matches_full_image_span`, dropping its loop body | restored the missing body; build/test/clippy/fmt all green afterwards |
| 9 | `error[E0432]: unresolved import vpng_testkit` in `vpng-codecs` tests | the palette unit test imported `V0_1_FORMATS` from `vpng-testkit`, which `vpng-codecs` must not depend on (dependency cycle) | inlined the eight formats in the test |
| 10 | clippy MSRV warnings: `is_multiple_of` stable since 1.87, `is_none_or` since 1.82 | used std methods newer than the workspace `rust-version` (1.74) | replaced with `% == 0` and `map_or(true, ..)` |
| 11 | `clippy::unnecessary_cast` warning | `x as u32` where `x` was already `u32` in an RLE test fixture | removed the cast |
| 12 | stress harness recorded a blank exit code for successful encodes | `Start-Process -PassThru -RedirectStandardOutput` does not populate `ExitCode` in this PowerShell | switched the harness to `System.Diagnostics.Process` with redirected streams |
| 13 | warm-up golden assertion failed on an in-test fixture | the seed was re-created with the *last* mutation value instead of the original when one offset was hit twice | restore recorded bytes in reverse order |
| 14 | corpus report printed `â€”` instead of an unavailable marker | the Python source literal was double-encoded | replaced it with an ASCII dash; scripts are now ASCII-only |
| 15 | predictor selection picked `none` for a flat tile and `up` for a horizontal ramp | the cost proxy summed each residual's significant-bit count, which cannot see that the adaptive model prices a repeated small residual at nearly zero bits | replaced the proxy with a measurement against the live coder's probability model (`Probe`), using a baked-in fixed-point `log2` table |
| 16 | predictor encoding took 221 ns/sample in release | the selector re-encoded every candidate through the **full** arithmetic encoder, then rolled the state back | split the coder into a `BitSink` trait with an output-producing `Encoder` and a cost-only `Probe`; the probe skips interval arithmetic and byte emission. 221 ns → 81 ns/sample |
| 17 | `cargo clippy --workspace --all-targets` reported 6 warnings in the new codec | `RangeInclusive::contains` hand-rolled, a loop variable used to index a vector, and `extend(drain(..))` where `append` fits | rewrote each site; clippy is warning-free |
| 18 | `auto_selection_records_a_codec_per_tile` failed after the predictor landed | the fixture's alpha channel was a constant `0xFF`, so the predictor could predict alpha exactly and legitimately beat Raw on the "noise" tiles | made every channel of the noise tiles random, so no predictor can predict them and Raw is genuinely the smallest representation |
| 19 | the `History` peak could exceed the byte budget | the peak was recorded **before** the history was trimmed, so an oversized step was counted in the peak even though it was dropped | moved the peak update after the trim; added `the_peak_never_exceeds_the_budget` and `a_step_too_large_for_the_budget_is_dropped_at_once` |
| 20 | `error[E0061]: this method takes 3 arguments but 2 were supplied` on `System::realloc` | the stable `GlobalAlloc` trait's `realloc` takes `(ptr, layout, new_size)` | passed `new_size` |
| 21 | `error[E0425]: cannot find value 'CURRENT' in this scope` in the Windows memory counters | the pseudo-handle constant was named `CURRENT_PROCESS` but referenced as `CURRENT` | renamed the reference |
| 22 | the 16-bit tile pattern wrote its samples one byte at a time | the sample's byte length was taken from the wrong slice, so a 16-bit sample was written as one byte | wrote the two little-endian bytes explicitly for the 16-bit path |
| 23 | `error[E0277]: the trait bound 'String: From<u64>' is not satisfied` | a raw `u64` was passed where `Json::string` expected a `String` | formatted it with `format!` |
| 24 | `error[E0308]: expected 'Option<String>', found '&Option<String>'` in the argument parser | `&args.next()` borrowed the iterator's item instead of consuming it | passed `args.next()` by value |
| 25 | `mem::tests::the_heap_counter_follows_the_allocations` failed in the library's test binary | the tracking allocator is declared in the two *binaries*, so the library's own test binary had no allocator and the counter read zero | declared the tracking allocator in the library's `#[cfg(test)]` module, which is a different binary from the two tools |
| 26 | the fuzz `region` target reported "tile 1 lies outside the image" | the target called `TileGrid::new` with the arguments in the wrong order (`width, height, tile_width, tile_height` instead of `tile_width, tile_height, image_width, image_height`) | corrected the call — a **harness** bug, not a library defect |
| 27 | the fuzz `container` target reported "an accepted file failed its own integrity check" on a corrupted payload | the target asserted that every file the parser accepts also passes its integrity check, but `Document::parse` deliberately does **not** verify payload checksums — that is an O(1) parse | the target now checks the contract the decoder actually makes: an integrity failure is a *corrupt file* error, and a successful decode has the declared dimensions, byte count and crop equality — a **harness** bug, not a library defect |
| 28 | the fuzz `header` target reported "an accepted header did not re-encode to its own bytes" | the target compared the re-encoded header with the **whole** input, but the parser reads only the first 64 bytes and ignores the rest | the comparison is now against the header's own 64 bytes — a **harness** bug |
| 29 | the fuzz smoke test took 166 s in the debug profile | the `editing` target re-encoded its test image on every input, and the `region` target walked up to 4,096² tiles | the image is built once behind a `OnceLock`, and the grid bounds are 512²; the release profile runs 200,000 iterations in 3.9 s |
| 30 | the matched-subset aggregation was empty | the subset filter disqualified an image when a *codec* reported "unsupported", instead of when a *baseline measurement* was missing | the filter now considers only the baselines, so a codec that cannot represent a tile (palette) no longer disqualifies an image, and a baseline tool that is absent from every image disqualifies nothing either |
| 31 | `cargo fuzz list` printed nothing and `cargo fuzz run <target>` failed with "no targets specified in the manifest" | the fuzz crate's `Cargo.toml` had no `[[bin]]` sections, and cargo-fuzz discovers its targets from the manifest, not from the files in `fuzz_targets/` | seven `[[bin]]` sections were added, one per target |
| 32 | `cargo fuzz run <target> --fuzz-dir fuzz/corpus` failed with "could not read the manifest file: …\fuzz\corpus\Cargo.toml" | `--fuzz-dir` is cargo-fuzz's *fuzz project* directory (the one holding the fuzz crate's `Cargo.toml`), not the corpus directory; the corpus lives in `<fuzz-dir>/corpus/<target>` | the runner and the CI job now pass `--fuzz-dir fuzz` |

No defect was found in the container, encoder, or decoder logic itself. The three
`integrity_check_*` tests were added with the `check` command (5 → 8 decoder
tests); the palette phase added the codec, the selection logic, and 23 tests
(99 → 122); the RLE phase added the codec, the registry/selector/CLI wiring, and
22 tests (122 → 144); the evidence batch added the mutation sweep and 4 tests
(144 → 148); the predictor phase added the codec, the registry/selector/CLI
wiring, and 33 tests (148 → 181); the P3.5 + P4 milestone added the research
codecs, the editing engine, the fuzzing harness, the validation tools and 102
tests (181 → 283); the pre-commit correction pass added 6 editing tests
(283 → 289); the `default_codec` specification clarification added 7
conformance tests (289 → 296); the coverage-guided fuzzing fix added 2 header
unit tests and 3 regression tests that replay the fuzzer's crash artifact
(296 → 301); the P5 streaming reader added 11 tests (301 → 312); the P5
scheduler added 17 (312 → 329); the P5 worker pool added 8 (329 → 337); the P5
prefetch policy added 12 viewport tests and 1 scheduler test (337 → 350); the
parallel-decode coverage matrix added 1 integration test over 144 cases
(350 → 351); the parallel-encoder identity suite added 6 integration tests
over 144 encode cases (351 → 357); the writer-agreement test added 1 more
(357 → 358); the command line's worker-count tests added 3, the
multi-run benchmark harness added 6, and the allocator's counter test
moved into its own binary with a peak test beside it, for 10 more
(358 → 368).

## 7. Not yet measured (explicitly no claims)

The **10 MB corpus success rate is now measured** — 748 of 750 images
(99.7%) under the decimal 10,000,000-byte ceiling, per category, with both
failures named (`docs/P5_CORPUS_REPORT.md` §3.4) — and general 16K
**performance** remains unmeasured beyond the one photograph. Parallel
decode *is* now measured (`docs/P5_REPORT.md`), but only for the parameters
recorded there. Palette demonstrably
shrinks palette-friendly tiles (CLI §3.1: 7.148x on a synthetic two-colour
image) and Run-length shrinks flat tiles (CLI §3.2: 33.851x on a synthetic
flat image). The predictor codec shrinks smooth and photographic content
(seed corpus: the 15360×8640 photo went from 0.774x to **0.312x** of its
canonical bytes), and the P3.5 study adds seven research codecs measured on
the same pixels — the best of them, CARC C, reaches 0.2995x on that photo
and 0.3062x across the corpus, against PNG's 0.3445x and WebP
lossless's 0.2575x.

What is **still** not claimed:

* **No distribution claim from the compression study.** The eleven-codec
  study is still a 9-image seed, so no per-resolution distribution or 10 MB
  success rate follows *from it*. The corpus itself now exists and is
  measured (`docs/P5_CORPUS_REPORT.md`); what has not happened is the
  eleven-codec study being re-run on it.
* **No claim that the corpus is representative.** It is five repositories
  chosen for coverage of five kinds of content, plus the project's own seed
  — 746 real images and 4 synthetic ones — and the category names are
  proxies for content types. The measured gap is real: `auto` is behind PNG
  on every one of the five categories, at 0.415× against PNG's 0.379× across
  the corpus.
* **No promotion of any experimental codec.** The research ids cannot
  reach a file, and CARC C is still 18.9% worse than WebP lossless across
  the corpus and expands incompressible data.
* **No claim that `cjxl -d 0` is the best JPEG XL can do.** JPEG XL lossless
  is measured on all nine images (0.2189× across the corpus, 0.2160× on the
  15360×8640 photograph) and is the strongest of the three baselines, but
  only one encoder setting was used.
* **No 16K performance claim.** The 12-case matrix reports memory and
  wall-clock times on one host; the writes take 2–461 s and are dominated
  by the predictor codec's arithmetic coding.
* **No general "P5 is faster" claim.** The parallel engine's speedup is
  1.87× on the first viewport of one file, with 8 workers, against the
  sequential baseline measured in the same run. It is not a claim about
  other images, other worker counts, or other hardware.
* **No claim that prefetching always helps.** The radius sweep shows it
  helping at 16K (best radius 2) and *not* helping at 2K (best radius 0).
  Both runs are kept, including the unfavourable one.
* **No statistical confidence in the P5 figures.** Each scenario is one
  run. The variance between separate invocations is visible in the raw
  files — the standalone no-prefetch pan (415.94 ms) and the sweep's
  radius-0 run (316.91 ms) are the same measurement on the same file and
  differ by roughly a quarter — so the comparisons are taken from the
  sweep, which runs the radii back to back in one process. **P5.2 has now
  measured how big that variance is: the run-to-run spread is 18.5% to
  52.4% of the median** (`docs/P5_BENCH_REPORT.md` §3.5), so the single-run
  P5 figures are read as indicative, not as measurements.
* **No latency matrix for the parallel engine.** The byte-identity
  correctness matrix covers 144 cases (8 formats × 6 sizes × 3 tile
  geometries). **P5.2 has now measured the latency** over the tile size
  (256, 512, 1024) and the worker count (1, 2, 4, 8), with ten runs per
  case, against the sequential baseline in the same run. What is still
  missing is the **viewport size** as a third axis, and any content other
  than one photograph.
* **No cold-read measurement.** The P5.2 matrix reads the photograph
  through the operating system's page cache after the first pass, so its
  figures are warm-cache figures.
* **No memory claim in the P5.2 figures.** That report measures latencies;
  the encoder's working-set bound is P5.1's, measured separately with a
  source that never touches the page cache.
* **No claim that the pool helps at every size.** At tile size 1024 the
  viewport needs two tiles, so eight workers are *slower* than four
  (158.89 ms against 135.85 ms). A viewer must not use the pool when
  there is nothing to parallelise.
* **No linear-scaling claim for the parallel encoder.** Eight workers cut
  the 16K encode 4.27× (128962.99 ms → 30174.82 ms), not 8×, on a 10-core
  mobile part that was also running the rest of the toolchain. The scaling
  is sub-linear and the report says so. **On the photograph, with ten runs
  per case, the figure is 6.15× (78793.67 ms → 12811.94 ms) at tile size
  256** — still sub-linear, and still not a claim about a dedicated server.
* **No statistical confidence in the encoder figures either.** Each worker
  count is one run. An earlier run measured the one-worker case at 176812 ms
  against this run's 128962.99 ms, before the scratch-buffer reuse was
  added — the variance is visible and is not smoothed away. **P5.2 has
  since measured the encoder properly**: ten runs per case, with the spread
  reported as a percentage of the median, in `docs/P5_BENCH_REPORT.md` §3.
* **No claim that the encoder is fast.** 30174.82 ms for a 1 GiB image is
  not fast in the absolute sense; the claim is only that more workers make
  it faster and do not change the file. On the photograph, 12811.94 ms for
  398,131,200 canonical bytes is the same kind of claim and no more.
* **No claim about a disk failure partway through the streaming write.** A
  failed *encode* removes the temporary file and leaves the target
  untouched; a failure during the final rename is the operating system's
  business.
* **No edit-latency claim against another editor.** The editing timings
  are measurements on this host with no baseline to compare against.
* **No coverage-guided fuzzing result *on this host*.** The deterministic
  harness ran here; the coverage-guided fuzzer ran in CI on Linux (132,429,711
  executions in the first run, a second run that found the header defect,
  and 78,850,985 executions in the third that re-verified the fix), not on
  this Windows-GNU machine.
* **No claim that the fuzzing found everything.** It found one real defect
  (§A22). That is a statement about the inputs that were tried, not a proof
  that the library has no others.
* **No graphical editor.** The engine is a library. There is no window, no
  pan-and-zoom widget, and no UI in this repository.

Raw stays ≈1.0x and remains the guaranteed fallback. The 16384×16384 RGBA8
case that was skipped in §3.4 for insufficient free memory has since been
run through the streaming writer and the file-backed editor, and passes the
64 MB target; what is still missing is a *whole-image in-memory* decode of
a 16K RGBA8 file, which is a different question from the one P4 asks. No
"faster than X" or "fits in 10 MB" claim is made anywhere in this
repository.

## 8. Next phase

P5 (streaming and parallel decoding) is **closed**: all six stages are
done, the 16K acceptance case ran, and the evidence is in
`docs/P5_REPORT.md` with every quoted figure checked against the raw JSON
by `benchmarks/verify_reports.py`. The work continues in this order:

* **P6** — the multiresolution preview pyramid. P5's engine reads one
  level of an image at a time; a pyramid needs a second grid of tiles, so
  it is a format revision with a migration policy and compatibility tests.
* **P7** — adaptive (variable-size) tiling. The grid is still fixed at
  256 × 256, and a spatially adaptive layout would change both the
  compression and the shape of a viewport's tile set. The prefetch policy
  would have to be re-measured on it.
* **P2** — the stable codec ids 4–6 are still reserved. The P3.5 study
  says **not** to promote any of the research codecs on the current
  evidence; a promotion needs a specification update, a version bump, a
  migration policy and compatibility tests, in that order.
* **The parallel-decode coverage matrix is done.** The integration test
  `crates/vpng-view/tests/parallel_matrix.rs` checks 144 cases — all eight
  v0.1 pixel formats × six representative sizes × three tile geometries —
  and asserts that every tile the pool produces is byte-identical to the
  sequential decoder's. What is not measured is how the latency moves with
  the tile size, the viewport size and the worker count.

Two housekeeping items were carried forward from this milestone. One is now
closed and one is still open:

1. **Closed (2026-11-18).** `docs/VPNG_SPEC_0_1.md` described header byte 27
   as the "codec used for all tiles in v0.1", while the implementation treats
   it as informational and the per-tile directory entry as authoritative. The
   specification now says so: §2.3 resolves the byte as **informational**,
   §4.1 marks the directory entry's `codec id` as authoritative, and §5
   repeats that the header byte plays no part in decoding. The clarification
   is dated at the top of the document and cross-references ADR-0003. The
   byte, its offset, its size and the format version are unchanged, so no
   file changes meaning and the golden fixtures are untouched. The behaviour
   is pinned by `crates/vpng-testkit/tests/default_codec.rs` (7 tests).
2. **Closed (2026-11-18).** The `cargo-fuzz` targets could not be exercised
   locally — not for want of a toolchain (the nightly toolchain and
   `cargo-fuzz` v0.13.2 both install and run, and the seven targets are
   correctly wired) but because the Windows-GNU target cannot support
   coverage-guided fuzzing: ASan is unsupported for it, LLVM's libFuzzer
   Windows build is MSVC-only, and rustc's sanitizer-coverage module is
   ELF-only. **The Linux path has now been verified**: the first GitHub
   Actions run executed all seven coverage-guided targets under
   AddressSanitizer — 132,429,711 executions, 0 findings (run
   44264162940). **The second run found a real defect** (run 44266490454):
   `Header::parse` accepted a zero-dimension header that `Header::to_bytes`
   refused to serialize. It is fixed and pinned by a regression test that
   replays the exact crash artifact, and **the third run re-verified the
   fix** (44274770109): 78,850,985 executions, 0 findings. The evidence for
   each local limit is in `docs/P4_COMPATIBILITY_REPORT.md`.

---

## 9. The future release plan (v0.2 → v0.23)

The phase gates in §2 say *what* is being built. This section says *when*
it is meant to ship, release by release, so that the work has an order and
not just a list.

**How to read it.**

* **A version is a product release, not automatically a format change.**
  The container's own version lives in the header (bytes 8–9: major and
  minor), and that minor version moves **only** when the layout changes.
  The "format revision?" column says which releases move it; the rest are
  engine, tooling and interface releases that leave every existing file
  readable and byte-identical.
* **Compression is a standing line of work, not one project at the end.**
  Every release from v0.2 to v0.23 carries compression work: a codec, a
  measurement, a tuning pass, or a format revision that opens a new
  compression axis. A release that ships without touching compression
  would be a release that leaves the main reason to use the format
  untouched.
* **No dates are promised, and no version is claimed.** Every row is a
  plan. A version ships when its gate passes and the evidence is recorded
  here; until then its state is NOT STARTED. The only versions that are
  more than a plan are the ones in §2, and those say what was executed.
* **The invariants hold in every row.** Lossless reconstruction, exact
  round-trips, deterministic output, compatibility with the existing
  golden fixtures, no silent binary-layout change, and no lossy fallback.
  A research codec never reaches a file: the research ids `0x80..=0xFF`
  are rejected by the decoder with `UnsupportedFeature`, and promoting one
  to a stable id requires a specification update, a version bump, a
  migration policy and compatibility tests, in that order.
* **Two version numberings live side by side, and they are not the same
  thing.** The rows in this table are **product releases** (`v0.2` here is
  the next release). The **format's** minor version lives in the header
  (bytes 8–9) and has already moved: it is now **0.2**, because the P6/P7
  milestone landed the preview pyramid and the exact-partition tiling in one
  revision (`docs/VPNG_SPEC_0_2.md`). That revision moved no field and no
  offset — the header is still 64 bytes and the directory entry is still
  40 bytes — and the default encoder path still writes **format 0.1**, so
  every existing file stays readable and byte-identical. The rows below
  therefore name a *format* change in the "Format revision?" column, not a
  release number.

| Version | Theme | Compression work in this release | Format revision? | Draws on | State |
|---|---|---|---|---|---|
| v0.2 | streaming and parallel engine | extend the encoder's per-tile cost model so `auto` chooses among **all** stable codecs, and measure the change on the 9-image seed corpus | no | P5, P2 | PARTIAL — the streaming and parallel encoder is done (P5.1: one worker and N workers write the same file; the CLI streams to disk with `--workers N`), and the cost-model extension has not been made against the 750-image corpus |
| v0.3 | adaptive tiling | variable-size tiles give the predictor more context: measure the ratio change against the fixed grid on the same corpus and the same denominator | **yes** — the partition representation | P7 | **DONE as a format revision** (the partition representation shipped in format 0.2 with the P6/P7 milestone) — but the *result* is unfavourable and recorded: adaptive tiling is a compression wash over the 750-image corpus (−0.22% bytes), and it **hurts UI screenshots**. The partition it enables is the point: half the tiles at the median. Evidence: `docs/P6_P7_REPORT.md` |
| v0.4 | preview pyramid | the pyramid levels are written with the same stable codecs, and the level a reader actually fetches is measured rather than assumed | **yes** — a second grid of tiles | P6 | **DONE as a format revision** (the pyramid shipped in format 0.2 with the P6/P7 milestone): the level index travels in the directory entry's `flags`, the whole picture is available 66× sooner at 1/64 scale, and the cost — +32.7% bytes, +44.2% encode time — is measured and stated. Evidence: `docs/P6_P7_REPORT.md`, `docs/VPNG_SPEC_0_2.md` |
| v0.5 | stable codec id 4 (LZ77) | implement the reserved LZ77 codec as stable id 4: golden vectors, conformance tests, and a measured ratio against ids 1–3 | **yes** — the codec table | P2 | NOT STARTED |
| v0.6 | stable codec id 5 (dictionary) | implement predictor + dictionary as stable id 5, with the dictionary scope stated exactly (tile-local, per the ADR) | **yes** — the codec table | P2 | NOT STARTED |
| v0.7 | stable codec id 6 (context-adaptive entropy) | implement the context-adaptive entropy codec as stable id 6, with the per-tile model cost included in the encoder's choice | **yes** — the codec table | P2 | NOT STARTED |
| v0.8 | the 750-image corpus | replace the seed corpus with the full one and re-derive the auto-selection thresholds from it; every ratio is reported against PNG, WebP lossless and JPEG XL lossless | no | P8, P7 | **PARTIAL** (the corpus exists and is measured — 750 images, a SHA-256 manifest, and the per-category distribution with PNG, WebP lossless and JPEG XL lossless on the same canonical pixels; the **auto-selection thresholds have not been re-derived** from it yet, which is the v0.20 row) |
| v0.9 | network streaming | add a `recompress` operation: re-encode a file's payloads with the current best codec set, changing the bytes and not the pixels | no | P13, P2 | NOT STARTED |
| v0.10 | Python bindings | expose the per-tile codec choice and the measured ratio through the API, so a pipeline can select codecs itself against the same numbers | no | P9 | NOT STARTED |
| v0.11 | graphical viewer | the viewer reports the codec mix of the file it opened, so a user can see why the file is the size it is | no | P11 | NOT STARTED |
| v0.12 | WebAssembly package | measure decode time per codec **on the target device** and re-tune the auto-selection thresholds for that device; the build ships the stable codecs only | no | P12 | NOT STARTED |
| v0.13 | graphical editor | the editor shows a dry-run comparison — what the file would weigh under a different codec — before the commit, measured with the encoder's own selection function | no | P11 | NOT STARTED |
| v0.14 | layers and masks | each layer is compressed independently and the container reports the per-layer ratio; cross-layer prediction is explicitly out of scope | **yes** — several grids in one file | P14 | NOT STARTED |
| v0.15 | GPU-ready payloads | the lossless payload stays normative; the GPU payload is an optional sibling, and the size of both is reported so the trade is visible rather than implied | **yes** — the payload kinds | P15 | NOT STARTED |
| v0.16 | shared dictionaries | a dictionary shared across the tiles of one layer, measured against the per-tile dictionary on the same corpus | **yes** — the payload context | P7 | NOT STARTED |
| v0.17 | context modelling | promote a research context model to a stable id **only** if it beats the stable predictor on the full corpus; the evidence is the comparison table, not the seed | **yes** — the codec table | P7, P2 | NOT STARTED |
| v0.18 | parallel encoding | parallel encoding must not change the bytes: the same file must come out of one thread and of four, byte for byte | no | P5.1, P2 | **DONE** (the parallel encoder landed with P5.1: one worker and N workers produce the same file, verified by digest over 144 encode cases and by the 16K benchmark's four worker counts) |
| v0.19 | streaming encoder | the streaming writer chooses codecs per tile as it goes, and produces exactly the file the batch encoder produces | no | P5.1, P2 | **DONE** (the streaming writer is a library feature in `vpng-encoder` now, and produces the batch encoder's file byte for byte; a `PixelSource` lets the image be encoded without ever being in memory) |
| v0.20 | corpus-driven defaults | the auto codec's thresholds come from the corpus rather than from a guess; the change is measured on the whole corpus, and the seed-only numbers are reported as seed-only | no | P8, P7 | **PARTIAL** (the corpus to derive the thresholds from now exists and is measured — `auto` is 0.415× against PNG's 0.379× across 750 images; the thresholds have **not** been re-derived, and the seed-only numbers are still the seed's) |
| v0.21 | closing the gap to JPEG XL | rANS with a shared dictionary and a trained per-category model, measured against the JPEG XL lossless baseline of 0.2189× on the corpus | **yes** — the codec table | P7, P2 | NOT STARTED |
| v0.22 | metadata chunks | metadata is stored compressed and its compressed size is accounted for exactly, so the file's size accounting stays exact | **yes** — the chunk layout | P10 | NOT STARTED |
| v0.23 | format freeze (the v1.0 candidate) | freeze the codec table, the golden fixtures and the compatibility tests; the research codecs stay research-only and cannot reach a file | **yes** — the version itself | P10 | NOT STARTED |

**What this plan does not say.**

* It does not say any of these versions exists. They are the order of the
  work, not a promise of a date.
* It does not say that a research codec will be promoted. v0.17 and v0.21
  are the versions where that decision *could* be taken, and the
  measurement that decides it is named in the row.
* It does not change the v0.1 layout. The first row that moves the
  container's minor version is v0.3, and each such row carries its own
  migration policy and compatibility tests.
* It does not cover work outside the format: security audits, release
  engineering and the compatibility guarantees themselves are tracked by
  the phase gates in §2, not here.
