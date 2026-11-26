# P11 — The WebAssembly package

**Date:** 2026-11-26
**Deliverables:** the new crate `crates/vpng-wasm` (`registry.rs`, `ffi.rs`);
the `MemorySource` in `crates/vpng-source/src/source.rs`; the JavaScript
wrapper `wasm/vpng.js`; the module's test suite `wasm/test/vpng.test.mjs`;
the export check `wasm/check_exports.mjs`; the measurement driver
`benchmarks/wasm/run_wasm_bench.py` and its Node half
`wasm/run_wasm_bench.mjs`; the native counterpart
`crates/vpng-validation/src/bin/vpng-region-bench.rs`; the raw result file
`benchmarks/p11_wasm_results.json`; the package's own `wasm/README.md`; and
the decision record `docs/decisions/ADR-0017-wasm-abi.md`.
**Status:** the case **ran** — it did not have to be reported BLOCKED. Every
figure below is read out of the raw JSON, and `check_wasm()` in
`benchmarks/verify_reports.py` binds the quoted figures to it.

---

## 1. What this is

Phase C step 3 of the agreed plan: **the browser's copy of the engine.**

The module is a **decoder**, and it is the *same* decoder the command line
runs. There is no codec, no predictor, no entropy coder and no container
parser in `crates/vpng-wasm`: it calls `vpng-decoder`, `vpng-codecs` and
`vpng-source`, and it validates a container through
`vpng_decoder::parse_container_parts` — the one shared implementation of the
acceptance rules that the in-memory decoder, the streaming reader and the
HTTP reader all use.

The consequence is the whole point: **the browser and the command line cannot
disagree about what a file means**, because there is only one implementation
to disagree with. The evidence is measured rather than asserted: the module's
canonical digest for a region is the command line's canonical digest for the
same region, on **750 of 750** corpus images, on both the whole-file path and
the range path.

**The binary layout did not move.** Not one byte of the 64-byte header or the
40-byte directory entry changed. The module reads what is there.

### 1.1 The ABI is a raw `extern "C"`

The module exports plain functions with `i32`, `i64` and `u32` arguments and
returns. There is no `wasm-bindgen`, no generated glue and no build-time code
generation: `cargo build --release --target wasm32-unknown-unknown -p vpng-wasm`
is the whole toolchain, and the workspace gains **no dependency** for this
package.

The conventions are the command line's, so a browser's error handling can be
written against the documented codes:

| | |
|---|---|
| A handle | `i32`; zero is always an error |
| A return value | `i32` or `i64`; zero is success and a negative value is the negative of the error's exit code — `-4` is a corrupt file, `-3` is an unsupported feature |
| An error message | fetched with `vpng_error_length` and `vpng_error_copy` |
| A pointer | `u32`, **checked against the module's real memory size** before use: a bad pointer from JavaScript is an error, not undefined behaviour |
| A byte length or offset | `i64`, because a 16K RGBA16 region is 2 GiB |

### 1.2 Two ways to open a container

* `vpng_open` takes the **whole container** in the module's memory. The
  ordinary case.
* `vpng_open_parts` takes the **header and the directory** alone. The caller
  then asks for a tile's payload with `vpng_tile_offset` and
  `vpng_tile_length`, fetches that range, and hands the bytes back with
  `vpng_decode_payload`. `vpng_region_tiles` says which tiles a rectangle
  touches, so the caller never works out the tiling itself.

The second is the path a container in an object store takes, and it is the
same shape as the HTTP Range reader in ADR-0016: the layout is known after
two ranges, and after that the reader fetches exactly the payloads it needs.

The payload's CRC32C is verified against the directory before the codec sees
it, in both paths, by the same `vpng_decoder::decode_tile_payload` the native
reader uses.

---

## 2. How it was measured

### 2.1 The host

`stable-x86_64-pc-windows-gnu`, rustc 1.100.0, an Intel Core i5-1235U with
**10 physical cores and 12 logical processors**, 7.7 GB of physical memory.
CPython 3.13.0 and Node.js v24.18.0. The CLI binary, the native benchmark
and the WebAssembly module are all the **release** profile. Tile size 256,
**4 encoder workers**, one image at a time.

### 2.2 The corpus

The same **750-image corpus** the P5, P6, P7, P8, P9 and P10 reports used:
**300** photographs, **100** pixel-art images, **100** scans, **150** textures
and **100** UI screenshots. The canonical pixels come from the same cache, so
the bytes the module is given are the same bytes the other reports encoded.

### 2.3 The method

For each image:

1. The image is encoded locally with the real `vpng` binary, from the cached
   canonical pixels, and the file's size is recorded.
2. A **1024×1024** viewport at the origin is decoded by the **native**
   reader — `vpng-region-bench`, which uses the same `RangeReader` and the
   same `decode_region_from` the module uses — five times, and the median and
   the canonical digest are recorded. The same is done a second time with the
   reader pointed at the **file on disk**, so the streaming cost is separate
   from the decode cost.
3. The file is handed to the **WebAssembly module**, which measures two
   paths:
   * the **whole-file** path: the container is copied into the module's
     memory and the region is decoded from there;
   * the **range** path: the module is opened with the header and the
     directory alone, it says which tiles the viewport touches, and only
     those payloads are fetched.
4. All three canonical digests — native, whole-file, range — are compared.
   A module that produced different pixels would fail the run instead of
   being reported as a win.

### 2.4 The percentile rules

The same rules the other reports use: the median is the middle sample, or the
mean of the two middle samples when the count is even; the p95 is the
**nearest-rank** percentile; the standard deviation is the **sample** one,
with n − 1 in the denominator.

### 2.5 What is in the byte figures

This measurement reports **bytes moved**, not process memory.

* **Included** in the range path's byte count: the 64-byte header, the
  directory (the entries plus its four-byte checksum trailer), and the
  compressed payload of every tile the viewport touches. Nothing else.
* **Excluded**: the decoded pixels, the module's heap overhead, the
  JavaScript wrapper's own allocations, the process working sets of the
  driver, of Node and of the native benchmark, and the file on disk.

The raw JSON carries the same statement under `settings.memory_scope`.

### 2.6 What is in the timing figures

Each timing is **in-process** and excludes the disk read of the container,
except the native `file` column, which includes it. The four columns are:

| Column | What it times |
|---|---|
| `native_memory` | the region decode through `RangeReader` over bytes already in memory — the exact counterpart of the module's whole-file path |
| `native_file` | the region decode through `RangeReader` over the file on disk, with the disk read inside the measurement |
| `wasm_whole` | the copy of the container into the module's memory **plus** the region decode |
| `wasm_range` | the open from the header and the directory, the fetch of the tiles the viewport touches, and the decode |

The raw JSON carries the same statement under `settings.timing_scope`.

### 2.7 What is in the memory figures

**There are none in this report.** This milestone did not measure process
memory and makes no memory claim. §2.5 says exactly what the byte counts
include and exclude.

---

## 3. What was measured

### 3.1 The headline

| | |
|---|---|
| Images measured | **750** of 750 |
| Regions identical to the native read — whole-file path | **750** of 750 |
| Regions identical to the native read — range path | **750** of 750 |
| Total object size | **453,767,460** bytes |
| Bytes moved to decode a 1024×1024 viewport, range path | **234,420,672** (**51.661%** of the corpus) |
| Bytes read by the native streaming reader | **234,420,672** — **exactly equal** |
| Range requests | 4,518 |
| Header + directory bytes | 408,760 (**0.0901%** of the corpus) |

The total object size, **453,767,460** bytes, is the published stable
baseline's, which is a useful cross-check: the measurement encoded the corpus
with the same bytes the P5 and P9 reports did.

The third row is the structural claim: **the module's range path moves
exactly the bytes the native streaming reader reads**, byte for byte and
image by image. It is not "about the same"; it is the same number, and
`check_wasm()` asserts the equality for every image.

### 3.2 The big picture

The 16K photograph — **124,197,035** bytes — is the clearest case:

| | bytes | requests | share of the object |
|---|---|---|---|
| Header and directory | 81,668 | — | **0.0658%** |
| A 1024×1024 viewport, range path | **1,039,409** | 16 | **0.8369%** |
| The whole object | 124,197,035 | — | 100% |

So a browser that wants a 1024×1024 window of a 16K photograph fetches
**0.84%** of the file. That is the same number the HTTP Range reader
measured — **1,039,409** bytes, **0.8369%** — which is the cross-check that
matters: the module's range path and the HTTP reader agree because they run
the same code.

The 16K photograph's viewport touches **16** tiles and moves **957,741** bytes
of payload.

### 3.3 The corpus as a whole — the unfavourable part

The **51.661%** headline is **not** a good number, and it is kept as it is:
**592 of the 750 images are smaller than the 1024×1024 viewport**, so for
them the whole file is the viewport and the reader fetches everything. Those
592 images account for 151,851,134 of the 234,420,672 bytes.

The subset where the range path can actually save something is the **158
images larger than the viewport**:

| | bytes | share |
|---|---|---|
| Their objects | 301,916,326 | 100% |
| Their viewport fetches | **82,569,538** | **27.3485%** |

Per category:

| Category | Images | Object bytes | Header + directory | Viewport fetch | Viewport share |
|---|---|---|---|---|---|
| photos | 300 | 234,827,986 | 158,960 | 104,628,889 | 44.5555% |
| pixel_art | 100 | 365,091 | 11,200 | 365,091 | 100.0000% |
| scans | 100 | 67,871,353 | 66,040 | 56,808,900 | 83.7009% |
| textures | 150 | 133,872,854 | 93,040 | 60,736,581 | 45.3689% |
| ui | 100 | 16,830,176 | 79,520 | 11,881,211 | 70.5947% |

The pixel-art row is the honest worst case: every pixel-art image in the
corpus is smaller than the viewport, so the reader fetches 100% of it. The
**header-and-directory** share is the part that is always cheap, and it is
**0.0901%** of the corpus overall.

### 3.4 The timings — the module is slower, and it is kept

| | median | p95 | mean | stdev | min | max |
|---|---|---|---|---|---|---|
| Native, bytes in memory | **26.569 ms** | **113.9322 ms** | 33.825952 ms | 36.276848 ms | 0.0014 ms | 212.4112 ms |
| Native, file on disk | 26.63475 ms | 111.4865 ms | 33.217343 ms | 35.367133 ms | 0.0237 ms | 206.1177 ms |
| WebAssembly, whole file | **29.51415 ms** | 125.0923 ms | 37.156452 ms | 41.662517 ms | 0.0066 ms | 285.7832 ms |
| WebAssembly, range path | **29.1638 ms** | 124.4701 ms | 36.897349 ms | 41.550719 ms | 0.0077 ms | 265.5815 ms |

**The module is slower than the native reader.** The whole-file path is
**1.1108×** the native in-memory median and the range path is **1.0950×** the
native file median. At the p95 the whole-file path is **1.0980×** the native
in-memory p95.

That is the honest number and no attempt is made to hide it: a WebAssembly
build is not the native build, and the same source compiled to WebAssembly
runs the same algorithms with more overhead. What the milestone claims is
**equality of pixels**, not equality of speed.

The one case where the difference is large is the 16K photograph, and it is
the whole-file path:

| 16K photograph | median |
|---|---|
| Native, bytes in memory | 187.6339 ms |
| Native, file on disk | 126.8556 ms |
| WebAssembly, whole file | **278.7018 ms** |
| WebAssembly, range path | **128.1419 ms** |

The whole-file path copies **124,197,035** bytes into the module's linear
memory before it decodes anything, and that copy is the cost. The range path
copies 1,039,409 bytes and lands within **1.01×** of the native streaming
reader — which is the practical argument for the range path on a large
picture.

### 3.5 The refusals and the bounds

The thirteen tests in `wasm/test/vpng.test.mjs` pin the behaviour against the
**real `vpng` binary**, not against a pinned constant:

| Test | What it pins |
|---|---|
| `the pixels are the command line pixels, byte for byte` | the module's canonical digest equals the command line's |
| `a region in the middle of the picture is the command line region` | four rectangles, including the last pixel |
| `the layout the module reports is the layout the command line reports` | width, height, tile count, level count and the format |
| `a preview pyramid is visible to the module` | the v0.2 levels are visible |
| `the parts path decodes the same pixels as the whole path` | the two ways in agree |
| `the range path paints the same region as the whole path` | the range path's pixels are the whole path's pixels |
| `a payload with a broken checksum is refused` | the CRC32C is checked before the codec sees the payload |
| `a payload of the wrong length is refused` | the directory's length is enforced |
| `a file that is not a container is refused` | the shared validation refuses it |
| `a pointer outside the module memory is refused, not obeyed` | the bounds check works |
| `every payload checksum verifies` | `vpng_check` |
| `the SHA-256 the module computes is the known SHA-256` | the digest function |
| `the module reports the container format it understands` | the format version |

Eight more tests in `crates/vpng-wasm/src/registry.rs` cover the registry on
the host, where the WebAssembly build is not available: the layout, the
region equality, the parts path, the two refusals, the closed handle, the
region outside the image, and the canonical shape.

A fourteenth check, `wasm/check_exports.mjs`, is not a test but a
**contract**: it asserts that all **28** documented exports are really
exported, that the module exports a linear memory at all, that closing
handle zero **fails** rather than succeeds, and that the format version the
module reports is the one this repository builds. A renamed or dropped
function cannot pass silently.

---

## 4. The twelve questions

**Q1 — Was the corpus actually run, or blocked?**
**Run, not blocked.** All **750** images were encoded locally, decoded by the
native reader, and decoded twice by the WebAssembly module — once through the
whole file and once through the range path. The raw file is
`benchmarks/p11_wasm_results.json`.

**Q2 — How many times was each case run, and what do the percentiles mean?**
The native path was run **five times** per image and its median is reported;
the module's paths are single runs, timed in-process. The corpus is the
repetition across images. The median is the middle sample, or the mean of the
two middle samples when the count is even. The p95 is the **nearest-rank**
percentile. The standard deviation is the sample one, with n − 1 in the
denominator.

**Q3 — What is the baseline, and was it measured in the same run?**
**The native reader, and it was measured in the same run.** Every image was
decoded by `vpng-region-bench` through the same `RangeReader` and the same
`decode_region_from` the module uses, five times, twice over (bytes in memory
and the file on disk), in the same driver pass that ran the module. The P5
corpus baseline was checked too: the total object size is **453,767,460**
bytes, the published stable baseline's.

**Q4 — Do the module's files change the pixels?**
**No.** The canonical pixel digest of the region the module decoded equals
the native digest on **750 of 750** images, on the whole-file path **and** on
the range path. The reconstruction is **exact**.

**Q5 — What does the module buy?**
**It puts the engine where the pictures are.** The range path moves exactly
the bytes the native streaming reader moves — **234,420,672** for the corpus,
**1,039,409** for the 16K photograph's viewport, **0.8369%** of that file —
and it does so with **4,518** range requests and **16** requests for the 16K
viewport.

**Q6 — How fast is it?**
**Slower than the native reader, by about 10%.** The whole-file path's
median is 29.51415 ms against the native 26.569 ms (**1.1108×**); the range
path's is 29.1638 ms against the native streaming 26.63475 ms (**1.0950×**).
The 16K photograph's range path is 128.1419 ms against 126.8556 ms
(**1.01×**), while its whole-file path is 278.7018 ms against 187.6339 ms
because it copies the whole 124,197,035-byte container first. No speed claim
is made beyond those measurements.

**Q7 — What does it cost?**
**One range request per tile the viewport touches**, plus the header and the
directory. Over the corpus: 4,518 range requests for 4,518 tiles.

**Q8 — Does the module move the default path's bytes or version?**
**No.** The files it reads are the published stable ones — the corpus total
is **453,767,460** bytes and every file reports minor version **1**. This
milestone added **no format revision**: the header and the directory entry
keep their exact layouts, and the module writes no container at all.

**Q9 — Is the format still binary-compatible?**
**Yes, and the milestone did not touch it.** No byte of the 64-byte header or
the 40-byte directory entry moved. The module validates a container through
the same `vpng_decoder::parse_container_parts` the native readers use.

**Q10 — Are the research codecs still out of reach?**
**Yes.** The shared metadata validation refuses a directory entry that names
an unimplemented codec, and the three promoted codecs may only be named by a
file whose minor version says so. The module uses that same validation, so
it accepts exactly the files the in-memory decoder accepts.

**Q11 — What is in the memory figures?**
**There are none in this report.** §2.5 says exactly what the byte counts
include and exclude: the header, the directory and the fetched payloads are
counted; the decoded pixels, the module's heap, the wrapper's allocations and
every process working set are not. This milestone did not measure process
memory and makes no memory claim.

**Q12 — What is not measured, and what remains untested?**
The limits are in §6. The four that matter most: the module was run in
**Node**, not in a browser, so the figures say nothing about a browser's
JavaScript engine, its garbage collector or its memory limits; the range path
fetched its bytes from an **in-process slice**, not from a network, so the
network cost of the 4,518 requests is not in any figure here; **one viewport
size** (1024×1024) and **one position** (the origin) were measured; and the
module **does not encode and does not edit** — it is a decoder, and a browser
editor is a separate piece of work.

---

## 5. Reproducing the evidence

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# One: the crate's own tests, on the host.
cargo test -p vpng-wasm

# Two: the module's test suite, against the real command line.
cargo build --release --target wasm32-unknown-unknown -p vpng-wasm
cargo build --release -p vpng-cli
node --test wasm/test/vpng.test.mjs
node wasm/check_exports.mjs

# Three: the corpus comparison.
cargo build --release -p vpng-validation --bin vpng-region-bench
python benchmarks\wasm\run_wasm_bench.py --sample 750

# Four: the whole Rust suite, and the check that every quoted figure matches
# the raw data.
cargo test --workspace --all-targets
python benchmarks/verify_reports.py
```

---

## 6. What this does not claim

* **No claim that the module is fast.** It was **slower** than the native
  reader at the median and at the p95 — by about 10%. The claim is equality
  of pixels, not equality of speed.
* **No claim about a browser.** The module was measured in Node. A browser's
  engine, its garbage collector and its memory limits are not measured here.
* **No claim about a network.** The range path fetched its bytes from an
  in-process slice. The 4,518 requests' network cost is not in any figure.
* **No claim that the whole corpus saves bytes.** The 51.661% figure is
  dominated by the 592 images that are smaller than the viewport. The honest
  statement is the subset: the **158** images larger than the viewport
  fetched **27.3485%** of their bytes.
* **No claim that the module can encode or edit.** It is a decoder. The
  editing engine is native and a browser editor over it is a later piece of
  work.
* **No claim that the format changed.** It did not. This milestone is a
  decoder, not a revision.
* **No claim that VPNG now beats PNG.** The P5 report's unfavourable
  comparison with PNG, WebP and JPEG XL still stands unchanged.

---

## 7. The acceptance gates

| # | Gate | Result | Evidence |
|---|---|---|---|
| 1 | The corpus actually ran, at its full size | **PASS** | 750 of 750 images, two module paths and two native paths each; `benchmarks/p11_wasm_results.json` |
| 2 | The module's pixels are the native pixels | **PASS** | 750 of 750 canonical digests match on the whole-file path |
| 3 | The range path's pixels are the native pixels | **PASS** | 750 of 750 canonical digests match on the range path |
| 4 | The range path moves only what the viewport needs | **PASS** | the 16K photograph's viewport costs **1,039,409** bytes of **124,197,035** — **0.8369%**, in **16** requests |
| 5 | The module and the native reader move the same bytes | **PASS** | **234,420,672** bytes on both sides, exactly, for every image |
| 6 | A corrupt payload is refused | **PASS** | `a payload with a broken checksum is refused`: the CRC32C is checked before the codec sees it |
| 7 | A bad pointer is refused, not obeyed | **PASS** | `a pointer outside the module memory is refused, not obeyed` |
| 8 | There is one decoder, not two | **PASS** | the module calls `vpng-decoder`, `vpng-codecs` and `vpng-source`; the validation is one shared `vpng_decoder::parse_container_parts` and the tile payload path is one shared `vpng_decoder::decode_tile_payload` |
| 9 | The format is unchanged | **PASS** | no format revision; the 64-byte header and the 40-byte entry are untouched; every file read is minor version 1 |
| 10 | The default path's bytes do not move | **PASS** | the corpus total is the published baseline's **453,767,460** |
| 11 | Every quoted figure is bound to the raw data | **PASS** | `check_wasm()` in `benchmarks/verify_reports.py` |
| 12 | The binary layout is unchanged | **PASS** | the milestone adds a decoder; it writes no container at all |

---

## 8. What is next

The WebAssembly package is in, measured and documented. What it leaves open:

* **Encoding and editing in the browser.** The module is a decoder. A browser
  that can *write* VPNG needs the encoder, and a browser editor needs
  `vpng-editing` — both are separate pieces of work, and the editing engine's
  memory behaviour has to be re-established on the WebAssembly target before
  it can be claimed there.
* **The viewer.** `vpng-view` is native. A browser viewer would sit on the
  module's `vpng_region_tiles` and `vpng_decode_payload`, which is exactly
  the surface the range path uses.
* **Then** layers and masks (which is a format revision), GPU-ready payloads
  and the graphical UI; and **last** the v0.2 → v0.23 release cadence and the
  v1.0 freeze.
