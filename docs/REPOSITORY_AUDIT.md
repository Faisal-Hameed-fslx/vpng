# Repository audit

**Audit date:** 2026-11-12
**Repository root:** `E:\VPNG`
**Audited by:** VPNG implementation agent (Phase 0, step 1 of the roadmap)

## 1. Prior state of the working directory

Before this phase the directory contained exactly one file:

| Path | Size | Meaning |
|------|-----:|---------|
| `VPNG_AGENT_MASTER_ROADMAP.md` | 47,202 bytes | The specification and roadmap. Authoritative for intent. |

There was no source code, no `docs/`, no version control, and no build output.
Nothing was overwritten or deleted during this phase.

## 2. Environment inventory

| Tool | Present | Version / path |
|------|:-------:|----------------|
| git | yes | 2.55.0.windows.3 |
| python | yes | 3.13.0 (`python` and `py`) |
| rustc | yes | 1.100.0 (0ef1bc386 2026-11-09), host `x86_64-pc-windows-gnu` |
| cargo | yes | 1.100.0 (3acc79c0c 2026-11-05) |
| rustup | yes | toolchain `stable-x86_64-pc-windows-gnu` (default, selected via `rust-toolchain.toml`) |
| MSVC `cl`/`link` | **no** | no Visual Studio Build Tools |
| mingw64 | yes | `C:\dev\toolchains\mingw64\bin` (`gcc`, `clang`, `ld`) |
| winget | **no** | not available |
| choco | **no** | not available |
| scoop | **no** | not available |

Network access to `static.rust-lang.org` is available (HTTP 200 verified), so a
toolchain can be installed when the user authorizes it.

### 2.1 Build environment (now reproducible)

The Rust toolchain is installed and the workspace compiles.

| Item | Value |
|------|-------|
| Installer | `rustup-init.exe` for `x86_64-pc-windows-gnu` |
| Install command | `rustup-init.exe -y --default-host x86_64-pc-windows-gnu --profile minimal --component rustfmt --component clippy` |
| Toolchain dir | `%USERPROFILE%\.rustup\toolchains\stable-x86_64-pc-windows-gnu` |
| Bundled linker | `rust-lld.exe` + `gcc-ld/` + `self-contained/` (no Visual Studio required) |
| Active toolchain | overridden by `E:\VPNG\rust-toolchain.toml` (channel `stable`) |

Because there is no MSVC linker, the GNU host is used. It needs no Visual Studio.

#### Registry mirror

`static.rust-lang.org` served the toolchain, but the crates.io registry download
from the same network path failed with `connection reset`. A sparse mirror was
configured in `%USERPROFILE%\.cargo\config.toml` (outside this repository):

```toml
[source.crates-io]
replace-with = 'rsproxy-sparse'

[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"

[net]
retry = 10
```

This is an environment workaround, not a project requirement. On a machine with
a reliable crates.io connection, no mirror is needed. The resolved dependency
set is pinned by `Cargo.lock`, so builds remain reproducible either way.

## 3. Workspace produced in this phase

A Rust workspace with thirteen crates, plus docs, tests, benchmark harnesses,
the corpus manifest, a Python package and a WebAssembly package. See
`docs/ROADMAP_STATUS.md` for the phase mapping and `docs/VPNG_SPEC_0_1.md`
for the format, revised through `docs/VPNG_SPEC_0_5.md`.

```text
vpng/ (root = E:\VPNG)
├── Cargo.toml                     workspace + shared dependency versions
├── rust-toolchain.toml            pinned channel and components
├── .gitignore
├── README.md  LICENSE  CHANGELOG.md
├── crates/
│   ├── vpng-core/                 formats, geometry, checked math, errors, limits
│   ├── vpng-container/            header, directory, layer table, checksums, canonical digest
│   ├── vpng-codecs/               TileCodec trait + Raw/Palette/RLE/Predictor codecs; ids 4-6 stable
│   ├── vpng-gpu/                  the GPU block codecs: BC1 colour blocks, BC4 alpha blocks
│   ├── vpng-tiling/               TileGrid, region-to-tile mapping
│   ├── vpng-image/                PNG and raw conversion: the one place that knows what a PNG means
│   ├── vpng-encoder/              canonical pixels -> .vpng bytes (flat or layered, with or without a GPU payload)
│   ├── vpng-decoder/              .vpng bytes -> full/tile/region/layer pixels + compositing + GPU payloads
│   ├── vpng-testkit/              deterministic fixtures + conformance tests
│   ├── vpng-python/               the PyO3 extension behind the Python package
│   ├── vpng-http/                 the HTTP Range source: a container fetched over the network
│   ├── vpng-wasm/                 the WebAssembly decoder: a raw `extern "C"` ABI, no wasm-bindgen
│   └── vpng-cli/                  encode/decode/info/check/verify/decode-region/extract-tile
│                                    + http-info/http-decode-region
│                                    + layers/decode-layer/composite/encode-layers
│                                    + gpu-info/gpu-decode
├── docs/                          spec, audit, status, threat model, ADRs
├── benchmarks/
│   ├── corpus_manifest.json       provenance, license, dimensions, SHA-256
│   ├── stress/                    large-image stress harness (Python + PowerShell)
│   ├── corpus/                    seed corpus assembler + benchmark runner
│   ├── python/                    the Python-binding measurement drivers
│   ├── http/                      the HTTP Range measurement driver
│   ├── wasm/                      the WebAssembly measurement driver
│   ├── layers/                    the layer-model memory driver
│   ├── gpu/                       the GPU-payload memory driver
│   └── results/                   git-ignored JSON/Markdown run output
├── corpus-data/                   git-ignored image bytes (provenance in the manifest)
├── python/                        the `vpng` package, its tests, and the experiment tooling
├── wasm/                          the JavaScript wrapper, the module's tests, its README
├── pyproject.toml                 the maturin build definition for the extension
```

## 4. Third-party dependencies

Chosen to be minimal, stable, and audited:

| Crate | Version | Used for | Rationale |
|-------|---------|----------|-----------|
| `crc32c` | 0.6 | header, directory, tile checksums | hardware-accelerated, tiny |
| `sha2` | 0.10 | canonical digest | the roadmap forbids hand-rolling SHA-256 |
| `png` | 0.17 | PNG I/O, in `vpng-image` | keeps the core free of image-format code |
| `pyo3` | 0.27 | the Python extension | the binding, not the engine; `abi3-py38` so one wheel covers every CPython from 3.8 |
| `ureq` | 3.2 | the HTTP Range source | a blocking client, so no async runtime enters the workspace; rustls, so no system OpenSSL is needed; pinned to 3.2 because 3.3 needs rustc 1.85 and the workspace's MSRV is 1.74 |

No dependency is used inside `vpng-core`, `vpng-codecs`, or `vpng-tiling`.

The WebAssembly package adds **none**: it is a raw `extern "C"` ABI over the
crates that are already here, so `cargo build --release --target
wasm32-unknown-unknown -p vpng-wasm` needs no `wasm-bindgen`, no code
generation and no extra toolchain.

The **GPU block codecs** add none either, for the same reason and a second
one: BC1 and BC4 live in their own crate (`crates/vpng-gpu`) rather than in
`vpng-codecs`, because that crate's own contract is that a codec is *exactly
reversible* and neither BC1 nor BC4 is. The crate needs no dependency at
all — the block arithmetic is the reference arithmetic, written out.

## 4.1 The test inventory (executed)

Command: `cargo test --workspace --all-targets -j 2`, release and debug
profiles, `x86_64-pc-windows-gnu`.

| | Tests | Result |
|---|---:|---|
| Rust workspace | **610** | **610 passed, 0 failed** |
| Python package (`python -m unittest discover -s python/tests`) | **62** | **OK** |
| WebAssembly package (`node --test wasm/test/vpng.test.mjs`) | **18** | **18 pass, 0 fail** |
| WebAssembly export check (`node wasm/check_exports.mjs`) | **47** exports | every documented export is really exported |

The per-suite breakdown is in `docs/ROADMAP_STATUS.md` §5.

## 5. Risks and unknowns carried into the next phase

Resolved during the build phase:

1. ~~**Nothing is compiled.**~~ Resolved: the workspace builds and 181 tests pass.
2. ~~**`png` crate API surface.**~~ Resolved: `png` resolved to 0.17.16 and
   `crates/vpng-cli/src/convert.rs` compiled on the first attempt, including
   `Reader::output_buffer_size`.
3. ~~**No MSVC.**~~ Resolved: the GNU host is installed and links with its
   bundled `rust-lld`/`self-contained` linker.

Still open:

4. ~~**Corpus coverage is thin.**~~ Resolved (2026-11-21): the corpus is
   **750 images** in five categories — photographs 300, screenshots 100, pixel
   art 100, scans 100, textures 150 — with a SHA-256 manifest that records
   each image's canonical digest, its source digest and the commit it was
   read at (`benchmarks/corpus_manifest.json` version 2,
   `docs/P5_CORPUS_REPORT.md`). The per-category distribution and the 10 MB
   success rate (748 of 750, 99.7%) are measured on it. What is *not* solved
   by this: the corpus is five repositories chosen for coverage rather than a
   random sample of the world's images, and only three of its images are
   larger than 4096 pixels, so the large-image tail is still thin.
5. **16K is partially validated.** A 16384×16384 gray8 gradient and a
   15360×8640 rgb8 photo are recorded; the 16384×16384 RGBA8 case was skipped
   because it needs about 3.4 GiB of live buffers and free physical memory never
   exceeded 1,712 MB. Full 16K coverage waits on P6 streaming.
6. **Registry reliability.** `static.rust-lang.org` served the toolchain while
   crates.io downloads from the same path were reset, which is why a mirror is
   configured. This affects developer onboarding, not the format.
7. **No coverage-guided fuzzing yet.** `malformed.rs` plus the deterministic
   `mutation_sweep.rs` (all truncations and single-byte substitutions over
   Raw/Palette/RLE files, 2,000 seeded multi-byte corruptions per file) exercise
   the parsers broadly and never panic, but this is not a substitute for
   `cargo-fuzz` coverage (P10).
