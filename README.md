# VPNG — Vanta Portable Network Graphics

A lossless, adaptive, tiled, random-access image format and editing engine.

**Primary requirement:** exact pixel reconstruction.
**Secondary target:** fit eligible images within 10 MB — measured, never promised.
**Status:** v0.1 container implemented, compiled, and tested; the editing engine
(P4) and the compression research (P3.5) have landed. `cargo build
--workspace` and `cargo test --workspace --all-targets` both pass (283 tests,
0 failures) on `stable-x86_64-pc-windows-gnu` rustc 1.100.0. Large-image stress
runs, a 16K memory matrix, an editing benchmark, and a seed-corpus measurement
are recorded in `docs/BENCHMARKS.md`. See `docs/ROADMAP_STATUS.md`.

## What works today

* A precise, ADR-backed binary format: `docs/VPNG_SPEC_0_1.md`.
* All eight v0.1 pixel formats (Gray/GrayAlpha/RGB/RGBA at 8 and 16 bits).
* A validated container: 64-byte header, 40-byte directory entries, CRC32C
  checksums, whole-image canonical SHA-256 digest.
* Raw, Palette, Run-length, and Predictor+entropy tile codecs, uniform tiling,
  per-tile codec selection, tile decode, full decode, and region decode.
* An **editing engine**: region reads and writes, dirty-tile tracking,
  transactions, bounded tile-level undo/redo, and a copy-on-write save that
  reuses every untouched payload byte for byte and replaces the file
  atomically. A 64×64 edit on a 16384×16384 image decodes exactly one tile.
  A successful commit is a new baseline: undo and redo cannot cross it,
  while a failed commit preserves the whole editing state.
* A CLI: `encode`, `decode`, `info`, `check`, `verify`, `decode-region`,
  `extract-tile`.
* Conformance tests: exhaustive round-trip, region-vs-crop, malformed-input
  sweeps, a bounded mutation sweep, golden files, the `default_codec`
  clarification, and a deterministic fuzzing harness over seven targets.
  301 tests, all passing.
* Layered integrity: `info` (structure only), `check` (every payload CRC32C),
  `verify` (exact pixels against a reference).
* Measured evidence: large-image stress (up to a 15360×8640 photo and
  16384×16384 pixels) with sampled peak RSS and exactness checks, a 12-case
  16K memory matrix (all PASS, against a 64 MB target), an editing benchmark,
  and a 9-image seed corpus compared against PNG, WebP lossless and JPEG XL
  lossless. See `docs/BENCHMARKS.md`.
* A **compression study** of eleven codecs on identical pixels and identical
  tile layouts: the four stable codecs, four context-adaptive residual models
  (CARC A–D), an LZ77 dictionary codec and three hybrids, each with a pinned
  golden vector. Across the corpus the best research codec (CARC C) reaches
  0.3062× of the canonical bytes against PNG's 0.3445× — and 0.2575× for
  WebP lossless and 0.2189× for JPEG XL lossless, neither of which VPNG
  beats. **No research codec is promoted.**
  See `docs/P3_5_COMPRESSION_RESEARCH_REPORT.md`.

## What is explicitly not here yet

Adaptive (variable-size) tiling, parallel decode, the streaming *reader* and
the preview/multiresolution surface, the Python bindings, and a frozen v1.0
format. The stable codec ids 4–6 are still reserved; the LZ77, CARC and hybrid
codecs that were measured in the P3.5 study are **research-only** — their ids
live in `0x80..=0xFF`, they cannot reach a file, and the decoder rejects a file
that names one. Coverage-guided fuzzing runs in CI's nightly job on Linux —
132,429,711 executions across seven targets in the first run, and a second run
that found a real defect (`Header::parse` accepted a zero-dimension header
that `Header::to_bytes` refused to serialize; it is fixed and the exact crash
input is replayed by the test suite) — but it cannot be run on the Windows-GNU
development host: the AddressSanitizer runtime, LLVM's MSVC-only libFuzzer
Windows build and rustc's ELF-only coverage module all stop it there (the
toolchain and `cargo-fuzz` themselves install and run); the local evidence is
the deterministic mutation harness (`crates/vpng-fuzzing`). There is **no
graphical editor** yet. Each phase has a row in the status table.

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
