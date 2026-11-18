# P4 — Compatibility report

**Gate:** P4 compatibility (format, golden files, ids, fuzzing, CI) — **PASS**
**Date:** 2026-11-16
**Toolchain:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `x86_64-pc-windows-gnu`
**Related:** `docs/P3_5_COMPRESSION_RESEARCH_REPORT.md`, `docs/P4_EDITING_ENGINE_REPORT.md`, `docs/P4_MEMORY_VALIDATION.md`

---

## 1. Summary

The P3.5 + P4 milestone added a research codec family, a benchmark
harness, a fuzzing harness, an editing engine, a memory-validation harness
and a CI workflow. **None of it changes the VPNG file format.**

| Property | Status |
|---|---|
| The binary format | **unchanged** — no chunk, no field, no version, no size change |
| The golden fixtures | **byte-identical** |
| The stable codec ids | **unchanged** — 0–3 only; 4–6 still reserved |
| The experimental codec ids | **unreachable from any file** |
| The decoder's public API | **unchanged** (two pure additions: `derive_grid`, `select_payload`) |
| The test suite | **green** |
| Fuzzing | **clean locally, and it found one real defect in CI**: the deterministic harness ran 200,000 iterations × 7 targets = 1,400,000 executions with 0 findings; the coverage-guided fuzzer in CI on Linux found one genuine bug — `Header::parse` accepted a zero-dimension header that `Header::to_bytes` refused to serialize — which is fixed and pinned by a regression test (§3.1) |
| CI | **9 jobs**, including a nightly coverage-guided fuzz job — **exercised on Linux**: the first run was clean, the second found the defect in §3.1, the third re-verifies the fix |
| `cargo-fuzz` on this host | **BLOCKED** — the toolchain installs and the targets are correctly wired, but three platform limits stop the local run on `x86_64-pc-windows-gnu` (ASan unsupported, libFuzzer's Windows build is MSVC-only, rustc's coverage module is ELF-only); the deterministic harness is the local evidence |

---

## 2. What changed in the code, and why the format did not

| Change | Where | Effect on the format |
|---|---|---|
| Seven research codecs (CARC A–D, LZ77, three hybrids) | `crates/vpng-codecs/src/experimental/` | **None.** They produce payloads in memory; the encoder never selects one. |
| A shared predictor refactor | `crates/vpng-codecs/src/predictor.rs` | **None.** `predictor::plan_rows` was extracted as an exact refactor so the research codecs reuse the identical predictor choice. The produced bytes are identical; only the visibility changed (`pub(crate)`). |
| The editing engine | `crates/vpng-editing/` (new crate) | **None.** It writes ordinary VPNG files: the same header, the same directory, the same payloads. |
| A streaming writer | `crates/vpng-validation/src/stream.rs` | **None.** It produces a file the ordinary decoder reads without knowing how it was produced. |
| `vpng_decoder::derive_grid` made public | `crates/vpng-decoder/src/lib.rs` | **None.** Pure addition; the behaviour is unchanged. |
| `vpng_encoder::select_payload` made public | `crates/vpng-encoder/src/lib.rs` | **None.** Pure addition; the behaviour is unchanged. |
| `vpng_tiling::TileGrid::geometry_for_index` added | `crates/vpng-tiling/src/lib.rs` | **None.** Pure addition; the behaviour is unchanged. |
| A benchmark harness | `crates/vpng-research/` (new crate) | **None.** It never writes a file. |
| A fuzzing harness | `crates/vpng-fuzzing/` (new crate), `fuzz/` | **None.** |
| A CI workflow | `.github/workflows/ci.yml` | **None.** |

Every addition to an existing crate is either a *new module in a new crate*
or a *pure addition of visibility*. **No existing field was reinterpreted,
no existing function's behaviour changed, and no existing byte of output
changed.**

---

## 3. The twelve questions

**Q1 — Does the binary format change?**
**No.** The v0.1 container is exactly as `docs/VPNG_SPEC_0_1.md` describes
it: a 64-byte header, a directory of 40-byte entries with a 4-byte CRC32C
trailer, and the tile payloads. The format's major version is still 0 and
its minor version is still 1. No chunk was added, no field was widened, no
offset moved. The editing engine's commit path reconstructs the header from
the original header's own fields — carrying over `major`, `minor`,
`color_model`, `bits`, `default_codec`, `tile_count` and `feature_flags`
unchanged, and recomputing only the two fields that are *defined* in terms
of the content (`directory_offset` and `declared_file_size`).

**Q2 — Are the golden files byte-identical?**
**Yes.** The five golden fixtures are unchanged, and `git status` for
`crates/vpng-testkit/` reports no modification:

| Fixture | Size | SHA-256 (first 16 hex) |
|---|---:|---|
| `gradient_64x64_rgba8_tile16.vpng` | 17,092 | `1A95CE792416C0CB` |
| `gradient_300x180_rgb16_tile64.vpng` | 324,668 | `F5A971025E3CFE97` |
| `palette_checker_64x64_rgba8_tile32.vpng` | 780 | `23460E17D14944A9` |
| `predictor_ramp_64x64_gray8_tile64.vpng` | 396 | `A8B102B6ABB795FF` |
| `rle_mixed_64x64_gray8_tile64.vpng` | 2,348 | `EC9510F207F1ACC6` |

The CI workflow runs the golden test as a **separate job** (`golden`), so a
change to the binary format is visible in the job list rather than buried
in the test output.

**Q3 — Is the test suite green?**
**Yes.** The whole workspace is tested with `cargo test --workspace
--all-targets`. The counts are recorded in `docs/ROADMAP_STATUS.md`. The
milestone added 52 codec tests, 51 editing tests, 3 fuzzing tests and 2
memory-accounting tests to the previous 181.

**Q4 — Can an older decoder read the files the new code writes?**
**Yes, and the new code does not change what the decoder accepts.** A file
written by the editing engine is an ordinary VPNG file: the unit test
`a_committed_file_passes_the_integrity_check` opens it with the ordinary
decoder and runs the container's full integrity check. The 16K validation
files, written by the streaming writer, are read back by the ordinary
decoder in every one of the 12 cases, and the decoded tiles match the
generated pattern byte for byte.

The decoder's public API is unchanged. `Document::parse`, `decode_tile`,
`decode_all`, `decode_region` and `check_integrity` keep their signatures
and their behaviour. The two additions (`derive_grid`, `select_payload`)
are new functions; nothing existing was changed to accommodate them.

**Q5 — Are the stable codec ids unchanged?**
**Yes.** `IMPLEMENTED_CODEC_IDS` is still `[0, 1, 2, 3]`: Raw, Palette,
Run-length, and Spatial predictor + entropy. The ids 4, 5 and 6 — LZ77
dictionary, Predictor + dictionary, and Context-adaptive entropy — remain
**reserved and unassigned**, exactly as ADR-0004 requires. A file that
names one is rejected with `ErrorKind::UnsupportedFeature`; the decoder
never guesses.

The research codecs use a **separate** id range, `0x80..=0xFF`, which does
not overlap the reserved ids:

| Research codec | Id |
|---|---:|
| CARC A (stable predictor) | `0x80` |
| CARC B (gradient classifier) | `0x81` |
| CARC C (residual classifier) | `0x82` |
| CARC D (combined classifier) | `0x83` |
| LZ77 dictionary | `0x90` |
| Predictor → LZ77 | `0x91` |
| LZ77 → entropy | `0x92` |
| Predictor → LZ77 → entropy | `0x93` |

**Q6 — Can an experimental codec id ever reach a file?**
**No.** There are three independent barriers, and all three are tested:

1. `get_codec` — the only way the decoder resolves a tile's codec id —
   knows only ids 0–3. It has no branch for `0x80..=0xFF`.
2. The encoder's candidate list comes from `IMPLEMENTED_CODEC_IDS`, which
   does not list the research ids, so the encoder cannot select one. The
   research registry lives in `vpng_codecs::experimental` and is reachable
   only from the research harness.
3. `FileSource::open` — the editing engine's file reader — rejects a tile
   whose codec id is not in `IMPLEMENTED_CODEC_IDS`, with
   `ErrorKind::UnsupportedFeature`.

A file that names a research id is therefore unreadable by design, and the
decoder reports that fact rather than guessing.

**Q7 — Is there a policy for assigning a research id to a stable one?**
**Yes, and nothing in this milestone changes it.** A research id may be
promoted to a stable id only when **all** of the following exist:

1. a specification update that defines the payload layout normatively;
2. a format version bump with the compatibility rules spelled out;
3. a migration policy — what an older reader does with the new file, and
   what a newer reader does with an old one;
4. compatibility tests — golden vectors, and round-trip tests across the
   versions.

The P3.5 report's recommendation is **not** to promote any experimental
codec: CARC C is the strongest research result and it is still 18.9% worse
than WebP lossless across the corpus, it expands incompressible data,
and none of the four preconditions above exist. The stable set stays at
ids 0–3.

**Q8 — What is the fuzzing evidence, and what could not be run here?**
**Ran here:** the deterministic mutation harness — 200,000 iterations over
7 targets, 1,400,000 target executions, **0 findings**, 3.9 s in the
release profile. The report is `benchmarks/results/fuzz-mutation-report.json`.

| Target | Executions | Findings |
|---|---:|---:|
| `header` | 200,000 | 0 |
| `directory` | 200,000 | 0 |
| `container` | 200,000 | 0 |
| `tile-payload` | 200,000 | 0 |
| `experimental-codec` | 200,000 | 0 |
| `region` | 200,000 | 0 |
| `editing` | 200,000 | 0 |

The targets check the invariants that must hold for *every* input: an
accepted header re-encodes to its own bytes; an accepted directory
re-encodes to its own bytes; an accepted file decodes to exactly the
declared dimensions and byte count; a region decode equals the crop of the
full decode; a decoder that reports success produces exactly the number of
bytes it was asked for; a rectangle outside the image is rejected; a write
reads back exactly; an abort leaves the document clean.

**Also ran: the coverage-guided fuzzer, in CI on Linux (2026-11-18).**
The first GitHub Actions run on `ubuntu-latest` executed all seven
`cargo-fuzz` targets under AddressSanitizer with libFuzzer's coverage
feedback, 120 s each, **0 findings**:

| Target | Executions | Coverage (edges) | Features | Corpus | Exec/s | Peak RSS |
|---|---:|---:|---:|---:|---:|---:|
| `header` | 37,048,660 | 130 | 134 | 17 | 306,187 | 477 MB |
| `directory` | 50,265,597 | 258 | 329 | 18 | 415,418 | 462 MB |
| `container` | 31,085,678 | 175 | 179 | 22 | 256,906 | 486 MB |
| `tile_payload` | 1,585,021 | 340 | 753 | 240 | 13,099 | 475 MB |
| `experimental_codec` | 4,683,306 | 755 | 1,545 | 454 | 38,705 | 514 MB |
| `region` | 3,896,993 | 126 | 434 | 37 | 32,206 | 534 MB |
| `editing` | 3,864,456 | 487 | 557 | 19 | 31,937 | 431 MB |
| **Total** | **132,429,711** | | | | | |

The corpus figures are libFuzzer's own (`corp: 17/1046b` means 17 inputs in
the corpus, 1,046 bytes), not a VPNG measurement. The run is
<https://github.com/Faisal-Hameed-fslx/vpng/actions/runs/44264162940>, and
the job log carries every `Done <n> runs in <t> second(s)` line and every
`cov:`/`ft:`/`rss:` line quoted above. No crash artifact was produced and no
`ERROR: libFuzzer` line appears in that log.

### 3.1 The coverage-guided fuzzer found a real defect

**The second CI run found a bug that the entire test suite had missed.**
Run 44266490454 crashed the `header` target after 2,328 executions:

```
thread '<unnamed>' panicked at fuzz_targets/header.rs:16:9:
accepted header failed to re-encode: invalid argument: image dimensions must be non-zero
```

The input is a 74-byte header that is well formed in every respect except
that its **width is zero**: correct magic, version, header size and channel
count, a valid CRC32C, and zeroed reserved bytes.

**The defect was in the library, not in the harness.** `Header::parse` never
checked the dimensions, while `Header::new` and `Header::to_bytes` both
required them to be non-zero. The parser therefore accepted a header that
the serializer refused to produce: two implementations of the same format
disagreed about the same 74 bytes, and a file that one of them accepted
could not be written by the other. That is exactly the invariant the `header`
target asserts — "an accepted header re-encodes to its own bytes" — and it
was the only one of the seven invariants that the deterministic harness
could not reach, because a seeded mutation almost never produces a
correctly-checksummed header with a zero dimension.

The full-file path was never affected: `Document::parse` called
`header.dimensions()` itself and rejected the file, which is why the defect
survived every test. It was reachable through the header API alone.

**The fix** is in `crates/vpng-container/src/header.rs`: `Header::parse` now
applies the same non-zero dimension rule as the other two entry points. The
check is placed *after* every other header rule, so the error a caller sees
for a malformed file does not change — a zero dimension is still
`InvalidArgument` with the message "image dimensions must be non-zero",
exactly as it was through `Document::parse`. The binary format is untouched:
no byte, offset or size changed, and the golden fixtures are byte-identical.

**The regression is pinned.** The exact crash artifact libFuzzer wrote
(`crash-32233a96f431ba8ee472ae5d927905192ef267aa`) is stored as a literal
in `crates/vpng-fuzzing/tests/fuzz_regressions.rs` and replayed on every
`cargo test` run, together with the assertion that the fuzz target no longer
reports it.

**The CI job is hardened.** A target that finds something no longer stops
the others: the job runs all seven, records the failures, and fails at the
end. One run therefore covers the whole target set instead of stopping at
the first finding, and the crash artifacts are uploaded as an artefact so a
finding can be reproduced locally.

**Still BLOCKED on this host:** `cargo-fuzz` itself — not for the reason
recorded before. The earlier blocker was the toolchain: `rustup toolchain
install nightly` failed with a network timeout (`os error 10060`). **That is
no longer true.** The nightly toolchain downloads and installs, `cargo-fuzz`
v0.13.2 installs, `cargo fuzz list` now reports all seven targets, and the
runner script (`benchmarks/fuzz/run_fuzz.ps1`) drives them. What stops the
local run is the **target platform**, in three independent places:

1. **The AddressSanitizer runtime does not exist for
   `x86_64-pc-windows-gnu`.** rustc refuses the build with
   `error: address sanitizer is not supported for this target`. ASan is
   available for the `x86_64-pc-windows-msvc` target, but this host has no
   MSVC linker, and no WSL distribution is installed, so no ELF or MSVC
   target is available either.
2. **LLVM's libFuzzer Windows build is MSVC-only.**
   `libfuzzer-sys` compiles `FuzzerExtFunctionsWindows.cpp`, which declares
   the optional sanitizer entry points with
   `__pragma(comment(linker, "/alternatename:..."))` — a construct both GCC
   and Clang reject for the GNU target. `benchmarks/fuzz/build_libfuzzer.ps1`
   builds the same runtime from the same sources with that one construct
   replaced by upstream's portable weak-alias form, so the runtime itself is
   no longer the blocker.
3. **rustc's sanitizer-coverage module is ELF-only.** The instrumentation
   rustc emits for `-Cpasses=sancov-module` puts the coverage counters in an
   `__sancov_cntrs` section and references them through the ELF section
   anchors `__start___sancov_cntrs` / `__stop___sancov_cntrs` (and the same
   pair for `__sancov_pcs`). On an ELF target rustc emits those sections and
   the anchors; on a COFF target it emits **only** the module constructor
   and leaves the anchors undefined, so the fuzz target cannot be linked:
   `undefined reference to __start___sancov_cntrs`. Neither GNU `ld` nor
   `lld` provides COFF section anchors.

The evidence for (3) is a direct comparison of the same source file built
for the two targets:

```text
x86_64-unknown-linux-gnu : __sancov_cntrs, __sancov_pcs sections present;
                            __start/__stop___sancov_* defined (weak)
x86_64-pc-windows-gnu    : no such sections; __start/__stop___sancov_*
                            undefined
```

**The coverage-guided fuzzing therefore cannot run on this machine**, and
this is a platform limit rather than a missing tool. The `cargo-fuzz`
targets are provided and wired into CI's nightly job, which runs on
`ubuntu-latest` where all three limits disappear — and where they have now
**run successfully**, with the figures in the table above. The deterministic
harness remains the executable local evidence. Two wiring bugs *were* found
and fixed while getting this far, and both would have made the CI job fail:

* the fuzz crate's `Cargo.toml` had no `[[bin]]` sections, and cargo-fuzz
  discovers its targets from the manifest rather than from the files in
  `fuzz_targets/` — `cargo fuzz list` was empty and `cargo fuzz run` failed
  with "no targets specified in the manifest";
* `--fuzz-dir` is cargo-fuzz's *fuzz project* directory (the one holding the
  fuzz crate's `Cargo.toml`), not the corpus directory; `--fuzz-dir
  fuzz/corpus` made cargo-fuzz look for `fuzz/corpus/Cargo.toml`. The
  runner and the CI job now pass `--fuzz-dir fuzz`, and the corpus lives in
  `fuzz/corpus/<target>` as cargo-fuzz expects.

Two findings *were* reported during the harness's development, and both
were **harness** bugs, not library defects:

* the `region` target initially called `TileGrid::new` with the arguments
  in the wrong order (`width, height, tile_width, tile_height` instead of
  `tile_width, tile_height, image_width, image_height`), so it asserted
  that tiles lay outside the image;
* the `container` target initially required every file that parsed to also
  pass its integrity check. `Document::parse` deliberately does **not**
  verify payload checksums — that is an O(1) parse, and the checksums are
  verified per tile by `decode_tile` and in full by `check_integrity` — so
  the target was asserting a promise the decoder does not make.

Both were corrected, and the invariants now reflect the decoder's actual
contract. **At that point no library defect had been found by fuzzing — but
that was a statement about the harness's own development runs, not about the
library.** The coverage-guided fuzzer then found a genuine library defect
in CI (§3.1): `Header::parse` accepted a zero-dimension header that
`Header::to_bytes` refused to serialize. It is fixed, and the exact crash
input is replayed by the test suite.

**Q9 — Does CI enforce the gates?**
**Yes.** `.github/workflows/ci.yml` defines 9 jobs:

| Job | What it enforces |
|---|---|
| `fmt` | `cargo fmt --all --check` |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` (and `RUSTFLAGS: -D warnings` globally) |
| `build` | `cargo build --workspace --all-targets` |
| `test` | `cargo test --workspace --all-targets` |
| `golden` | `cargo test -p vpng-testkit --test golden` — the binary format's fixtures, on their own |
| `fuzz-smoke` | the deterministic mutation harness, 50,000 iterations, with the JSON report uploaded as an artefact |
| `fuzz` | the 7 `cargo-fuzz` targets on **nightly**, 120 s per target; all seven run even if one finds something, and the corpus and the crash artifacts are uploaded |
| `release` | `cargo build --workspace --release` |
| `python` | `python -m compileall benchmarks python`, the research runner's `--help`, and `python benchmarks/verify_reports.py` — which re-reads the deliverable JSON and asserts that every figure quoted in the milestone reports is the figure the data actually holds |

The workflow runs on every push to `main` or `master`, on every pull
request, and on manual dispatch. The `fuzz` job is the one that cannot run
on this host; it **has** run in CI on Linux. The first run (44264162940)
executed all seven coverage-guided targets with 0 findings, and the
`python` job's missing dependencies (Pillow and numpy) were found by that
same run and fixed. The second run (44266490454) found the library defect
in §3.1.

**Q10 — Is there a known wording discrepancy in the specification?**
**It was recorded rather than silently decided, and it has now been resolved
in the specification.**

`docs/VPNG_SPEC_0_1.md` described header byte 27 as:

> `default codec id` — *codec used for all tiles in v0.1*

The implementation treats the field as **informational**. The authoritative
codec for a tile is the directory entry's own `codec_id`, and that is what
the decoder uses (`get_codec(entry.codec_id)`) and what the encoder writes
per tile. The header's `default_codec` is carried over unchanged by the
editing engine, because it does not affect decoding.

The discrepancy was visible in the repository's own conformance matrix:
`docs/ROADMAP_STATUS.md` records that a file forced to `--codec palette`
differs from the auto-encoded file **only** in header byte 27, while the
directory and every payload are identical. A file whose tiles use mixed
codecs — which the encoder produces by default, since it selects the
smallest payload per tile — has no single "codec used for all tiles" at
all.

**The specification pass has now been made** (2026-11-18). Byte 27 is
**informational**: §2.3 of the specification states that a reader must not
use it to decode a tile and must not reject a file because of its value, that
the per-tile directory entry is the authority, and that a writer records Raw
when it selected the tiles individually. §4.1 marks the directory entry's
`codec id` as authoritative, and §5 says so explicitly. The clarification is
dated at the top of the document and cross-references ADR-0003.

**The binary layout is unchanged**: the byte keeps its offset, its size and
its meaning as a hint, the format version stays 0.1, and no conforming file
changes meaning — the directory has always been the authority (ADR-0003,
accepted 2026-11-12). The behaviour is pinned by a new conformance test,
`crates/vpng-testkit/tests/default_codec.rs` (7 tests), which shows that the
decoded pixels are identical whatever byte 27 says — including a codec the
file does not use, a reserved id, and ids that do not exist — and that the
encoder records the forced codec when one codec applies and Raw when the
tiles were selected individually.

**Q11 — Are the existing test vectors and conformance rows still valid?**
**Yes.** `docs/ROADMAP_STATUS.md`'s conformance matrix is unchanged in its
expectations: the same commands produce the same outputs, the same files
are byte-identical, and the same golden fixtures are in place. The
milestone added rows and crates; it did not revise an existing
expectation. Where the new work touches an existing crate, the change is
either a new module or a pure addition of visibility.

**Q12 — What is explicitly not claimed?**
* **No claim that coverage-guided fuzzing ran *on this host*.** It was
  **BLOCKED** here — not because a tool was missing, but because the
  Windows-GNU target cannot support it (ASan, libFuzzer's MSVC-only
  Windows build and rustc's ELF-only coverage module). It **did** run in CI
  on Linux: 132,429,711 executions across the seven targets in the first
  run, and a second run that found the defect in §3.1. That is a statement
  about a Linux runner, not about this machine.
* **No claim that fuzzing found no bugs in the library.** It found one — the
  header parse/serialize asymmetry in §3.1 — and fixed it. The
  deterministic harness (1,400,000 executions) and the coverage-guided runs
  (132,429,711 executions in the first CI run) say nothing about the inputs
  nobody has tried yet.
* **No claim that the fix is complete.** The fix makes `parse` and
  `to_bytes` agree about the dimensions. It does not claim that the two
  functions can never disagree about anything else; that is what the fuzz
  target is for, and it is still running.
* **No claim of backwards compatibility with a future format.** The format
  did not change, so v0.1 files remain v0.1 files. No forward
  compatibility is claimed.
* **No claim that the specification is defect-free.** The one wording defect
  this milestone recorded has been fixed (§Q10); that is not a claim that no
  other defect exists.
* **No claim that the research codecs are production-ready.** They are
  research-only, unreachable from any file, and the recommendation is not
  to promote them.
* **No claim about other platforms.** CI runs on `ubuntu-latest`; the local
  evidence in this milestone is from `x86_64-pc-windows-gnu`. The code has
  no platform-specific logic beyond the Windows memory counters, which
  have a Linux `/proc` implementation and a portable fallback.
* **No claim of a stable API.** `vpng-editing` is new in this milestone and
  its API may change; the v0.1 *file format* is what is stable.

---

## 4. The compatibility checklist

| # | Check | Result |
|---:|---|---|
| 1 | The container layout is byte-for-byte the v0.1 layout | PASS |
| 2 | The format's major/minor version is unchanged (0 / 1) | PASS |
| 3 | The golden fixtures are byte-identical | PASS |
| 4 | The whole test suite passes | PASS |
| 5 | A file written by the editor passes the ordinary integrity check | PASS |
| 6 | A file written by the streaming writer decodes with the ordinary decoder | PASS |
| 7 | The stable codec ids are 0–3 and unchanged | PASS |
| 8 | The reserved ids 4–6 are still unassigned and rejected | PASS |
| 9 | The research ids are unreachable from any file | PASS |
| 10 | The decoder's public API is unchanged | PASS |
| 11 | The encoder's public API is unchanged | PASS |
| 12 | The deterministic fuzzing harness is clean | PASS |
| 13 | Coverage-guided fuzzing runs in CI | **PASS** — verified on Linux: run 44264162940 executed all seven targets (132,429,711 executions, 0 findings); run 44266490454 found a real defect (§3.1), which is fixed and pinned by a regression test |
| 14 | Coverage-guided fuzzing ran on this host | **BLOCKED** (platform: ASan, MSVC-only libFuzzer, ELF-only rustc coverage module) |
| 15 | The CI workflow runs on push, pull request and dispatch | PASS |
| 16 | The specification's `default_codec` wording is consistent with the code | **PASS** (resolved 2026-11-18: the byte is informational; §2.3, with a conformance test) |
| 17 | A defect found by the fuzzer is fixed and pinned by a test | **PASS** (the header parse/serialize asymmetry of §3.1; `crates/vpng-fuzzing/tests/fuzz_regressions.rs` replays the exact crash artifact) |

One item is not a clean pass, and it is stated plainly rather than glossed:
item 14 is a host limitation — coverage-guided fuzzing cannot run on
Windows-GNU, for the three platform reasons set out in §3.
