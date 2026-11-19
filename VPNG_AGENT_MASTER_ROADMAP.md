# VPNG --- Vanta Portable Network Graphics

## Autonomous Agent Master Specification, Implementation Roadmap, and Acceptance Contract

**Document:** `VPNG_AGENT_MASTER_ROADMAP.md`\
**Specification baseline:** VPNG container v0.1 proposal; roadmap
revision 0.2\
**Project type:** Lossless, adaptive, tiled, random-access image format
and editing engine\
**Primary extension:** `.vpng`\
**Primary implementation language:** Rust\
**Experimentation and tooling:** Python\
**Primary requirement:** Exact pixel reconstruction\
**Secondary target:** Fit eligible images within 10 MB\
**Runtime target:** Bounded-memory streaming and localized editing\
**Status:** Research and implementation specification; all performance
figures below are targets until measured.

------------------------------------------------------------------------

# 0. Instructions to the autonomous coding agent

You are the lead systems architect, codec engineer, Rust implementer,
test engineer, benchmark engineer, and release engineer for VPNG.

Your task is to build a real, reproducible, secure, well-tested
implementation of VPNG. Do not merely write a design document, create
mock APIs, or claim that features work without executable tests.

## 0.1 Non-negotiable principles

1.  **Losslessness is absolute.** For every supported input, decoding
    must reproduce the exact canonical pixel buffer that was encoded. No
    quantization, chroma subsampling, neural reconstruction,
    approximation, hidden downscaling, or perceptual substitution is
    permitted in the lossless path.
2.  **The 10 MB target is not a universal guarantee.** The encoder must
    report when an image cannot fit. It must never discard information
    to meet the limit.
3.  **Resolution is adaptive.** Support configurable dimensions subject
    to safe resource limits. The format must not assume that every image
    is 16K.
4.  **Random access is a core requirement.** A decoder must be able to
    decode an independently addressable tile without decoding the entire
    image.
5.  **Editing must be localized.** Editing one region must not require
    decoding and recompressing the entire image.
6.  **Memory use must be bounded and measurable.** Streaming mode must
    not allocate a full-image pixel buffer unless the caller explicitly
    requests full decode.
7.  **Determinism matters.** Given identical input and encoder settings,
    the deterministic mode must produce byte-identical output across
    supported platforms and thread counts, unless a future specification
    explicitly defines otherwise.
8.  **Never trust file contents.** Validate all dimensions, offsets,
    lengths, arithmetic, codec IDs, directory entries, chunk boundaries,
    and allocation requests before use.
9.  **Preserve evidence.** Keep raw benchmark data, test logs, corpus
    manifests, and exact toolchain versions.
10. **Do not silently weaken acceptance criteria.** If a gate fails,
    report the failure, root cause, and proposed corrective experiment.

## 0.2 Work style

-   First inspect the existing repository and environment. Do not
    overwrite an existing project.
-   Produce a concise implementation plan based on the actual repository
    state.
-   Implement in small, reviewable phases, but maintain the complete
    end-to-end roadmap.
-   Run tests after each meaningful change.
-   Do not claim an acceptance gate passes until its test is implemented
    and executed.
-   Prefer established, audited primitives over inventing cryptographic
    or low-level entropy-coding algorithms.
-   Keep the format specification independent of implementation details.
-   Maintain compatibility tests using golden files.
-   If a requested feature cannot be implemented safely within the
    current phase, document it and leave a typed, explicit unsupported
    result rather than a fake implementation.

## 0.3 Required final report for every phase

At the end of each phase, provide:

-   Changes made.
-   Files added/modified.
-   Exact commands executed.
-   Test counts and pass/fail results.
-   Benchmark measurements, where applicable.
-   Memory measurements, where applicable.
-   Known limitations.
-   Acceptance gates passed and failed.
-   Next phase recommendation.

------------------------------------------------------------------------

# 1. Product definition

VPNG is a binary image container with a lossless codec and a
random-access editing model.

It combines:

-   An image metadata/header layer.
-   A tile index.
-   Independently decodable tile payloads.
-   A codec identifier per tile.
-   Optional multiresolution preview levels.
-   Integrity checks.
-   Optional metadata chunks.
-   An editing journal or copy-on-write generation mechanism.
-   A decoder API for full decode, tile decode, and region decode.

VPNG is not inherently a neural image format. AI may be used
experimentally to select encoding strategies, but it must not generate
or infer original pixels in the lossless decoding path.

## 1.1 Initial supported pixel formats

The first stable implementation should support:

  Format                     Channels             Channel depth
  ----------------- ----------------- -------------------------
  Gray                              1              8 or 16 bits
  Gray + alpha                      2              8 or 16 bits
  RGB                               3              8 or 16 bits
  RGBA                              4              8 or 16 bits
  Indexed palette     Palette-defined   8-bit indices initially

Requirements:

-   Preserve channel values exactly.
-   Preserve alpha exactly.
-   Preserve RGB values beneath fully transparent pixels.
-   Define byte order for 16-bit channels.
-   Define canonical pixel order and row order.
-   Define whether row padding is forbidden in canonical pixel buffers.
-   Color profiles and metadata must be preserved when supported.
-   Do not conflate pixel-value preservation with color-managed visual
    appearance.

Initial version should use unsigned integer channels. Floating-point/HDR
formats are out of scope for v1.0 unless separately specified and
tested.

## 1.2 Scope exclusions for v1.0

Do not include these in the first stable release:

-   Animation.
-   Layers.
-   Lossy encoding.
-   Neural reconstruction.
-   GPU-only decoding.
-   Unbounded streaming decompression.
-   Arbitrary executable metadata.
-   Unspecified external dictionary dependencies.
-   In-place mutation that can corrupt the only valid copy of a file.

Design extension points for future versions without implementing them
prematurely.

------------------------------------------------------------------------

# 2. Measurable product requirements

## 2.1 Correctness

-   100% exact pixel round-trip for every supported pixel format.
-   SHA-256 digest of canonical pixel data matches after decode.
-   All codecs have independent round-trip tests.
-   All tile layouts reconstruct the correct global image coordinates.
-   No tile overlap, gaps, or out-of-bounds access.
-   Malformed files fail safely with typed errors.

## 2.2 Random access

-   Decode one tile without decoding unrelated tiles.
-   Decode a rectangular region by loading only intersecting tiles plus
    any explicitly required preview/metadata.
-   Provide a stable tile-coordinate API.
-   Support concurrent decoding of independent tiles.
-   A tile's codec must not depend on hidden state from another tile.

## 2.3 Editing

-   Read-modify-write a region without full-image decode.
-   Unmodified tile payloads should be reusable byte-for-byte where the
    container layout permits.
-   Use copy-on-write or append-only generations so an interrupted edit
    cannot destroy the last valid file state.
-   Support edit transactions, commit, abort, undo, and redo at the
    library level.
-   Preserve exact pixels after saving and reopening an edited image.

## 2.4 Memory

Proposed engineering targets, to be measured rather than assumed:

  -----------------------------------------------------------------------
  Scenario                                                         Target
  ------------------------------ ----------------------------------------
  Small tile decode working set                             32 MB or less

  16K tiled region editing CPU                              64 MB or less
  working set                    

  Full 16K RGBA8 decode              Explicit opt-in; approximately 1 GiB
                                               pixel buffer plus overhead

  Streaming decode                         No full-image pixel allocation

  Cache                                      Configurable hard byte limit
  -----------------------------------------------------------------------

The memory target excludes OS page cache, application display
framebuffer, GPU textures, and memory owned by the calling application.
Reports must state exactly what is included.

## 2.5 File size

-   `--max-size 10MB` means a decimal 10,000,000-byte ceiling unless the
    CLI explicitly offers a separate MiB option.
-   The encoder must include header, directory, metadata, previews,
    padding, and all payloads in its size calculation.
-   The encoder must not write a partial file and report success.
-   If no lossless representation fits, return a clear
    `SizeLimitExceeded` result and report the smallest valid size found.
-   A user may request `--max-size 10MiB` separately.
-   Report compression ratio and bytes per pixel.

## 2.6 Performance

No performance claim is valid until benchmarked on named hardware and a
named corpus.

Initial targets for investigation:

-   Tile decode latency: measure p50/p95/p99 for 256², 512², and 1024²
    tiles.
-   Preview display: measure time-to-first-preview.
-   Region decode: measure latency by region size and tile count.
-   Local edit: measure read, modify, encode, commit, and reopen
    separately.
-   Parallel decode: compare 1, 2, 4, and 8 workers.
-   Memory: measure peak RSS and internal allocations.

------------------------------------------------------------------------

# 3. Repository reconnaissance and initial deliverables

Before implementation:

1.  Inspect the current working directory and repository.
2.  Identify language/toolchain versions and available build tools.
3.  Determine whether the repository already contains code or
    uncommitted work.
4.  Do not delete or replace files without explicit need.
5.  Create a baseline report at `docs/REPOSITORY_AUDIT.md`.
6.  Create a baseline test and benchmark manifest.
7.  Initialize version control only if the repository is not already
    under version control and the user has authorized repository
    modification.

Required initial deliverables:

-   `docs/REPOSITORY_AUDIT.md`
-   `docs/ROADMAP_STATUS.md`
-   `docs/VPNG_SPEC_0_1.md`
-   `docs/THREAT_MODEL.md`
-   `benchmarks/corpus_manifest.json`
-   A compiling Rust workspace
-   A minimal Python experiment environment

------------------------------------------------------------------------

# 4. System architecture

Use a Rust workspace. Keep the file format, codecs, image model, and
editor abstractions separated.

Suggested repository:

``` text
vpng/
├── Cargo.toml
├── Cargo.lock
├── pyproject.toml
├── README.md
├── LICENSE
├── CHANGELOG.md
├── .gitignore
├── docs/
│   ├── REPOSITORY_AUDIT.md
│   ├── ROADMAP_STATUS.md
│   ├── VPNG_SPEC_0_1.md
│   ├── VPNG_SPEC_1_0.md
│   ├── BITSTREAM.md
│   ├── CODEC_DESIGN.md
│   ├── EDITING_MODEL.md
│   ├── MEMORY_MODEL.md
│   ├── THREAT_MODEL.md
│   ├── BENCHMARKS.md
│   └── decisions/
├── crates/
│   ├── vpng-core/
│   ├── vpng-container/
│   ├── vpng-codecs/
│   ├── vpng-analyzer/
│   ├── vpng-tiling/
│   ├── vpng-decoder/
│   ├── vpng-encoder/
│   ├── vpng-editor/
│   ├── vpng-cli/
│   └── vpng-python/
├── python/
│   ├── experiments/
│   ├── corpus/
│   └── tools/
├── tests/
│   ├── golden/
│   ├── roundtrip/
│   ├── malformed/
│   ├── fuzz/
│   ├── interoperability/
│   └── editing/
├── benchmarks/
│   ├── corpus_manifest.json
│   ├── results/
│   ├── scripts/
│   └── reports/
└── examples/
```

## 4.1 Crate responsibilities

### `vpng-core`

-   Canonical image descriptor.
-   Pixel formats.
-   Checked dimension arithmetic.
-   Error types.
-   Resource limits.
-   Coordinates and rectangles.
-   Public configuration structures.

### `vpng-container`

-   Signature and header parser/writer.
-   Chunk parser/writer.
-   Tile directory.
-   Offset validation.
-   Checksums and content hashes.
-   Version and feature compatibility.

### `vpng-codecs`

-   Raw codec.
-   Palette codec.
-   RLE codec.
-   Predictive residual codec.
-   Dictionary codec.
-   Entropy coders.
-   Codec registry.
-   Codec capability declarations.

### `vpng-analyzer`

-   Entropy estimates.
-   Channel statistics.
-   Local correlation.
-   Gradient statistics.
-   Repetition estimates.
-   Codec-cost estimation.
-   No pixel mutation.

### `vpng-tiling`

-   Tile geometry.
-   Tile partition planning.
-   Edge-tile handling.
-   Region-to-tile mapping.
-   Tile scheduling.

### `vpng-decoder`

-   Full decode.
-   Tile decode.
-   Region decode.
-   Streaming reader.
-   Bounded tile cache.
-   Parallel decode.

### `vpng-encoder`

-   Input normalization to canonical pixels.
-   Analysis and tile planning.
-   Candidate codec evaluation.
-   Compression selection.
-   Directory generation.
-   Output size accounting.
-   Atomic output commit.

### `vpng-editor`

-   Tile-backed document.
-   Edit transactions.
-   Copy-on-write.
-   Dirty-tile tracking.
-   Undo/redo.
-   Region operations.
-   Save/recovery.

### `vpng-cli`

-   Encode/decode/info/verify/benchmark commands.
-   Stable exit codes.
-   Human-readable and JSON output.

### `vpng-python`

-   Python bindings around the stable Rust APIs.
-   Avoid implementing the codec twice in Python.

------------------------------------------------------------------------

# 5. File format specification

The first task is to turn this proposal into a precise, unambiguous
binary specification. Do not begin broad codec optimization until the
container parser and writer are stable.

## 5.1 General rules

-   File extension: `.vpng`
-   Magic bytes: `56 50 4E 47 0D 0A 1A 0A`
-   Multibyte integers: little-endian.
-   No implicit structure padding.
-   All offsets are from the beginning of the file unless explicitly
    stated.
-   All lengths are unsigned and validated using checked arithmetic.
-   Reserved fields must be zero in v0.1.
-   Unknown optional chunks may be skipped.
-   Unknown mandatory features must fail with `UnsupportedFeature`.
-   The file must have one authoritative directory generation.
-   The parser must reject overlapping payload ranges unless the spec
    explicitly allows safe deduplication.
-   Define exact maximum lengths and integer widths for every field.

## 5.2 Proposed 64-byte main header

    Offset   Bytes Field
  -------- ------- -----------------------
         0       8 Magic
         8       2 Major version
        10       2 Minor version
        12       4 Header size
        16       4 Width
        20       4 Height
        24       1 Channel count
        25       1 Bits per channel
        26       1 Color model
        27       1 Default codec/profile
        28       4 Tile count
        32       8 Directory offset
        40       8 Declared file size
        48       4 Feature flags
        52       4 Header checksum
        56       8 Reserved

The agent must verify that this layout is exactly 64 bytes and that
checksum coverage and checksum-field treatment are explicitly defined.

## 5.3 Tile directory

The initial proposal uses a 40-byte fixed entry:

  Field                   Bytes
  --------------------- -------
  Tile X coordinate           4
  Tile Y coordinate           4
  Tile width                  2
  Tile height                 2
  Codec ID                    1
  Predictor ID                1
  Flags                       2
  Payload offset              8
  Compressed length           8
  Uncompressed length         4
  Payload checksum            4

The original 40-byte draft had an ambiguous combination of sizes.
Reconcile the fields and publish one exact layout. If 64-bit
uncompressed length is needed, enlarge the entry and update the
specification. Do not preserve a flawed table merely for compatibility
before any files have been released.

Directory invariants:

-   Every tile lies within image bounds.
-   Tile rectangles cover the image exactly once for a complete level.
-   No unintended overlaps or gaps.
-   Edge tiles may be smaller than nominal tile size.
-   Every payload range lies within the file.
-   Payload offsets and lengths must not overflow.
-   The directory must be sorted in a defined order, such as level, y,
    x.
-   A tile is independently decodable in v1.0.

## 5.4 Chunk structure

Proposed generic chunk:

``` text
type       4 bytes
flags      4 bytes
length     8 bytes
checksum   4 bytes
payload    length bytes
```

Initial chunk types:

-   `ICCP`: ICC color profile.
-   `EXIF`: EXIF metadata.
-   `XMP`: XMP metadata.
-   `THMB`: preview image.
-   `HASH`: canonical pixel digest.
-   `TIDX`: tile index, if stored as a chunk.
-   `PYRM`: preview pyramid descriptors.
-   `EDIT`: optional editing metadata/journal descriptor.

Define whether the main directory or `TIDX` is authoritative. There must
not be two conflicting sources of truth.

## 5.5 Codec identifiers

Initial IDs:

    ID Codec
  ---- -----------------------------
     0 Raw
     1 Palette
     2 Run-length
     3 Spatial predictor + entropy
     4 LZ77
     5 Predictor + dictionary
     6 Context-adaptive entropy

Each codec ID needs:

-   Exact input byte layout.
-   Exact output byte layout.
-   Predictor definitions.
-   Reset rules.
-   Maximum memory requirements.
-   Error behavior.
-   Versioning rules.
-   Golden test vectors.

Never rely on unspecified platform behavior.

## 5.6 Integrity

Use two levels:

1.  Per-tile checksum for corruption detection.
2.  Whole-image cryptographic digest over canonical image metadata and
    pixel bytes.

Use a standard SHA-256 implementation from a maintained cryptographic
crate. Do not implement SHA-256 yourself.

Specify canonical digest input exactly:

-   Width and height.
-   Pixel format and channel order.
-   Bit depth and endianness.
-   Row order.
-   Pixel bytes with no row padding.
-   Explicit inclusion of RGB under zero alpha.

Metadata should have its own integrity policy. A pixel hash alone does
not prove metadata preservation.

------------------------------------------------------------------------

# 6. Codec implementation roadmap

Implement codec candidates behind one stable trait.

Conceptual interface:

``` rust
pub trait TileCodec {
    fn codec_id(&self) -> u8;

    fn encode(
        &self,
        input: &[u8],
        context: &EncodeContext,
        output: &mut Vec<u8>,
    ) -> Result<EncodeStats, CodecError>;

    fn decode(
        &self,
        input: &[u8],
        expected_output_len: usize,
        context: &DecodeContext,
        output: &mut Vec<u8>,
    ) -> Result<(), CodecError>;
}
```

The actual API must avoid unnecessary allocations and must enforce
output limits. A production API may use caller-provided buffers or
bounded writers.

## 6.1 Codec A --- Raw

Purpose:

-   Correctness baseline.
-   Fallback for incompressible tiles.
-   Golden reference.

Tests:

-   All supported formats.
-   Odd dimensions.
-   Empty/invalid input rejection.
-   Exact bytes.

## 6.2 Codec B --- Palette

-   Build a per-tile palette.
-   Encode palette entries exactly.
-   Encode indices with the smallest valid bit width.
-   Preserve alpha and all channel values.
-   Fall back if palette overhead exceeds raw size.

Test images with 1, 2, 3, 16, 256, and more than 256 unique colors.

## 6.3 Codec C --- RLE

-   Run-length encode exact repeated symbols or pixel tuples.
-   Define maximum run length.
-   Escape literal/run markers unambiguously.
-   Avoid pathological expansion.
-   Select only if total encoded size is smaller.

Test alternating pixels, single long runs, short runs, and random noise.

## 6.4 Codec D --- Spatial prediction

Implement and test:

-   Left.
-   Up.
-   Upper-left.
-   Average.
-   Paeth.
-   Gradient predictor.
-   Adaptive predictor selected by measured cost.

Use reversible modular residual arithmetic appropriate to channel depth.
Do not use floating-point prediction in the lossless transform.

Prediction must be defined across channels and rows. Tile boundaries
reset prediction state unless the format explicitly encodes
dependencies.

## 6.5 Codec E --- Dictionary

-   Implement a bounded LZ77-style dictionary.
-   Bound match length, distance, and memory.
-   Ensure references are valid within the tile.
-   Define literal and match tokens unambiguously.
-   Avoid cross-tile dependencies in v1.0.

## 6.6 Codec F --- Entropy coding

Benchmark:

-   Canonical Huffman.
-   Range coding.
-   Context-adaptive binary arithmetic coding.

Do not assume the most sophisticated coder is always best. Measure
complete tile payload size including model headers and termination
bytes.

## 6.7 Hybrid selection

For each tile:

1.  Estimate candidate costs.
2.  Run candidate encoders where the estimate is promising.
3.  Include all per-codec headers and model overhead.
4.  Choose the smallest valid complete payload.
5.  Use raw if no candidate is smaller.
6.  Record chosen codec and measured cost in diagnostics.
7.  Ensure deterministic tie-breaking.

The encoder must not select based only on an approximate entropy
estimate if a real encoded size is available.

------------------------------------------------------------------------

# 7. Adaptive tiling and random access

## 7.1 Initial tile sizes

Experiment with:

-   32 × 32
-   64 × 64
-   128 × 128
-   256 × 256
-   512 × 512
-   1024 × 1024

Do not assume variable tile sizes are automatically superior. Larger
tiles reduce directory overhead but can increase edit cost, memory use,
and decode latency. Smaller tiles improve localization but add overhead.

## 7.2 Initial policy

Start with fixed 256 × 256 tiles for the reference implementation. Add
selectable tile sizes after correctness and random-access APIs work.

Then compare:

-   Fixed 128.
-   Fixed 256.
-   Fixed 512.
-   Fixed 1024.
-   Adaptive tile partitioning.

If using variable-size rectangles, specify a complete partition
algorithm and verify exact coverage.

## 7.3 Region decoding

Public API should accept a rectangle in image coordinates:

``` rust
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
```

Region decode should:

-   Validate rectangle bounds with checked arithmetic.
-   Find intersecting tiles via the index.
-   Decode only those tiles.
-   Copy only intersecting pixel spans into the result.
-   Return region origin and exact pixel format.
-   Avoid allocating the full image.

## 7.4 Parallel decoding

-   Tiles must be independent.
-   Worker count must be configurable.
-   Bound queued tasks and output buffers.
-   Avoid unbounded task creation.
-   Preserve deterministic output placement.
-   Compare 1/2/4/8 workers.
-   Ensure errors cancel or drain work safely.

------------------------------------------------------------------------

# 8. Editing engine

Editing is a first-class subsystem, not an afterthought.

## 8.1 Document model

A VPNG document should contain:

-   Immutable image metadata snapshot.
-   Tile index.
-   Compressed tile payload references.
-   Bounded decoded-tile cache.
-   Dirty tile set.
-   Transaction state.
-   Optional undo/redo history.
-   Current generation identifier.

The editor must not require the entire decoded image to exist in RAM.

## 8.2 Localized edit flow

1.  Map requested edit rectangle to affected tile IDs.
2.  Load only affected tiles.
3.  Decode into reusable buffers.
4.  Apply the exact pixel operation.
5.  Mark affected tiles dirty.
6.  Re-encode dirty tiles.
7.  Preserve unmodified tile payloads.
8.  Build a new directory generation.
9.  Write a temporary file or append a new safe generation.
10. Validate the new generation.
11. Atomically commit the file replacement where supported.

Do not mutate the only valid file state before the new state is complete
and verified.

## 8.3 Transaction API

Proposed interface:

``` rust
pub trait EditTransaction {
    fn read_region(&mut self, rect: Rect) -> Result<PixelRegion, EditError>;
    fn write_region(&mut self, rect: Rect, pixels: &PixelRegion)
        -> Result<(), EditError>;
    fn commit(self) -> Result<CommitInfo, EditError>;
    fn abort(self) -> Result<(), EditError>;
}
```

The exact ownership/lifetime design may differ. The implementation must
make invalid transaction states difficult to express.

## 8.4 Undo and redo

Use tile-level copy-on-write history initially.

For each committed edit, store references to prior and new tile payloads
or immutable tile generations. Avoid duplicating unchanged tiles.

Requirements:

-   Undo restores exact previous pixels.
-   Redo restores exact edited pixels.
-   History has a configurable byte budget.
-   Eviction policy is explicit.
-   Crash recovery never mistakes an incomplete generation for committed
    data.

## 8.5 Editing operations

Initial library-level operations:

-   Read region.
-   Write region.
-   Brush-like pixel writes.
-   Erase by exact pixel replacement.
-   Crop by creating a new image view/generation.
-   Copy/paste region.
-   Color transform using exact integer arithmetic where applicable.
-   Undo/redo.

Brush rendering UI is outside the core codec. Provide APIs that an
editor UI can call.

------------------------------------------------------------------------

# 9. Preview pyramid

Preview levels are optional derived data. They must never replace
original pixels.

Proposed levels for a 16K source:

-   16,384: full resolution.
-   8,192.
-   4,096.
-   2,048.
-   1,024.
-   512. 

Important distinction:

-   The full-resolution level is lossless source data.
-   Lower levels are derived previews and are not exact substitutes for
    source pixels.
-   The format must mark derived levels as previews.
-   The exact downsampling algorithm and color/alpha handling must be
    documented if previews are intended to be reproducible.
-   Editing a preview must not silently edit the original. An editor
    must map operations to full-resolution source pixels or explicitly
    identify preview-only edits.

For the initial implementation, a 512 or 1024 thumbnail is sufficient.
Add the full pyramid only after the base format is stable.

------------------------------------------------------------------------

# 10. Memory and performance architecture

## 10.1 Bounded memory model

Use:

-   Reusable tile buffers.
-   Bounded tile cache.
-   Bounded worker queue.
-   Streaming file reader/writer.
-   Caller-configurable memory budget.
-   Explicit decompressed-size limits.
-   LRU or clock cache with measurable accounting.

Do not allocate buffers from untrusted declared lengths before
validating them against configured limits.

## 10.2 Initial memory targets

These are targets for experiments:

  Workload                                                    Target
  --------------------- --------------------------------------------
  One 256² RGBA8 tile                         256 KiB decoded pixels
  One 512² RGBA8 tile                           1 MiB decoded pixels
  Four 512² tiles                               4 MiB decoded pixels
  16K region editing             ≤64 MB codec/editor CPU working set
  Streaming view          ≤32 MB decoder working set where practical

Measure both internal allocation accounting and process peak RSS.
Explain differences.

## 10.3 CPU and GPU responsibilities

Initial version:

-   CPU: parse container, decode, encode, edit pixels, maintain
    transactions.
-   GPU: optional display textures and application rendering only.
-   Do not require a GPU for correctness or format interoperability.

Later:

-   GPU color operations.
-   GPU brush preview.
-   GPU tile transforms.
-   Optional GPU-assisted analysis.

GPU acceleration must not change exact pixel semantics unless the
operation is explicitly defined with exact integer arithmetic and tested
across devices.

## 10.4 Performance instrumentation

Every benchmark should record:

-   CPU model.
-   OS.
-   RAM.
-   Rust version.
-   Build profile and compiler flags.
-   Worker count.
-   Input dimensions and format.
-   Tile size.
-   Codec choices.
-   Input bytes.
-   Output bytes.
-   Encode time.
-   Decode time.
-   Region decode time.
-   Edit/commit time.
-   Peak RSS.
-   Internal allocation peak.
-   Verification result.

Use machine-readable JSON/CSV results and preserve raw runs.

------------------------------------------------------------------------

# 11. Benchmark corpus and experimental method

## 11.1 Corpus categories

Create a reproducible corpus manifest. Use legally usable images or
user-provided test images with recorded provenance.

Target at least:

  Category                   Initial count
  ------------------------ ---------------
  Photographs                          200
  Anime/illustrations                  100
  Digital paintings                    100
  Screenshots/text/UI                  100
  3D renders                           100
  Procedural textures                  100
  Synthetic random/noise                50
  **Total**                        **750**

Test a range of dimensions, including 512, 1024, 2048, 4096, 8192, and
16K where available.

Do not upscale small images and count them as native 16K compression
evidence.

## 11.2 Baselines

Compare exact pixel data against:

-   PNG.
-   WebP lossless.
-   JPEG XL lossless.
-   FLIF where a reliable implementation is available.
-   Raw uncompressed data.

Record encoder versions, settings, and color interpretation.

Do not compare files with different metadata or pixel semantics without
explaining normalization.

## 11.3 Metrics

-   Exact pixel equality.
-   Canonical SHA-256 equality.
-   File bytes.
-   Compression ratio.
-   Bits per pixel.
-   Encode throughput.
-   Decode throughput.
-   Random tile latency.
-   Region decode latency.
-   Edit latency.
-   Peak memory.
-   Corruption detection.
-   Corpus success rate under 10 MB.

## 11.4 Reporting rules

Report:

-   Mean, median, p95, p99 where relevant.
-   Per-category results.
-   Per-resolution results.
-   Per-codec results.
-   Worst cases.
-   Failures and skipped cases.
-   Aggregate results only alongside per-category results.

Do not claim "VPNG is better" based on one image or one category.

------------------------------------------------------------------------

# 12. CLI and Python API

## 12.1 CLI commands

Implement:

``` bash
vpng encode input.png output.vpng
vpng encode input.png output.vpng --max-size 10MB
vpng decode input.vpng output.png
vpng info input.vpng
vpng verify input.png output.vpng
vpng decode-region input.vpng output.png --x 100 --y 200 --width 512 --height 512
vpng extract-tile input.vpng --tile-x 3 --tile-y 4 --output tile.png
vpng benchmark input.png --compare png,webp,jxl
vpng inspect-directory input.vpng
vpng repair-index input.vpng --output repaired.vpng
```

The repair command must only rebuild information that can be
unambiguously recovered. It must never invent missing pixels.

## 12.2 Exit codes

Define stable exit codes, at minimum:

-   0: success.
-   1: generic failure.
-   2: invalid arguments.
-   3: unsupported feature/version.
-   4: corrupt file.
-   5: resource limit exceeded.
-   6: size limit exceeded.
-   7: exact verification failure.
-   8: I/O failure.

## 12.3 Python API

Proposed API:

``` python
import vpng

vpng.encode(
    "input.png",
    "output.vpng",
    lossless=True,
    max_size=10_000_000,
    adaptive_tiles=True,
)

vpng.decode("output.vpng", "restored.png")

assert vpng.verify("input.png", "output.vpng")

with vpng.open_document("output.vpng", cache_bytes=16_000_000) as doc:
    region = doc.read_region(x=0, y=0, width=512, height=512)
    with doc.edit() as tx:
        tx.write_region(x=0, y=0, pixels=region)
        tx.commit()
```

This is a target API. Implement only when the underlying Rust behavior
exists and is tested.

------------------------------------------------------------------------

# 13. Security and robustness

Treat all VPNG files as untrusted input.

Required protections:

-   Checked integer arithmetic.
-   Maximum dimensions and pixel count.
-   Maximum tile count.
-   Maximum metadata size.
-   Maximum directory size.
-   Maximum compressed and decompressed tile sizes.
-   Maximum compression expansion ratio or equivalent allocation budget.
-   Bounded recursion (prefer no recursive parsing).
-   Validate every offset and range before reading.
-   Reject invalid codec parameters.
-   Reject malformed entropy streams safely.
-   Reject invalid dictionary references.
-   Validate tile coverage and overlaps.
-   Avoid panic on malformed input.
-   No arbitrary code execution from metadata.
-   No network access during decode.
-   Fuzz parser, directory, chunks, and every codec.
-   Run sanitizers or equivalent tools where available.
-   Keep dependencies current and audited.

Add a threat model describing denial-of-service, malformed-file,
integer-overflow, decompression-bomb, metadata, and partial-write risks.

------------------------------------------------------------------------

# 14. Testing strategy

## 14.1 Unit tests

Test each module independently:

-   Header read/write.
-   Chunk read/write.
-   Directory validation.
-   Checked size calculations.
-   Pixel format conversions.
-   Tile geometry.
-   Region-to-tile mapping.
-   Predictor residual/reconstruction.
-   Palette coding.
-   RLE.
-   Dictionary.
-   Entropy coder.
-   Cache accounting.
-   Transaction state machine.

## 14.2 Property-based tests

Generate random:

-   Dimensions.
-   Pixel formats.
-   Tile sizes.
-   Pixel data.
-   Tile boundaries.
-   Codec parameters.
-   Region rectangles.

Assert exact round-trip and no panic.

## 14.3 Golden files

Keep a versioned set of `.vpng` files with:

-   Expected dimensions.
-   Expected format.
-   Expected canonical hash.
-   Expected tile count.
-   Expected codec IDs.
-   Expected decode result.

Golden files are required for every released format version.

## 14.4 Malformed-file tests

Mutate:

-   Magic bytes.
-   Version.
-   Header size.
-   Width/height.
-   Tile count.
-   Directory offset.
-   Payload offset.
-   Payload length.
-   Checksum.
-   Codec ID.
-   Chunk length.
-   Tile rectangles.
-   Entropy stream termination.
-   Dictionary references.

All malformed files must fail safely and deterministically.

## 14.5 Editing tests

-   Edit one tile; verify untouched tile payloads remain unchanged where
    promised.
-   Edit across tile boundaries.
-   Abort transaction.
-   Commit transaction.
-   Undo and redo.
-   Simulate interrupted write.
-   Reopen after commit.
-   Verify exact pixel hash.
-   Enforce cache budget.

------------------------------------------------------------------------

# 15. Detailed phase roadmap and gates

## Phase 0 --- Repository audit and specification

Deliver:

-   Repository audit.
-   Architecture decision records.
-   Precise v0.1 binary specification.
-   Threat model.
-   Workspace skeleton.

Gate P0:

-   Specification has no ambiguous field sizes.
-   Header and directory layouts have executable serialization tests.
-   Build environment is reproducible.

## Phase 1 --- Minimal valid container

Implement:

-   Header parser/writer.
-   Directory parser/writer.
-   Raw tile codec.
-   Single-level tiled image.
-   Checksums.
-   CLI `encode`, `decode`, `info`, `verify`.

Gate P1:

-   Exact round-trip for all initial pixel formats.
-   Golden files.
-   No whole-image requirement for tile extraction.
-   Malformed header/directory tests pass.

## Phase 2 --- Baseline codecs

Implement:

-   Palette.
-   RLE.
-   Spatial predictors.
-   LZ77.
-   Entropy coder candidates.
-   Cost-based selection.

Gate P2:

-   Every codec has exact round-trip tests.
-   Candidate selection accounts for all overhead.
-   Raw fallback works.
-   No regression in exactness.

## Phase 3 --- Adaptive tiling and region decode

Implement:

-   Tile-size configuration.
-   Adaptive partition experiment.
-   Tile index.
-   Region decoder.
-   Bounded cache.
-   Parallel independent tile decode.

Gate P3:

-   Region decode equals the corresponding crop of full decode.
-   Tile coverage validation passes.
-   No unrelated tiles are decoded in a region request.
-   Memory stays within configured bounds.

## Phase 4 --- Editing engine

Implement:

-   Document API.
-   Dirty-tile tracking.
-   Localized decode/edit/encode.
-   Copy-on-write generation.
-   Atomic commit.
-   Undo/redo.
-   Recovery tests.

Gate P4:

-   Single-tile edit does not decode the full image.
-   Unmodified payload reuse is demonstrated.
-   Abort leaves original file valid.
-   Crash simulation never exposes an incomplete generation as
    committed.
-   Save/reopen pixel hash is exact.

## Phase 5 --- Preview and multiresolution

Implement:

-   Thumbnail chunk.
-   Preview generation.
-   Preview loading before full-resolution tiles.
-   Optional pyramid descriptors.
-   Viewport-oriented tile scheduling.

Gate P5:

-   Preview is explicitly marked derived.
-   Original level remains exact.
-   Preview cannot silently replace source data.
-   Time-to-first-preview is measured.

## Phase 6 --- 16K and memory engineering

Implement:

-   Large-image stress harness.
-   Checked arithmetic.
-   Streaming input/output.
-   Allocation budget.
-   Cache eviction.
-   Parallel worker limits.

Gate P6:

-   16K RGBA8 encode/decode where resource limits permit.
-   Region editing under the target memory budget.
-   No full-image allocation in streaming mode.
-   Peak RSS and internal allocations are reported.
-   All resource-limit tests pass.

## Phase 7 --- Compression research

Run controlled experiments:

-   Fixed tile sizes.
-   Adaptive tile sizes.
-   Predictor combinations.
-   Dictionary combinations.
-   Entropy models.
-   Context models.
-   Codec selector heuristics.

Gate P7:

-   Raw results preserved.
-   Baseline comparisons use identical canonical pixels.
-   Per-category reports generated.
-   Improvements are statistically and practically meaningful.
-   No lossless failures.

## Phase 8 --- 10 MB target evaluation

Implement:

-   Exact final-size accounting.
-   Candidate strategy search.
-   `--max-size`.
-   Clear size-limit failure.
-   Category-level success reporting.

Gate P8:

-   No output exceeding the configured limit is reported as success.
-   No lossy fallback exists.
-   Success rate is reported separately for every corpus category and
    resolution.
-   Random-noise worst cases are included.

## Phase 9 --- Python API and editor integration

Implement:

-   Stable Python bindings.
-   Region read/write.
-   Edit transactions.
-   Cache configuration.
-   Documentation and examples.
-   A small demonstration editor or integration harness.

Gate P9:

-   Python API passes the same correctness tests.
-   No duplicate codec implementation in Python.
-   Editing workflow is reproducible.

## Phase 10 --- Hardening and v1.0

Implement:

-   Fuzzing.
-   Security review.
-   Golden compatibility suite.
-   Format freeze.
-   Release packaging.
-   CLI help and API documentation.
-   Reproducible benchmark report.

Gate P10:

-   All release-blocking tests pass.
-   No known silent-corruption issue.
-   Version compatibility policy documented.
-   Reference decoder available.
-   v1.0 specification frozen.

------------------------------------------------------------------------

# 16. Acceptance matrix

Maintain this table in `docs/ROADMAP_STATUS.md`.

  ID    Requirement                    Evidence required
  ----- ------------------------------ --------------------------
  A01   Exact RGB8 round-trip          Automated tests + hashes
  A02   Exact RGBA8 round-trip         Automated tests + hashes
  A03   Exact 16-bit round-trip        Automated tests + hashes
  A04   Transparent RGB preservation   Dedicated tests
  A05   Raw fallback                   Incompressible test
  A06   Palette codec                  Golden vectors
  A07   RLE codec                      Golden vectors
  A08   Prediction codec               Property tests
  A09   Dictionary codec               Property tests
  A10   Entropy coder                  Golden vectors
  A11   Tile-only decode               Instrumented test
  A12   Region decode                  Crop equality test
  A13   Parallel decode                Determinism test
  A14   Bounded cache                  Allocation test
  A15   Localized edit                 Instrumented test
  A16   Undo/redo                      Exact hash tests
  A17   Crash-safe commit              Fault injection
  A18   Preview separation             Format and API tests
  A19   16K stress                     Recorded run
  A20   10 MB enforcement              Boundary tests
  A21   Corruption detection           Mutation tests
  A22   Fuzz robustness                Fuzz report
  A23   Python API                     Integration tests
  A24   v1.0 compatibility             Golden corpus

A gate may be marked `PASS`, `FAIL`, `BLOCKED`, or `NOT STARTED`. Never
use ambiguous status such as "mostly done."

------------------------------------------------------------------------

# 17. Required benchmark experiments

## Experiment E1 --- Raw container overhead

-   Encode raw tiles only.
-   Measure header, directory, and metadata overhead.
-   Verify all pixel formats.

## Experiment E2 --- Codec comparison

For each image and tile size, encode with each individual codec.
Preserve per-tile and per-image results.

## Experiment E3 --- Hybrid selection

Compare the selected hybrid against the best individual codec per image
and per tile.

## Experiment E4 --- Tile size

Compare 32, 64, 128, 256, 512, and 1024 square tiles. Measure file size,
random-access latency, edit cost, and memory.

## Experiment E5 --- Region decode

Test small, medium, and large rectangles at multiple image locations.
Verify the decoder loads only intersecting tiles.

## Experiment E6 --- Editing locality

Edit a small region of a 16K image. Record: - Tiles loaded. - Tiles
dirtied. - Bytes read. - Bytes written. - Time to commit. - Peak
memory. - Unchanged payload identity.

## Experiment E7 --- 10 MB success distribution

For each corpus category, report: - Number tested. - Number at or below
10,000,000 bytes. - Median output size. - p95 output size. - Worst
output size. - Exactness pass rate.

## Experiment E8 --- Streaming performance

Measure: - Time to first preview. - Time to first visible tile. -
Panning latency. - Zoom-level transition latency. - Cache hit rate. -
Decode worker utilization.

------------------------------------------------------------------------

# 18. Design decisions that must be resolved explicitly

The agent must create architecture decision records for:

1.  Exact 64-byte header checksum coverage.
2.  Final tile directory entry width and field order.
3.  Whether the directory is inline or chunk-based.
4.  Tile partition representation for adaptive rectangles.
5.  Checksums: CRC32C or another specified checksum for tile corruption
    detection.
6.  SHA-256 canonical image digest encoding.
7.  Entropy coder choice based on measured results.
8.  Whether dictionary references are tile-local only in v1.0.
9.  Atomic replacement behavior on Windows, Linux, and macOS.
10. Append-only generation versus temporary-file rewrite.
11. Metadata preservation and unsupported metadata policy.
12. Preview generation algorithm.
13. Cache eviction policy.
14. Parallel decode scheduling policy.
15. Whether 16-bit channels are supported in the first public release or
    in a subsequent minor version.

Do not silently decide a format-level question in implementation code.
Record the decision in `docs/decisions/`.

------------------------------------------------------------------------

# 19. Release criteria and claims policy

VPNG v1.0 may be released only when:

-   The binary specification is frozen.
-   The reference decoder passes all golden tests.
-   Every advertised codec is tested.
-   Malformed input is handled safely.
-   Full and region decoding are exact.
-   Local editing is transaction-safe.
-   16K behavior is measured.
-   Memory use is measured.
-   Benchmarks against established lossless formats are reproducible.
-   The 10 MB limit is enforced honestly.

Allowed claims:

-   "Lossless" only for exact verified paths.
-   "16K supported" only after successful tests at the stated pixel
    format.
-   "Under 10 MB" only for the tested image or explicitly reported
    corpus subset.
-   "Low memory" only with a stated measurement and workload.
-   "Faster" only with named baselines, hardware, and settings.

Forbidden claims:

-   Guaranteed 16K arbitrary image compression under 10 MB.
-   Guaranteed 32 MB runtime for every workload.
-   "Visually lossless" as a substitute for exact losslessness.
-   Neural reconstruction described as exact lossless decoding.
-   Benchmark superiority based on selected favorable images only.

------------------------------------------------------------------------

# 20. Immediate execution order

Start now with this exact order:

1.  Audit repository and environment.
2.  Create `docs/REPOSITORY_AUDIT.md`.
3.  Create `docs/ROADMAP_STATUS.md`.
4.  Resolve and document the exact header and directory layouts.
5.  Create Rust workspace and compile it.
6.  Implement canonical pixel model and checked dimension arithmetic.
7.  Implement raw tiled container.
8.  Implement encode/decode/info/verify CLI.
9.  Add exact RGB/RGBA 8-bit and 16-bit tests.
10. Add golden files and malformed-file tests.
11. Only then begin palette, RLE, predictor, dictionary, and entropy
    coding.

Do not jump directly to neural compression, GPU optimization, adaptive
AI selection, or a graphical editor.

# 21. Feature backlog and the release plan (v0.2 &#8594; v0.23)

## 21.1 The feature backlog

The capabilities below were proposed for the project and are now attached
to phases, so that each one is part of the plan rather than a wish outside
it. The canonical table — with the state of every item and what each one
needs — lives in `docs/ROADMAP_STATUS.md` &#167;2.2, which is the living
record; this section is the authoritative statement that the items exist
and where they belong.

| Capability | Phase | Notes |
|---|---|---|
| Multiresolution preview pyramid (mipmaps) | P6 | More than one grid of tiles; a format revision. |
| Parallel tile decoding | P5 | **Done.** The streaming reader, the bounded scheduler, the parallel worker pool, the predictive prefetch and the acceptance benchmark are all in place. The 16K RGBA8 acceptance case ran: 63.56 ms for the first viewport against the sequential baseline's 118.96 ms, 16 of 4,096 tiles decoded, 47.9 MB peak resident set. A 144-case matrix asserts every parallel tile is byte-identical to the sequential decoder's. Evidence: `docs/P5_REPORT.md`. |
| Streaming reader and viewport priority | P5 | Done for local files (`vpng-source`, `vpng-view`); the HTTP Range variant is P13. |
| Python bindings | P9 | The same engine, not a second implementation of it. |
| Adaptive tiling and advanced codecs | P7 and P2 | Variable tile sizes and the reserved stable ids 4&#8211;6. A research codec is promoted only after a specification update, a version bump, a migration policy and compatibility tests. |
| Graphical UI editor | P11 | A desktop front end over `vpng-view` and `vpng-editing`. No graphical editor exists in this repository yet. |
| WebAssembly (WASM) package | P12 | A `wasm32` build of the decoder, the viewer and the editor. |
| HTTP Range-request cloud streaming | P13 | The format already allows it: every payload has its own offset and length, so no format change is needed. |
| Multi-layer / mask compositing | P14 | Several named grids in one file; a format revision. |
| GPU texture block support (BC7/ASTC) | P15 | A separate, explicitly named payload kind; never a silent substitute for a lossless tile. |

## 21.2 The release plan

The release plan runs from **v0.2 to v0.23** and is maintained in
`docs/ROADMAP_STATUS.md` &#167;9, where the full table lives. Its rules:

-   A release version is a product version. The container's own version
    (header bytes 8&#8211;9) moves only when the layout changes, and the
    release plan says which releases move it.
-   **Compression work is present in every release from v0.2 to v0.23.**
    It is a standing line of work — a codec, a measurement, a tuning pass
    or a format revision that opens a new compression axis — not one
    project deferred to the end.
-   No dates are promised and no version is claimed. A version ships when
    its gate passes and the evidence is recorded.
-   The invariants of &#167;0.1 hold in every release: exact lossless
    reconstruction, deterministic output, compatibility with the existing
    golden fixtures, no silent binary-layout change, and no lossy
    fallback. A research codec never reaches a file.

## Final instruction

Build VPNG as a real, independently specified, lossless, tiled image
format with random access and transactional editing. Treat the 10 MB
limit as a measurable target rather than a promise. Prioritize
correctness, safety, reproducibility, and bounded memory before
compression-ratio optimization.

At every stage, preserve raw evidence and report exactly what works,
what does not, and what remains untested.
