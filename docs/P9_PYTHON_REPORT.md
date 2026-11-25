# P9 — the Python bindings

**Date:** 2026-11-25
**Deliverables:** the PyO3 extension `crates/vpng-python`; the Python package
`python/vpng`; the shared PNG/raw crate `crates/vpng-image`; the build
definition `pyproject.toml` (maturin); the test suite `python/tests`
(45 tests); the measurement drivers `benchmarks/python/run_python_bench.py`
and `benchmarks/python/run_python_memory.py`; and the raw result files
`benchmarks/p9_python_results.json` and
`benchmarks/p9_python_memory_results.json`.
**Status:** the case **ran** — it did not have to be reported BLOCKED. Every
figure below is read out of the raw JSON, and `check_python()` in
`benchmarks/verify_reports.py` binds the quoted figures to it.

---

## 1. What this is

Phase C step 1 of the agreed plan: **expose the stable engine to other
applications**, starting with Python.

The binding is a **thin wrapper**, not a second implementation. Every pixel
operation it performs is the same Rust code the `vpng` command line runs, so
the two cannot drift apart about what a file means. The Python side owns no
codec, no pixel semantics and no container logic; it owns the *shape* of the
API — the argument names, the exception hierarchy, the transaction boundary —
and calls straight into the engine.

That design has a consequence worth stating plainly, because it is the whole
argument of this report: **if the binding were allowed to differ, the
difference would show up as bytes.** And it does not, on any of the 750
images in the corpus.

### 1.1 The surface

| Function | What it does |
|---|---|
| `encode(input, output, …)` | encode a PNG or raw file into a `.vpng` |
| `encode_pixels(pixels, width, height, output, …)` | encode canonical pixel bytes directly, with no intermediate file |
| `decode(input, output, …)` | write a `.vpng` back out as a PNG or raw file |
| `decode_to_bytes(path, …)` | the whole picture as canonical bytes |
| `decode_region(path, output, x, y, width, height)` | write one region out as a PNG or raw file |
| `verify(input, output, …)` | decode and compare with the original pixels |
| `check(path)` | the container's own integrity check (every payload CRC32C) |
| `info(path)` | the container's metadata |
| `open_document(path, *, cache_bytes=…, history_bytes=…)` | a `Document` for region reads and edits |

`Document` gives `read_region`, `write_region`, `width`, `height`, `format`,
`tile_count`, `is_dirty`, `edit_count`, `dirty_tiles` and `edit()`, which
opens a `Transaction`. A transaction has `write_region`, `commit` and
`rollback`; **a transaction whose block ends without committing is rolled
back**, so an abandoned edit cannot leak into the file.

`open_document` takes the two budgets the editing engine already has:
`cache_bytes` bounds the decoded-tile cache and `history_bytes` bounds the
undo history. Both are the engine's defaults unless the caller says
otherwise, and both are the engine's own accounting — the binding does not
keep a second copy of anything.

`encode_pixels` is the entry point for a caller that already has the pixels
in memory. The bytes it takes are **canonical**: rows tightly packed with no
padding, and a 16-bit sample little-endian — the same order the format
stores and the same order `decode_to_bytes` returns. It exists so the
benchmark in §3 could compare the two paths on **identical pixels** rather
than on two different PNG decodes.

### 1.2 The exceptions are a real hierarchy

Every failure carries the engine's stable error label, and every class is
*also* a builtin Python exception, so either `except` clause catches it:

| Class | Also a | Raised for |
|---|---|---|
| `VpngError` | `Exception` | the base of everything below |
| `InvalidArgumentError` | `ValueError` | a bad argument |
| `UnsupportedFeatureError` | `NotImplementedError` | a feature the reader does not have |
| `CorruptFileError` | `ValueError` | a malformed file |
| `ResourceLimitError` | `MemoryError` | a limit was hit |
| `SizeLimitExceededError` | `ValueError` | the size ceiling was missed |
| `VerificationFailedError` | `ValueError` | a round trip did not match |
| `VpngIOError` | `OSError` | an I/O failure |
| `InternalError` | `RuntimeError` | an invariant broke |

### 1.3 Two deliberate refusals

* **`lossless=False` raises.** VPNG has no lossy mode and no lossy fallback, so
  the argument is refused rather than quietly ignored. A refused encode leaves
  **no partial file** behind.
* **The extended codec set is opt-in.** The binding's default codec set is
  the stable one, exactly as the CLI's is, so the published bytes do not
  move.

### 1.4 One source of truth for PNG

The PNG and raw conversion moved out of the CLI into a new shared crate,
`crates/vpng-image`. The CLI and the binding now both call it, so they cannot
disagree about what a PNG means. The three unit tests moved with the code, so
the Rust test count did not change.

### 1.5 One source of truth for the codec names

The binding needs to turn a codec's *name* into its id, and so does the CLI.
Rather than write that table twice, it lives in `vpng-codecs` as
`codec_id_by_name` and `codec_set_by_name`, and **both** front ends call it.
The table is the CLI's own — the same names, the same decimal aliases, the
same rejections — so the CLI's accepted surface did not move by one name
when the table was lifted out.

---

## 2. How it was measured

### 2.1 The host

`stable-x86_64-pc-windows-gnu`, rustc 1.100.0, an Intel Core i5-1235U with
**10 physical cores and 12 logical processors**, 7.7 GB of physical memory.
CPython 3.13.0. The extension is the **release** profile, built with
`maturin develop --release`; the CLI binary is the release profile too. Tile
size 256, **4 encoder workers**, one image at a time.

### 2.2 The corpus

The **same 750 images** the P5 and P8 corpus reports used — photographs 300,
screenshots 100, pixel art 100, scans 100, textures 150 — read from
`benchmarks/corpus_manifest.json` and canonicalised to the same raw pixels.
The denominator is the **canonical pixel bytes**, the same number the other
reports divide by, so the three can be read side by side.

### 2.3 What the run does, per image

1. The **CLI** encodes the image from the canonical raw file, with
   `--input-format raw --width W --height H --format F --tile-size 256
   --workers 4`.
2. The **Python extension** encodes the same pixels through
   `encode_pixels`, with the same tile size and the same worker count.
3. The two files are compared **byte for byte** by SHA-256.
4. The Python-written file is checked with **`vpng check`** (the header and
   directory checksums and every payload's CRC32C).
5. The Python-written file is **verified** with **`vpng verify`**, which
   decodes every tile and compares the result with the original pixels — so a
   file that is smaller but not lossless fails the run instead of being
   reported as a win.
6. The Python extension encodes the image **again with one worker instead of
   four**, and the digest is compared, so the worker count is shown not to
   move the bytes.

### 2.4 The rules

* **Every image is measured.** All 750, with no sampling and no exclusions.
* **The median.** The middle sample, or the mean of the two middle samples
  when the count is even.
* **The p95.** The **nearest-rank** percentile: the smallest sample at or
  above the 95th percentile.
* **The standard deviation.** The sample one, with n − 1 in the denominator.
* **The bytes are checked.** Every file carries its SHA-256, and the stable
  run is compared with the P5 baseline file by file.

### 2.5 What is in the memory figures

Two different memory numbers appear in this report, and they measure
different things.

**The edit's memory** — the one that is compared with the roadmap's target —
was measured in a **fresh child process**, so the figure is the edit's own
and is not contaminated by an earlier encode in the same process.

* **Included:** the whole child process — the CPython interpreter, the
  extension module, the decoded-tile cache, the undo history, the tile
  payloads in flight, the rewritten payloads waiting to be written, the
  container metadata, the stacks and the allocator's own bookkeeping.
* **Excluded:** the `.vpng` file on disk, the operating system's page cache,
  and the memory of other processes.

**The benchmark process's memory** — **809,496,576 B** — is a *different*
measurement and is **not** a target claim. It is the peak of the whole
driver across all 750 images, and it is dominated by the 16K picture's
398,131,200 canonical bytes plus the encoder's own buffers. The **encode**
path materialises the whole image, exactly as the CLI does; §6 says so.

---

## 3. Results

### 3.1 The binding writes the same bytes as the command line — 750 of 750

| | Result |
|---|---:|
| Images measured | **750** |
| Byte-identical to the CLI | **750** |
| Differing | **0** |
| `vpng check` failures | **0** |
| `vpng verify` failures | **0** |
| Digests that moved when the worker count changed | **0** |
| Total bytes | **453,767,460** |

One SHA-256 per image, on both sides, and they match every time. The total is
the published stable baseline's, to the byte.

### 3.2 The bytes are the published baseline's

The sharpest case is the 16K photograph: the Python file is
**124,197,035 bytes** with the SHA-256
`d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf` — **the
P5.2 baseline's tile-256 digest**, the same file the preview milestone and
the codec-promotion milestone both measured against. All 750 files report
minor version **1**: the binding does not move the default path's version
either.

| Category | n | Bytes | Canonical bytes | Ratio |
|---|---:|---:|---:|---:|
| **overall** | **750** | **453,767,460** | **1,450,363,330** | **0.312865** |
| photos | 300 | 234,827,986 | 592,678,520 | 0.396215 |
| pixel_art | 100 | 365,091 | 3,849,799 | 0.094834 |
| scans | 100 | 67,871,353 | 199,028,456 | 0.341013 |
| textures | 150 | 133,872,854 | 387,416,819 | 0.345553 |
| ui | 100 | 16,830,176 | 267,389,736 | 0.062942 |

### 3.3 The timings

| Path | median | p95 | mean | stdev | max |
|---|---:|---:|---:|---:|---:|
| CLI, wall clock | 126.9835 ms | 285.174 ms | 176.588527 ms | 1050.911477 ms | 28,581.094 ms |
| Python, wall clock | **124.369 ms** | 270.628 ms | 167.661888 ms | 945.640446 ms | 25,688.263 ms |
| Python, the encoder's own timer | **116.0 ms** | 261.0 ms | 157.22 ms | 907.762921 ms | 24,637 ms |

The two paths are the same size to within the noise — the Python wall median
is 2.1% **below** the CLI's. That is the expected result and not a claim of
a speed-up: the CLI's wall clock includes the process start and the process
exit, and the Python path does not, so the two are comparable in size but not
identical in what they count. The Python wall clock includes the FFI call,
the buffer copy into the extension and the file write.

The distributions are wide — a stdev around 1 second against a median around
0.13 seconds — because one image in the corpus is 16K and 398,131,200 bytes
of pixels. The spread is the same on both sides, which is the point.

### 3.4 The editing path stays inside the memory target

A **localized edit** of the 16K photograph through the Python API — open the
file, read a 1024×1024 region, write it back, commit — measured in a fresh
child process:

| | |
|---|---:|
| Edit elapsed | 687.935 ms |
| Tiles rewritten | 16 |
| Tiles copied payload-for-payload | 2,024 |
| Peak working set | **44,249,088 B** |
| Target | 67,108,864 B (64 MB) |

**Within target**, at 66.0% of the budget. The 2,024 untouched tiles were
copied **payload for payload** — they were not re-encoded — and only the 16
tiles the edit touched were rewritten.

### 3.5 A defect the measurement found

The memory run failed the first time, with a message that was wrong:

```
ValueError: the pixel block is 3145728 bytes but 4194304 were expected for
1024x1024 rgba8
```

The picture was **RGB**, not RGBA. The binding's raw-byte region write
assumed **RGBA8** instead of asking the document for its own format, so a
write to a grayscale or RGB document was rejected for the wrong reason — and
a write that happened to be the right size would have been interpreted with
the wrong number of channels.

The binding now reads the bytes in the document's format, and the Python
suite pins it with a grayscale document and a wrong-sized block. This is the
kind of defect the corpus exists to find: the unit tests had not caught it
because they only ever used RGBA.

---

## 4. The twelve questions

**Q1 — Was the corpus actually run, or blocked?**
**Run, not blocked.** All **750** images were encoded twice — once by the CLI,
once by the Python extension — checked, verified and digested, and every one
was encoded a third time with a different worker count. The raw file is
`benchmarks/p9_python_results.json`.

**Q2 — How many times was each case run, and what do the percentiles mean?**
**Three times per image: two encodes and a worker-count check.** The corpus is
the repetition. The median is the middle sample, or the mean of the two middle
samples when the count is even. The p95 is the **nearest-rank** percentile:
the smallest sample at or above the 95th percentile. The standard deviation
is the sample one, with n − 1 in the denominator. The rules are in the raw
JSON and in `benchmarks/python/run_python_bench.py`.

**Q3 — What is the baseline, and was it measured in the same run?**
**The command line, and it was measured in the same run.** Every image was
encoded by both paths inside the same driver pass, and the two SHA-256
digests were compared there and then. The P5 corpus baseline was checked
too: the total is **453,767,460** bytes and the 16K photograph's digest is
the P5.2 tile-256 baseline's.

**Q4 — Do the Python-written files change the pixels?**
**No.** `vpng verify` decodes every tile of every Python-written file and
compares the result with the original pixels: **0 verify failures** across
all 750. Every file also passed the container's own integrity check: **0
check failures**. The reconstruction is **exact**.

**Q5 — What does the binding buy?**
**The same engine, reachable from Python, at no cost in bytes.** The file is
byte-identical to the command line's, so there is no "Python version" of the
format to keep in step with the CLI's. The savings are in the plumbing: no
subprocess, no temporary PNG, and `encode_pixels` takes the pixels the
caller already has.

**Q6 — How fast is it?**
**The same order as the command line, and no slower.** The Python wall
median is **124.369 ms** against the CLI's **126.9835 ms**; the encoder's own
timer reports a median of **116.0 ms**. The distributions overlap closely
(stdev around 1 second on both, against a median around 0.13 seconds,
because one image in the corpus is 16K).

**Q7 — What does the binding cost?**
**The extension's own footprint, and nothing else.** The engine's memory
behaviour is unchanged, because it is the same engine: the edit's peak
working set is **44,249,088 B** against the 67,108,864-byte target. The
**encode** path still materialises the whole image, exactly as the CLI does;
the binding did not make that worse and did not make it better.

**Q8 — Does the binding move the default path's bytes or version?**
**No. Not one byte and not one version.** All 750 files report minor version
**1**, and the total is the published stable baseline's. The binding's
default codec set is the stable one, and `lossless=False` is refused rather
than ignored.

**Q9 — Is the format still binary-compatible?**
**Yes, and the binding did not touch the format.** No byte of the 64-byte
header or the 40-byte directory entry moved. The binding reads and writes
the same container the CLI does, and the file it writes passes the CLI's own
`check` and `verify`.

**Q10 — Are the research codecs still out of reach?**
**Yes.** The binding exposes the same stable codec sets — `stable` and
`extended` — and nothing else. The research ids `0x80..=0xFF` cannot be
named by the encoder and are rejected by the decoder with
`UnsupportedFeature`, exactly as they are on the Rust side. The binding adds
no third set and no escape hatch.

**Q11 — What is in the memory figures?**
**The edit's figure is the whole child process**, and §2.5 says exactly what
that includes and what it excludes: the CPython interpreter, the extension
module, the decoded-tile cache, the undo history, the payloads in flight, the
container metadata, the stacks and the allocator's bookkeeping are **in**;
the file on disk, the operating system's page cache and other processes are
**out**. That figure — **44,249,088 B** against the 67,108,864-byte target —
is the one that answers the roadmap's question, and it was measured in a
fresh process so it is the edit's own. The benchmark process's peak,
**809,496,576 B**, is a different measurement of a different thing and is not
a target claim.

**Q12 — What is not measured, and what remains untested?**
The limits are in §6. The four that matter most: the **decode** path's
timings were not measured separately (only the encode was timed); the
**cache and history budgets** are exposed (`open_document(cache_bytes=…,
history_bytes=…)`) but the measurement in §3.4 used the defaults, so the
budget's *effect* on the edit's memory is untested; the binding was measured
on **one** Python version (3.13) on **one** platform, though the wheel is
built against the stable ABI and CI runs the suite on another version; and
there is **no NumPy path**, so a caller that wants an array has to convert
the bytes itself.

---

## 5. Reproducing the evidence

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# One: build the extension and run its test suite.
python -m venv .venv
.venv\Scripts\python -m pip install "maturin>=1.5,<2.0"
.venv\Scripts\python -m maturin develop --release
.venv\Scripts\python -m unittest discover -s python/tests -v

# Two: the corpus comparison, against the release CLI.
cargo build --release -p vpng-cli
.venv\Scripts\python benchmarks\python\run_python_bench.py --sample 750

# Three: the editing path's memory, on the 16K picture.
.venv\Scripts\python benchmarks\python\run_python_memory.py

# Four: the Rust suite, and the check that every quoted figure matches the
# raw data.
cargo test --workspace --all-targets
python benchmarks/verify_reports.py
```

---

## 6. What this does not claim

* **No claim that the binding is faster.** The medians are within the noise
  of each other, the CLI's wall clock includes a process start and exit that
  the Python path does not, and the corpus is dominated by one very large
  image. The claim is that the two are **the same size**.
* **No claim about the encode path's memory.** The encode materialises the
  whole picture, in Python exactly as in the CLI. The memory target was
  measured on the **edit** path, which is where the roadmap states it.
* **No claim that the binding works on every platform.** It was measured on
  Windows on x86-64 with CPython 3.13. The wheel is built against the stable
  ABI so it *should* load on every CPython from 3.8 onwards, and CI builds
  and tests it on Linux with another version — but this report's figures are
  from one host.
* **No claim that the API is final.** The surface is small and the exception
  hierarchy is settled, but the names will move if the rest of the ecosystem
  (WASM, HTTP Range, layers) says they should. Nothing in the **format**
  depends on them.
* **No claim that VPNG now beats PNG.** The corpus ratios in §3.2 are VPNG's
  own, and the P5 report's unfavourable comparison with PNG, WebP and JPEG
  XL still stands.
* **No claim that the format is frozen.** This milestone added no format
  revision at all.

---

## 7. The acceptance gates

| # | Gate | Result | Evidence |
|---|---|---|---|
| 1 | The corpus actually ran, at its full size | **PASS** | 750 of 750 images, three encodes each; `benchmarks/p9_python_results.json` |
| 2 | The binding writes the same bytes as the command line | **PASS** | 750 of 750 SHA-256 digests match; 0 differ |
| 3 | Every Python-written file is lossless | **PASS** | `vpng verify` on all 750: **0 failures** |
| 4 | Every Python-written file passes the container's own check | **PASS** | `vpng check` on all 750: **0 failures** |
| 5 | The default path's bytes and version do not move | **PASS** | the total is the published baseline's **453,767,460**; all 750 files are minor version 1; the 16K digest is the P5.2 baseline's |
| 6 | The worker count does not change the bytes | **PASS** | 750 of 750 digests match with one worker instead of four |
| 7 | The edit stays inside the memory target | **PASS** | **44,249,088 B** against 67,108,864 B, measured in a fresh process; 2,024 tiles copied payload-for-payload |
| 8 | There is no second implementation of the format | **PASS** | the binding calls the engine; the PNG semantics live in one shared crate (`crates/vpng-image`) that the CLI and the binding both use |
| 9 | A lossy request is refused, never ignored | **PASS** | `lossless=False` raises; the Python suite pins it, and a refused encode leaves no partial file |
| 10 | The failures are typed and catchable | **PASS** | every class is also a builtin Python exception; the suite pins `ValueError`, `OSError` and the rest |
| 11 | Every quoted figure is bound to the raw data | **PASS** | `check_python()` in `benchmarks/verify_reports.py` |
| 12 | The binary layout is unchanged | **PASS** | no format revision in this milestone; the 64-byte header and the 40-byte entry are untouched |

---

## 8. What is next

The Python binding is in, measured and documented. What it leaves open:

* **The rest of the ecosystem.** The WASM package, HTTP Range streaming,
  layers and masks, GPU-ready payloads and the graphical UI are the rest of
  Phase C, and the same "the binding must not diverge from the engine"
  discipline applies to each of them.
* **The encode path still materialises the whole picture.** The streaming
  encoder exists on the Rust side (`vpng encode` uses it), but the binding's
  `encode` reads the input file into memory first, so it cannot benefit from
  it. Exposing a streaming entry point is the obvious next improvement, and
  it is a real one: it is the difference between a 16K encode that needs
  ~1 GB and one that needs a bounded amount.
* **A NumPy path.** The roadmap's wording says "NumPy arrays"; this milestone
  returns canonical bytes, and the conversion is the caller's to make.
* **Then the release cadence**, from v0.2 to v0.23, and the **v1.0 freeze**
  last.
