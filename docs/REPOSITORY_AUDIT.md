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

A Rust workspace with eight crates, plus docs, tests, a benchmark manifest, and
a Python experiment skeleton. See `docs/ROADMAP_STATUS.md` for the phase mapping
and `docs/VPNG_SPEC_0_1.md` for the format.

```text
vpng/ (root = E:\VPNG)
├── Cargo.toml                     workspace + shared dependency versions
├── rust-toolchain.toml            pinned channel and components
├── .gitignore
├── README.md  LICENSE  CHANGELOG.md
├── crates/
│   ├── vpng-core/                 formats, geometry, checked math, errors, limits
│   ├── vpng-container/            header, directory, checksums, canonical digest
│   ├── vpng-codecs/               TileCodec trait + Raw codec; reserved ids
│   ├── vpng-tiling/               TileGrid, region-to-tile mapping
│   ├── vpng-encoder/              canonical pixels -> .vpng bytes
│   ├── vpng-decoder/              .vpng bytes -> full/tile/region pixels
│   ├── vpng-testkit/              deterministic fixtures + conformance tests
│   └── vpng-cli/                  encode/decode/info/verify/decode-region/extract-tile
├── docs/                          spec, audit, status, threat model, ADRs
├── benchmarks/corpus_manifest.json
└── python/                        experiment + corpus + tools skeleton
```

## 4. Third-party dependencies

Chosen to be minimal, stable, and audited:

| Crate | Version | Used for | Rationale |
|-------|---------|----------|-----------|
| `crc32c` | 0.6 | header, directory, tile checksums | hardware-accelerated, tiny |
| `sha2` | 0.10 | canonical digest | the roadmap forbids hand-rolling SHA-256 |
| `png` | 0.17 | CLI PNG I/O only | keeps the core free of image-format code |

No dependency is used inside `vpng-core`, `vpng-codecs`, or `vpng-tiling`.

## 5. Risks and unknowns carried into the next phase

Resolved during the build phase:

1. ~~**Nothing is compiled.**~~ Resolved: the workspace builds and 99 tests pass.
2. ~~**`png` crate API surface.**~~ Resolved: `png` resolved to 0.17.16 and
   `crates/vpng-cli/src/convert.rs` compiled on the first attempt, including
   `Reader::output_buffer_size`.
3. ~~**No MSVC.**~~ Resolved: the GNU host is installed and links with its
   bundled `rust-lld`/`self-contained` linker.

Still open:

4. **No corpus.** Compression behavior and the 10 MB target cannot be evaluated
   without one; only overhead and Raw round-trips are measurable today.
5. **16K is unvalidated.** No 16K run has been recorded; `Limits` permits it,
   but permission is not evidence.
6. **Registry reliability.** `static.rust-lang.org` served the toolchain while
   crates.io downloads from the same path were reset, which is why a mirror is
   configured. This affects developer onboarding, not the format.
7. **No fuzzing yet.** The malformed-input sweeps are deterministic and broad
   but are not a substitute for `cargo-fuzz` coverage (P10).
