# Changelog

All notable changes to VPNG are recorded here. The format follows Keep a
Changelog; versions follow semantic versioning, with the format version tracked
separately as major.minor in the header.

## [Unreleased]

### Added

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

### Measured

Re-measured on the 9-image seed corpus (2026-11-15, same host and corpus as
the 2026-11-14 run; see `docs/BENCHMARKS.md` for the full table):

* The large photo (15360×8640 rgb8) drops from **0.774×** to **0.312×** of its
  canonical bytes, and the cat photograph from 0.830× to 0.509×.
* Auto selection was again no worse than the best forced codec on all 9
  images, and it now beats forced predictor on the large photo by 2,588 bytes
  by mixing in Palette for two tiles.
* The noise image still falls back to Raw at 1.000× even though a forced
  predictor run expands to 1.016×.

### Known limitations

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
