# VPNG — Vanta Portable Network Graphics

A lossless, adaptive, tiled, random-access image format and editing engine.

**Primary requirement:** exact pixel reconstruction.
**Secondary target:** fit eligible images within 10 MB — measured, never promised.
**Status:** v0.1 container implemented, compiled, and tested. `cargo build
--workspace` and `cargo test --workspace` both pass (122 tests, 0 failures) on
`stable-x86_64-pc-windows-gnu` rustc 1.100.0. See `docs/ROADMAP_STATUS.md`.

## What works today

* A precise, ADR-backed binary format: `docs/VPNG_SPEC_0_1.md`.
* All eight v0.1 pixel formats (Gray/GrayAlpha/RGB/RGBA at 8 and 16 bits).
* A validated container: 64-byte header, 40-byte directory entries, CRC32C
  checksums, whole-image canonical SHA-256 digest.
* Raw and Palette tile codecs, uniform tiling, per-tile codec selection, tile
  decode, full decode, and region decode.
* A CLI: `encode`, `decode`, `info`, `check`, `verify`, `decode-region`,
  `extract-tile`.
* Conformance tests: exhaustive round-trip, region-vs-crop, malformed-input
  sweeps, golden files. 122 tests, all passing.
* Layered integrity: `info` (structure only), `check` (every payload CRC32C),
  `verify` (exact pixels against a reference).

## What is explicitly not here yet

RLE/predictor/dictionary/entropy codecs, adaptive tiling, the tile cache,
parallel decode, the editing engine, previews, 16K stress, the Python bindings,
and fuzzing. Each has a phase in the roadmap and a row in the status table.

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
vpng encode input.png out.vpng --codec palette      # force one codec (auto by default)
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
crates/vpng-codecs      TileCodec trait + Raw and Palette codecs (reserved ids 2-6)
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
