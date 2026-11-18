# Roadmap status

**Updated:** 2026-11-16
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
| `cargo test --workspace --all-targets` | **exit 0**, `312 passed; 0 failed` |
| `cargo fmt --all --check` | **exit 0**, no diffs |
| `cargo clippy --workspace --all-targets` | **exit 0**, zero warnings |

No Visual Studio is required: the GNU host ships `rust-lld` and a
`self-contained` linker set.

## 2. Phase gates

| Phase | Gate summary | Status | Evidence |
|-------|--------------|--------|----------|
| P0 — repository audit and specification | unambiguous field sizes; executable serialization tests; reproducible build env | **PASS** | `vpng-container` 19 tests pass (header is exactly 64 bytes, directory entry exactly 40 bytes, round-trips exact, checksum detects a flipped byte). Build environment recorded in §1 and `docs/REPOSITORY_AUDIT.md`. |
| P1 — minimal valid container | exact round-trip all formats; golden files; tile-only decode; malformed tests | **PASS** | `roundtrip.rs` 7 pass, `golden.rs` 5 pass, `malformed.rs` 12 pass, `vpng-decoder` 8 pass. Structural integrity checking (`vpng check`) is implemented and tested. See §3 for the CLI end-to-end run. |
| P2 — baseline codecs | palette, RLE, predictor, LZ77, entropy | **PARTIAL** | Palette (id 1) implemented with byte-level golden vectors, whole-file golden fixture, and property tests (§3.1, A06); RLE (id 2) implemented with byte-level golden vectors, a whole-file golden fixture, and conformance tests (§3.2, A07); Predictor+entropy (id 3) implemented with seven spatial predictors, per-row measured-cost selection, an adaptive arithmetic coder, byte-level golden vectors, a whole-file golden fixture, and conformance/property tests (§3.3, A08). LZ77 (id 4), predictor+dictionary (id 5), and context-adaptive entropy (id 6) remain reserved and are rejected on decode. Cost-based per-tile selection with a Raw fallback is implemented and tested. |
| P3 — adaptive tiling, region decode | region == crop; coverage validation; bounded cache; parallel decode | **PARTIAL** | Fixed-grid region decode and exact-coverage validation pass (`region_decode_equals_full_decode_crop`). A **bounded byte-budget LRU tile cache** now exists and is tested (`the_cache_evicts_when_the_budget_is_exceeded`; A14). Adaptive (variable-size) tiling and parallel decode are still not started. |
| P4 — editing engine | localized edit; payload reuse; crash safety; undo/redo | **PASS** | The engine is implemented in `crates/vpng-editing` (51 tests). A localized edit decodes 1 tile of 64 and modifies 1; 63 of 64 payloads are preserved byte for byte and 1 is rewritten; BEGIN/COMMIT/ABORT, atomic copy-on-write save, and two injected failure points all preserve the original; undo/redo restore the pixels exactly. A successful commit is a **new baseline**: undo and redo cannot cross it, while a failed commit preserves the whole editing state. The design decisions it resolved are ADR-0012 (atomic replacement, §18.9/§18.10) and ADR-0013 (cache eviction, §18.13). Evidence: `docs/P4_EDITING_ENGINE_REPORT.md`, `benchmarks/p4_editing_results.json`. |
| P5 — preview and multiresolution | thumbnail, pyramid | **NOT STARTED** | |
| P6 — 16K and memory engineering | 16K stress; streaming; budgets | **PARTIAL** | A tile-by-tile streaming writer and a 12-case memory matrix now exist: 4096², 8192² and 16384² × {gray8, rgb8, rgba8, rgba16}, **12 PASS / 0 FAIL / 0 BLOCKED**. The 16384×16384 RGBA16 case (2 GiB of canonical pixels) writes in an 8.8 MB heap peak and edits with a 13.2 MB resident-set growth, against a 64 MB target. A bounded cache and a bounded history are implemented. Parallel limits and parallel decode are still not started. Evidence: `docs/P4_MEMORY_VALIDATION.md`, `benchmarks/p4_memory_results.json`. |
| P7 — compression research | controlled experiments | **PARTIAL** | Eleven codecs were measured on identical pixels and identical tile layouts: the four stable codecs, four context-adaptive residual models (CARC A–D), an LZ77 dictionary codec and three hybrids, with pinned golden vectors for each. Every image was also measured against PNG, WebP lossless and JPEG XL lossless, so every column of the comparison shares one denominator. The results are in `benchmarks/p3_5_raw_results.json` (per-image, per-tile, per-category, per-resolution, aggregate and worst-case) and `benchmarks/p3_5_comparison.csv`. The **750-image corpus and statistical reporting do not exist**; the study is a 9-image seed. Evidence: `docs/P3_5_COMPRESSION_RESEARCH_REPORT.md`. |
| P8 — 10 MB target evaluation | exact accounting; honest failure | **PARTIAL** | Exact size accounting passes (`size_accounting_is_exact`); refusal is honest and typed (`--max-size 1024` → exit 6 reporting the smallest lossless size 240,148). A 9-image seed corpus exists, but the 750-image target is unmet, so the per-category success distribution is unmeasured. |
| P9 — Python API | bindings around Rust | **NOT STARTED** | CLI-driver tooling only. |
| P10 — hardening and v1.0 | fuzzing; freeze | **PARTIAL** | A deterministic mutation harness over 7 targets runs in CI (200,000 iterations × 7 targets = 1,400,000 executions, 0 findings), and the 7 `cargo-fuzz` targets ran **coverage-guided in CI on Linux** (run 44264162940: 132,429,711 executions, 0 findings). A second run (44266490454) **found a real defect** — `Header::parse` accepted a zero-dimension header that `Header::to_bytes` refused to serialize — which is fixed and pinned by a regression test. The coverage-guided fuzzer cannot run on this Windows-GNU host — the toolchain and `cargo-fuzz` install and run, but the AddressSanitizer runtime, LLVM's MSVC-only libFuzzer Windows build and rustc's ELF-only coverage module each stop it there — and the format is not frozen. |

### 2.1 The forward plan (2026-11-18) and how it maps onto the phases above

The phase table in §2 is the record of work as it was planned and delivered.
The forward plan renumbers the remaining work, so this table maps the new
names onto the old ones — the old numbering stays authoritative for what has
already been done.

| Forward plan | Objective | Existing phases it draws on | State |
|---|---|---|---|
| P3.5 + P4 — compression and editing | the two gates just closed | P3.5 (recorded as P7) and P4 | **verified; committed** (`c821f25`) |
| P4.1 — specification and compatibility closure | resolve `default_codec`; verify the Linux fuzzing CI path | P10 | **CLOSED** (2026-11-18): the specification clarification is committed (`b692227`) and the Linux coverage-guided fuzz job ran (runs 44264162940 and 44266490454 — the second found the header defect, which is fixed and pinned) |
| P5 — streaming and parallel decoding | open a 16K image and navigate it without a full-image allocation | P6 (streaming, parallel limits) and the parallel-decode part of P3 | **IN PROGRESS** — design in `docs/P5_DESIGN.md`; stage 1 (the streaming reader) is done: `crates/vpng-source` |
| P6 — multiresolution preview pyramid | fast zooming, progressive presentation | P5 | NOT STARTED |
| P7 — adaptive tiling | variable tile dimensions and spatially adaptive layouts | P3 | NOT STARTED |
| P8 — production validation | the 750-image corpus, multiple platforms, 16K workloads | P8 | PARTIAL (seed corpus only) |
| P9 — Python bindings and integration | expose the stable engine to other applications | P9 | NOT STARTED |
| P10 — format freeze and release | compatibility guarantees, security audit, v1.0 | P10 | PARTIAL (the format is not frozen) |

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
| A13 | Parallel decode | determinism test | NOT STARTED | |
| A14 | Bounded cache | allocation test | **PASS** | `TileCache` is a strict LRU with a **byte** budget (`with_cache_bytes`), not a tile count. `the_cache_evicts_when_the_budget_is_exceeded`; the benchmark's cache scenario fills a 524,288-byte budget exactly with 2 evictions and never exceeds it. A tile larger than the whole budget is rejected rather than cached. |
| A15 | Localized edit | instrumented test | **PASS** | `a_region_read_does_not_decode_unrelated_tiles`, `a_region_write_changes_only_the_tiles_it_intersects`, `the_engine_never_holds_a_full_image_buffer`. Measured: a 64×64 edit on 2048×2048 decodes 1 tile of 64 and dirties 1 (1.5625% of the image); on 16384² it decodes 1 tile of 1,024. |
| A16 | Undo/redo | exact hash tests | **PASS** | `undo_and_redo_restore_the_pixels_exactly`, `undo_makes_a_tile_clean_again`, `undo_with_no_history_reports_that_there_is_nothing_to_do`, `a_new_edit_invalidates_the_redo_chain`. The benchmark confirms `undo_restores_pixels` and `redo_restores_pixels` with a byte-exact region comparison. |
| A17 | Crash-safe commit | fault injection | **PASS** | The save never writes to the original: it writes a temporary file in the same directory, `sync_all`s it, and renames it over the target. `FailurePoint::AfterBytes` and `FailurePoint::BeforeRename` are injected in `a_failed_write_leaves_the_original_file_untouched` and `a_failure_before_the_rename_leaves_the_original_untouched`; in both cases the original is byte-identical, still parses, still passes its integrity check, and no temporary file is left behind. |
| A18 | Preview separation | format and API tests | NOT STARTED | |
| A19 | 16K stress | recorded run | **PASS** | A 12-case matrix — 4096², 8192², 16384² × {gray8, rgb8, rgba8, rgba16} — **12 PASS / 0 FAIL / 0 BLOCKED** (`benchmarks/p4_memory_results.json`). Every case writes the file, verifies it, edits it, commits it and verifies the result. The 16384×16384 RGBA16 case (2,147,483,648 canonical bytes) writes with an 8.83 MB heap peak and edits with an 11.69 MB resident-set growth and a 12.52 MB heap peak, against the 64 MB target. A general 16K **performance** claim is not made; the write times are 2–461 s and are dominated by the predictor codec's arithmetic coding. |
| A20 | 10 MB enforcement | boundary tests | **PASS** | `size_limit_is_reported_not_violated`; CLI step 9 exit 6 |
| A21 | Corruption detection | mutation tests | **PASS** | `malformed.rs` 12 tests; `vpng-decoder` integrity tests; CLI step 12 (`decode`) and step 14 (`check`) exit 4 |
| A22 | Fuzz robustness | fuzz report | **PARTIAL** | `malformed.rs` 12 tests, `mutation_sweep.rs` 4 tests (every truncation and every single-byte substitution, plus 2,000 seeded multi-byte corruptions per file, with no panic), and a deterministic mutation harness over 7 targets: **200,000 iterations × 7 targets = 1,400,000 executions, 0 findings** (`benchmarks/results/fuzz-mutation-report.json`). The 7 `cargo-fuzz` targets ran **coverage-guided in CI on Linux** (run 44264162940, 2026-11-18): **132,429,711 executions across the seven targets, 0 findings**, with AddressSanitizer and libFuzzer's coverage feedback. **A second run found a real defect** (run 44266490454): `Header::parse` accepted a zero-dimension header that `Header::to_bytes` refused to serialize — the parser and the serializer disagreed about the same 74 bytes. It is **fixed** (`Header::parse` now applies the same non-zero dimension rule as `Header::new` and `Header::to_bytes`) and **pinned**: the exact crash artifact is replayed on every test run in `crates/vpng-fuzzing/tests/fuzz_regressions.rs`. **Coverage-guided fuzzing cannot run on this Windows-GNU host** — the toolchain and `cargo-fuzz` v0.13.2 install and run, and `cargo fuzz list` sees all seven targets, but three platform limits stop the local run on `x86_64-pc-windows-gnu` (ASan is unsupported for the target, LLVM's libFuzzer Windows build is MSVC-only, and rustc's sanitizer-coverage module is ELF-only) — so the local evidence is the deterministic harness only. |
| A23 | Python API | integration tests | NOT STARTED | |
| A24 | v1.0 compatibility | golden corpus | NOT STARTED | |

## 5. Test inventory (executed)

Command: `cargo test --workspace --all-targets -j 2`, release and debug
profiles, `x86_64-pc-windows-gnu`.

| Suite | Tests | Result |
|-------|------:|--------|
| `vpng-cli` unit | 11 | ok |
| `vpng-codecs` unit | 110 | ok |
| `vpng-container` unit | 21 | ok |
| `vpng-core` unit | 19 | ok |
| `vpng-decoder` unit | 8 | ok |
| `vpng-editing` unit | 51 | ok |
| `vpng-encoder` unit | 9 | ok |
| `vpng-fuzzing` unit | 0 | ok |
| `vpng-fuzzing` bin `vpng-fuzz-mutate` | 0 | ok |
| `vpng-fuzzing` `tests/fuzz_smoke.rs` | 3 | ok |
| `vpng-fuzzing` `tests/fuzz_regressions.rs` | 3 | ok |
| `vpng-source` unit | 11 | ok |
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
| `vpng-tiling` unit | 6 | ok |
| `vpng-validation` unit | 2 | ok |
| `vpng-validation` bins | 0 | ok |
| Doc-tests | 0 | ok |
| **Total** | **312** | **312 passed, 0 failed** |

The 115 new tests are: 52 codec tests (22 LZ77, 14 CARC, 12 hybrids, plus
the registry and entropy-stage tests), 51 editing tests, 3 fuzzing tests,
2 memory-accounting tests and 7 `default_codec` conformance tests.

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
(296 → 301); the P5 streaming reader added 11 tests (301 → 312).

## 7. Not yet measured (explicitly no claims)

Corpus success rate under 10 MB, general 16K **performance**, parallel
decode, and streaming *latency* remain unmeasured. Palette demonstrably
shrinks palette-friendly tiles (CLI §3.1: 7.148x on a synthetic two-colour
image) and Run-length shrinks flat tiles (CLI §3.2: 33.851x on a synthetic
flat image). The predictor codec shrinks smooth and photographic content
(seed corpus: the 15360×8640 photo went from 0.774x to **0.312x** of its
canonical bytes), and the P3.5 study adds seven research codecs measured on
the same pixels — the best of them, CARC C, reaches 0.2995x on that photo
and 0.3062x across the corpus, against PNG's 0.3445x and WebP
lossless's 0.2575x.

What is **still** not claimed:

* **No distribution claim.** The compression study is a 9-image seed, not
  the 750-image corpus, so no per-resolution distribution or 10 MB success
  rate follows from it.
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
* **No edit-latency claim against another editor.** The editing timings
  are measurements on this host with no baseline to compare against.
* **No coverage-guided fuzzing result *on this host*.** The deterministic
  harness ran here; the coverage-guided fuzzer ran in CI on Linux (132,429,711
  executions in the first run, and a second run that found the header
  defect), not on this Windows-GNU machine.
* **No claim that the fuzzing found everything.** It found one real defect
  (§A22). That is a statement about the inputs that were tried, not a proof
  that the library has no others.

Raw stays ≈1.0x and remains the guaranteed fallback. The 16384×16384 RGBA8
case that was skipped in §3.4 for insufficient free memory has since been
run through the streaming writer and the file-backed editor, and passes the
64 MB target; what is still missing is a *whole-image in-memory* decode of
a 16K RGBA8 file, which is a different question from the one P4 asks. No
"faster than X" or "fits in 10 MB" claim is made anywhere in this
repository.

## 8. Next phase

P3 and P6 continue in parallel, and neither blocks the other:

* **P3** — adaptive (variable-size) tiling and parallel independent tile
  decode. The bounded cache that the gate asks for now exists; the grid is
  still fixed.
* **P6** — parallel decode limits and the remaining streaming work, plus
  the preview/multiresolution surface P5 needs.
* **P2** — the stable codec ids 4–6 are still reserved. The P3.5 study
  says **not** to promote any of the research codecs on the current
  evidence; a promotion needs a specification update, a version bump, a
  migration policy and compatibility tests, in that order.

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
   replays the exact crash artifact. The evidence for each local limit is in
   `docs/P4_COMPATIBILITY_REPORT.md`.
