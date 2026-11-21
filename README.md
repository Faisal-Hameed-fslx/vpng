# VPNG — Vanta Portable Network Graphics

A lossless, adaptive, tiled, random-access image format and editing engine.

**Primary requirement:** exact pixel reconstruction.
**Secondary target:** fit eligible images within 10 MB — measured, never promised.
**Status:** v0.2 container implemented, compiled, and tested; the editing engine
(P4), the compression research (P3.5), the interactive engine (P5) and the
parallel/streaming encoder (P5.1) have landed, and so have the preview pyramid
(P6) and the adaptive partition (P7) in one format revision. `cargo build
--workspace` and `cargo test --workspace --all-targets`
both pass on `stable-x86_64-pc-windows-gnu` rustc
1.100.0. Large-image stress runs, a 16K memory matrix, an editing benchmark, a
16K viewing benchmark, a 16K encoding benchmark and a seed-corpus measurement
are recorded in `docs/BENCHMARKS.md`, `docs/P5_REPORT.md` and
`docs/P5_ENCODER_REPORT.md`. See `docs/ROADMAP_STATUS.md`.

## What works today

* A precise, ADR-backed binary format: `docs/VPNG_SPEC_0_1.md`, revised as
  `docs/VPNG_SPEC_0_2.md` (the same layout, with the level index carried in
  the directory entry's `flags` and a level's tiles allowed to differ in
  size).
* All eight v0.1 pixel formats (Gray/GrayAlpha/RGB/RGBA at 8 and 16 bits).
* A validated container: 64-byte header, 40-byte directory entries, CRC32C
  checksums, whole-image canonical SHA-256 digest.
* Raw, Palette, Run-length, and Predictor+entropy tile codecs, uniform tiling,
  per-tile codec selection, tile decode, full decode, and region decode.
* A **preview pyramid** (format v0.2): derived levels built with an
  integer-only 2 × 2 box average, the level marked in each entry's `flags`,
  and a decoder that reads any level. Measured on the corpus's own 16K
  photograph: the whole picture in **197.94 ms** at 1/64 scale against
  **13062.32 ms** at full resolution, for +32.7% bytes and +44.2% encode
  time. See `docs/P6_P7_REPORT.md`.
* An **adaptive partition** (format v0.2): variable-size tiles forming an
  exact cover of the level, validated before a file is written. Measured
  over the whole 750-image corpus it is a compression **wash** (−0.22%
  bytes, and it hurts UI screenshots) and it delivers **half the tiles at the
  median**. The unfavourable result is kept. See `docs/P6_P7_REPORT.md`.
* An **editing engine**: region reads and writes, dirty-tile tracking,
  transactions, bounded tile-level undo/redo, and a copy-on-write save that
  reuses every untouched payload byte for byte and replaces the file
  atomically. A 64×64 edit on a 16384×16384 image decodes exactly one tile.
  A successful commit is a new baseline: undo and redo cannot cross it,
  while a failed commit preserves the whole editing state.
* A CLI: `encode`, `decode`, `info`, `check`, `verify`, `decode-region`,
  `extract-tile`.
* An **interactive engine** (`crates/vpng-view`): a bounded scheduler with
  viewport priority and cancellation, a parallel worker pool whose tiles are
  byte-identical to the sequential decoder's, and a predictive prefetch
  whose radius is measured rather than assumed. The 16K RGBA8 acceptance
  case **ran**: the first viewport takes 63.56 ms against the sequential
  baseline's 118.96 ms, and the whole viewing run peaks at 47.9 MB of
  resident set against the 64 MB target. See `docs/P5_REPORT.md`.
* A **parallel and streaming encoder** (`crates/vpng-encoder`): a
  `PixelSource` hands the encoder one tile at a time, so a 16K image is
  encoded without ever being in memory, and worker threads encode chunks of
  tiles while a coordinator consumes them in tile order. Eight workers cut
  the 16K encode 4.27× (128962.99 ms → 30174.82 ms), and **every worker
  count wrote the same file** — one SHA-256, checked over 144 encode cases.
  The whole run peaks at 19.3 MB of resident set against the 64 MB target.
  `vpng encode` uses it, with `--workers N`. See `docs/P5_ENCODER_REPORT.md`.
* A **measurement foundation** (`crates/vpng-validation`): every benchmark
  case is repeated ten times and reported with a **median** and a **p95**,
  not a single figure, against a sequential baseline measured in the same
  run, on a real photograph, over a tile-size × worker-count matrix. The
  rules are written down in `docs/BENCHMARKS.md` §Method and pinned by unit
  tests. See `docs/P5_BENCH_REPORT.md`.
* Conformance tests: exhaustive round-trip, region-vs-crop, malformed-input
  sweeps, a bounded mutation sweep, golden files, the `default_codec`
  clarification, and a deterministic fuzzing harness over seven targets.
  418 tests, all passing.
* Layered integrity: `info` (structure only), `check` (every payload CRC32C),
  `verify` (exact pixels against a reference).
* Measured evidence: large-image stress (up to a 15360×8640 photo and
  16384×16384 pixels) with sampled peak RSS and exactness checks, a 12-case
  16K memory matrix (all PASS, against a 64 MB target), an editing benchmark,
  a multi-run photographic latency matrix, and a 9-image seed corpus compared
  against PNG, WebP lossless and JPEG XL lossless. See `docs/BENCHMARKS.md`.
* A **compression study** of eleven codecs on identical pixels and identical
  tile layouts: the four stable codecs, four context-adaptive residual models
  (CARC A–D), an LZ77 dictionary codec and three hybrids, each with a pinned
  golden vector. Across the corpus the best research codec (CARC C) reaches
  0.3062× of the canonical bytes against PNG's 0.3445× — and 0.2575× for
  WebP lossless and 0.2189× for JPEG XL lossless, neither of which VPNG
  beats. **No research codec is promoted.**
  See `docs/P3_5_COMPRESSION_RESEARCH_REPORT.md`.

## What is explicitly not here yet

The Python bindings, the WebAssembly package, HTTP Range streaming, layers and
masks, GPU-ready payloads, and a frozen v1.0 format. The stable codec ids 4–6
are still reserved; the LZ77, CARC and hybrid codecs that were measured in the
P3.5 study
are **research-only** — their ids live in `0x80..=0xFF`, they cannot reach a
file, and the decoder rejects a file that names one. Coverage-guided fuzzing runs
in CI's nightly job on Linux —
132,429,711 executions across seven targets in the first run, a second run
that found a real defect (`Header::parse` accepted a zero-dimension header
that `Header::to_bytes` refused to serialize; it is fixed, the exact crash
input is replayed by the test suite, and a third run of 78,850,985
executions re-verified the fix) — but it cannot be run on the Windows-GNU
development host: the AddressSanitizer runtime, LLVM's MSVC-only libFuzzer
Windows build and rustc's ELF-only coverage module all stop it there (the
toolchain and `cargo-fuzz` themselves install and run); the local evidence is
the deterministic mutation harness (`crates/vpng-fuzzing`). There is **no
graphical editor** yet. Each phase has a row in the status table.

## What is being built next

The **measurement foundation**, then the **750-image corpus**, then the
container revisions. The order follows the agreed plan: faster tooling
first, so every later benchmark is affordable and trustworthy.

The interactive engine (P5) is done: a streaming reader (`crates/vpng-source`),
a bounded scheduler with viewport priority and cancellation, a parallel worker
pool whose output is byte-identical to the sequential decoder's, a predictive
prefetch, and a measured acceptance benchmark. Prefetch is a measured
trade-off, not a promise: it helps at 16K (best radius 2) and does not help at
2K, and both runs are kept in `docs/P5_REPORT.md` §3.3.

The parallel and streaming encoder (P5.1) is done too: the encoder now scales
across workers without changing a single byte of the file it writes. That is
what makes the 750-image corpus affordable, and it is the reason the corpus
comes before the container revisions rather than after them.

The **750-image corpus** is done as well (P5.3): 750 images in five
categories — photographs, screenshots, pixel art, scans, textures — 746 of
them real, fetched from five public repositories at pinned commits and
recorded by a SHA-256 manifest that names every image's canonical digest,
its source digest and the commit it came from. The distribution is measured
on it, and it is unfavourable and kept: `auto` writes **0.415×** the
canonical bytes against PNG's **0.379×**, WebP lossless's **0.242×** and
JPEG XL lossless's **0.219×**, so VPNG is behind PNG on every category. The
10 MB target succeeds on **748 of the 750** images, with both failures named.
See `docs/P5_CORPUS_REPORT.md`.

After that come the multiresolution preview pyramid (P6) and adaptive tiling
(P7), which are container revisions and are done back to back so the
container version is bumped once.

Both are now done, in one revision: the container's minor version is **0.2**
and the layout did not move with it — the header is still 64 bytes, the
directory entry is still 40 bytes, and the default encoder path still writes
v0.1 bytes, so every golden fixture and every existing file is untouched.
The pyramid puts the whole 16K picture on screen 66× sooner at 1/64 scale,
for +32.7% bytes and +44.2% encode time. The adaptive partition was measured
over the whole 750-image corpus and is a compression **wash** — it is kept
because it halves the tile count, not because it compresses better. The
next steps are the stable codec ids 4–6, then the ecosystem: Python, WASM,
HTTP Range, layers, GPU payloads and the graphical UI. See
`docs/P6_P7_REPORT.md`.

The roadmap also carries a **feature backlog** — a preview pyramid, Python
bindings, a graphical editor, a WebAssembly package, HTTP Range streaming,
layers and masks, GPU texture blocks — each attached to a phase in
`docs/ROADMAP_STATUS.md` §2.2, and a **release plan from v0.2 to v0.23**
(§9) in which compression work appears in every release. Both are plans, not
promises: no date is claimed and no version exists until its gate passes.

## Build

The GNU host (`x86_64-pc-windows-gnu`) is used and needs no Visual Studio: it
ships `rust-lld` and a `self-contained` linker set. This checkout pins the
toolchain with `rust-toolchain.toml`; the reference build used `rustc 1.100.0`.
To install the same host on a fresh machine:

```powershell
# fetch the x86_64-pc-windows-gnu rustup-init, then:
.\rustup-init.exe -y --default-host x86_64-pc-windows-gnu --profile minimal
```

Then:

```bash
cargo build --workspace
cargo test  --workspace
cargo run -p vpng-cli -- --help
```

## Usage

```bash
vpng encode input.png output.vpng --tile-size 256
vpng info output.vpng                              # header + directory only (fast)
vpng check output.vpng                             # verify every payload CRC32C
vpng decode output.vpng restored.png
vpng verify input.png output.vpng                  # exact pixels vs the original
vpng decode-region output.vpng crop.png --x 100 --y 200 --width 512 --height 512
vpng extract-tile output.vpng --tile-x 0 --tile-y 1 --output tile.png
vpng encode input.png out.vpng --max-size 10MB      # decimal ceiling (ADR-0006)
vpng encode input.png out.vpng --codec predictor  # raw | palette | rle | predictor | auto (default)
```

`info` reads only the header and directory, so it is O(1) in the tile count and
is **not** an integrity check. Use `check` to validate a file on its own, and
`verify` to compare it against the original image.

Exit codes: `0` ok, `2` bad args, `3` unsupported, `4` corrupt, `5` resource
limit, `6` size limit, `7` verification failed, `8` I/O.

## Layout

```text
crates/vpng-core        formats, geometry, checked arithmetic, errors, limits
crates/vpng-container   header, directory, checksums, canonical digest
crates/vpng-codecs      TileCodec trait + Raw, Palette, Run-length, Predictor (ids 0-3; 4-6 reserved)
                         + research-only CARC / LZ77 / hybrids under src/experimental/
crates/vpng-tiling      TileGrid and region-to-tile mapping
crates/vpng-encoder     canonical pixels -> .vpng bytes
crates/vpng-decoder     .vpng bytes -> full / tile / region pixels
crates/vpng-source      the streaming reader: container metadata + per-tile payload reads
crates/vpng-cache       the bounded, byte-budgeted LRU cache of decoded tiles
crates/vpng-view        the interactive engine: scheduler, worker pool, prefetch
crates/vpng-editing     the editing engine: regions, transactions, undo/redo, atomic save
crates/vpng-research    the P3.5 codec benchmark binary
crates/vpng-fuzzing     fuzz targets + the deterministic mutation harness
crates/vpng-validation  memory accounting, the streaming writer, the editing benchmark
crates/vpng-testkit     deterministic fixtures + conformance tests
crates/vpng-cli         the `vpng` binary
fuzz/                    cargo-fuzz wrappers over the vpng-fuzzing targets
docs/                   spec, audit, status, threat model, ADRs, milestone reports
```

## Claims policy

"Lossless" is used only for verified paths. "16K supported" only after measured
tests at a stated format. "Under 10 MB" only for a named image or reported corpus
subset. "Faster" only against named baselines, hardware, and settings. See
`docs/BENCHMARKS.md`.

## License

MIT. See `LICENSE`.
