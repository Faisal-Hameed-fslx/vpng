# Changelog

All notable changes to VPNG are recorded here. The format follows Keep a
Changelog; versions follow semantic versioning, with the format version tracked
separately as major.minor in the header.

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
