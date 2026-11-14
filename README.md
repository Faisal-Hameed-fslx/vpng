# VPNG — Vanta Portable Network Graphics

A lossless, adaptive, tiled, random-access image format and editing engine.

**Primary requirement:** exact pixel reconstruction.
**Secondary target:** fit eligible images within 10 MB — measured, never promised.
**Status:** v0.1 container implemented, compiled, and tested. `cargo build
--workspace` and `cargo test --workspace` both pass (148 tests, 0 failures) on
`stable-x86_64-pc-windows-gnu` rustc 1.100.0. Large-image stress runs and a
seed-corpus measurement are recorded in `docs/BENCHMARKS.md`. See
`docs/ROADMAP_STATUS.md`.

## What works today

* A precise, ADR-backed binary format: `docs/VPNG_SPEC_0_1.md`.
* All eight v0.1 pixel formats (Gray/GrayAlpha/RGB/RGBA at 8 and 16 bits).
* A validated container: 64-byte header, 40-byte directory entries, CRC32C
  checksums, whole-image canonical SHA-256 digest.
* Raw, Palette, and Run-length tile codecs, uniform tiling, per-tile codec
  selection, tile decode, full decode, and region decode.
* A CLI: `encode`, `decode`, `info`, `check`, `verify`, `decode-region`,
  `extract-tile`.
* Conformance tests: exhaustive round-trip, region-vs-crop, malformed-input
  sweeps, a bounded mutation sweep, golden files. 148 tests, all passing.
* Layered integrity: `info` (structure only), `check` (every payload CRC32C),
  `verify` (exact pixels against a reference).
* Measured evidence: large-image stress (up to a 15360×8640 photo and
  16384×16384 pixels) with sampled peak RSS and exactness checks, plus a 9-image
  seed corpus compared against PNG and WebP lossless. See `docs/BENCHMARKS.md`.

## What is explicitly not here yet

Predictor/dictionary/entropy codecs, adaptive tiling, the tile cache, parallel
decode, streaming, the editing engine, previews, the Python bindings, and
coverage-guided fuzzing. A deterministic mutation sweep
(`crates/vpng-testkit/tests/mutation_sweep.rs`) covers truncations and
single-byte substitutions over Raw, Palette, and Run-length files, but it is not
a substitute for `cargo-fuzz`. Each phase has a row in the status table.

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
vpng encode input.png out.vpng --codec rle         # raw | palette | rle | auto (default)
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
crates/vpng-codecs      TileCodec trait + Raw, Palette, Run-length (ids 0-2; 3-6 reserved)
crates/vpng-tiling      TileGrid and region-to-tile mapping
crates/vpng-encoder     canonical pixels -> .vpng bytes
crates/vpng-decoder     .vpng bytes -> full / tile / region pixels
crates/vpng-testkit     deterministic fixtures + conformance tests
crates/vpng-cli         the `vpng` binary
docs/                   spec, audit, status, threat model, ADRs
```

## Claims policy

"Lossless" is used only for verified paths. "16K supported" only after measured
tests at a stated format. "Under 10 MB" only for a named image or reported corpus
subset. "Faster" only against named baselines, hardware, and settings. See
`docs/BENCHMARKS.md`.

## License

MIT. See `LICENSE`.
