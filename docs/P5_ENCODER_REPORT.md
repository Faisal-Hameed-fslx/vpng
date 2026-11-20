# P5.1 — Parallel and streaming encoder report

**Date:** 2026-11-20
**Deliverable:** the parallel encoder in `crates/vpng-encoder`
(`parallel.rs`, `source.rs`, `stream.rs`), the acceptance benchmark
`vpng-encode-bench` in `crates/vpng-validation`, and the raw result file
`benchmarks/p5_encode_results.json`.
**Status:** the 16K RGBA8 acceptance case **ran** — it did not have to be
reported BLOCKED. Every figure below is read out of the raw JSON, and
`benchmarks/verify_reports.py` checks the quoted figures against it.

---

## 1. What was built

Phase A, Step 1 of the agreed plan: make the encoder fast enough that every
later benchmark is affordable, and prove that making it fast does not change
the file.

The encoder's per-tile work was already independent. `select_payload` sees
only the tile's own pixels, its own geometry and the encode options — never
a neighbouring tile, never the image, never another thread's state. So the
same tile always produces the same payload. That is the fact that makes
parallel encoding safe, and this milestone is built on it rather than on a
new codec or a new file format.

Three pieces:

| Piece | Where | What it is |
|---|---|---|
| **The pixel source** | `crates/vpng-encoder/src/source.rs` | A `PixelSource` trait: hand the encoder one tile at a time, so the whole image is never in memory. `CanonicalImage` implements it. |
| **The parallel driver** | `crates/vpng-encoder/src/parallel.rs` | Chunks of tiles are encoded by worker threads; a coordinator consumes the chunks **in tile order** and hands each tile to the sink. |
| **The streaming writer** | `crates/vpng-encoder/src/stream.rs` | Writes a file from a `PixelSource`, with a bounded window of chunks in flight, to a temporary name that is renamed over the target. |

The serial encoder was **refactored, not replaced**. `encode()` now goes
through the same tile loop and the same assembly function as the parallel
and streaming paths, so there is exactly one place in the codebase where
offsets, checksums and the header are computed.

## 2. How it was measured

**The host.** `stable-x86_64-pc-windows-gnu`, rustc 1.100.0, an Intel
Core i5-1235U with **10 physical cores and 12 logical processors**, 7.7 GB
of physical memory. The benchmark is built with the release profile.

**The file.** A 16384 × 16384 **RGBA8** image — **1,073,741,824 canonical
pixel bytes** (1 GiB) — generated one tile at a time, never held in memory.
The tiles are 256 × 256 pixels, so the image is 4,096 tiles. The finished
file is **140,431,266 bytes**.

That file size is not new: it is exactly the size of the file the P5
viewing benchmark wrote from the same pattern. Two independent writers
produced the same number of bytes.

**The baseline.** One worker. It is the same code path, the same file and
the same machine, so the speedup is a measurement rather than a comparison
between two different programs.

**The memory figures.** Included in the heap peak: the tiles in flight, the
encoded payloads waiting to be written, the container metadata and the
benchmark's own report. Included in the resident-set peak: all of that, plus
the binary, the stacks and the allocator's metadata. Excluded: the file on
disk, the operating system's page cache, other processes, and the benchmark
driver's own memory.

**The scenario order matters, and the reason is stated in the raw JSON.**
The streaming scenario runs **first** and its memory is sampled before the
agreement scenario runs. The operating system's resident-set high-water mark
cannot be reset, so a scenario that ran earlier would put a floor under
every later measurement — and the agreement scenario deliberately holds a
whole image in memory, because that is what the *batch* encoder does.

## 3. Results

### 3.1 The scaling

| Workers | Encode time | Speedup vs one worker | File size | SHA-256 |
|---|---|---|---|---|
| 1 | 128962.99 ms | 1.00 | 140,431,266 | `9cc5a3e266b53553bc0e871be471857cc8387085f71727d18dffa4c533e1fa62` |
| 2 | 70712.86 ms | 1.82 | 140,431,266 | `9cc5a3e266b53553bc0e871be471857cc8387085f71727d18dffa4c533e1fa62` |
| 4 | 49681.19 ms | 2.60 | 140,431,266 | `9cc5a3e266b53553bc0e871be471857cc8387085f71727d18dffa4c533e1fa62` |
| 8 | 30174.82 ms | 4.27 | 140,431,266 | `9cc5a3e266b53553bc0e871be471857cc8387085f71727d18dffa4c533e1fa62` |

**Eight workers cut the 16K encode from 128962.99 ms to 30174.82 ms — 4.27×.**

The scaling is **sub-linear**, and the report does not dress that up:

* 1 → 2 workers: 1.82×
* 2 → 4 workers: 1.40×
* 4 → 8 workers: 1.65×

The per-tile work is CPU-bound, so the limit is the machine. This is a
10-core mobile U-series part that was also running the rest of the
toolchain; it is not a 12-core server with nothing else to do. **The claim
is "more workers are faster, and the bytes do not change", not "it scales
linearly."** A clean scaling measurement needs an idle dedicated host, and
that is Phase A Step 2's job.

### 3.2 Byte identity — the central claim

**Every worker count wrote the same file.** The raw JSON's
`all_worker_counts_identical` is true, and the proof is the digest, not the
assertion:

```
9cc5a3e266b53553bc0e871be471857cc8387085f71727d18dffa4c533e1fa62
```

One digest, four worker counts, one file size. If a single byte differed,
the digests would differ.

This is not a happy accident of the benchmark's pattern. It holds by
construction:

* the per-tile work is independent, so a tile's payload does not depend on
  which worker encoded it or when;
* the coordinator consumes the chunks **in tile order**, so the payloads are
  laid out exactly as the serial encoder lays them out;
* the serial encoder, the parallel encoder and the streaming writer all
  build the container through **one** assembly function, so they cannot
  disagree about offsets, checksums or the header.

The test suite checks the same rule over **144 encode cases** — all eight
v0.1 pixel formats × six representative sizes × three tile geometries — and
over worker counts from 1 to 16 and chunk sizes from 1 to 64, with the batch
encoder, the parallel encoder and the streamed file compared byte for byte.

### 3.3 The batch encoder and the streaming writer agree

| Figure | Value |
|---|---|
| Case | 4096 × 4096 RGBA8, 256 × 256 tiles |
| Batch encoder (holds the whole image) | 15633.55 ms |
| Streaming writer, 4 workers (holds neither) | 4029.73 ms |
| Batch file size | 27,333,060 |
| Streaming file size | 27,333,060 |
| Batch SHA-256 | `5654d436780a76f30a7ecdc752aa8d82d58ae2a11b71ab4c687bdc616b403503` |
| Streaming SHA-256 | `5654d436780a76f30a7ecdc752aa8d82d58ae2a11b71ab4c687bdc616b403503` |
| Identical | **true** |

The batch encoder holds the image (67 MB of pixels) and the whole output in
memory. The streaming writer holds neither. They produce the same file.

### 3.4 Memory against the target

The target is the same one P4 and P5 use: **≤ 64 MB (67,108,864 bytes)** of
CPU working memory.

| Figure | Value |
|---|---|
| Streaming scenario: peak resident set | **19,296,256 B (18.4 MiB)** |
| Streaming scenario: peak heap | 13,198,153 B (12.6 MiB) |
| Target | 67,108,864 B (64 MiB) |

The streaming encoder's whole 16K run — generating 1 GiB of pixels, encoding
4,096 tiles and writing a 140 MB file — peaks at **19.3 MB of resident set**,
well inside the target. The memory does not grow with the image: a bounded
window of chunks is in flight, whatever the image size.

The agreement scenario's figures are reported separately, because the batch
encoder deliberately holds a whole image: it peaks at 128,991,232 B of
resident set. That is the batch encoder's cost, not the streaming writer's,
and the two are never added together.

## 4. The twelve questions

**Q1 — Was the 16K case actually run, or blocked?**
**Run, not blocked.** A 16384 × 16384 RGBA8 image — 1 GiB of canonical
pixels — was streamed and encoded four times on this host, in one process.
No figure here is inferred from a smaller image. The raw file is
`benchmarks/p5_encode_results.json`.

**Q2 — Does the encoder avoid allocating the whole image?**
**Yes.** The encoder pulls one tile at a time from a `PixelSource`. The
whole 16K run peaks at **19,296,256 bytes** of resident set on a 1 GiB
image. The pixel buffer is reused by each worker, so the encoder allocates
once per worker rather than once per tile.

**Q3 — Does the streaming writer produce exactly the file the batch
encoder produces?**
**Yes, byte for byte.** The digests are equal
(`5654d436780a76f30a7ecdc752aa8d82d58ae2a11b71ab4c687bdc616b403503`) and
the file sizes are equal (27,333,060). A separate test in
`crates/vpng-validation/src/stream.rs` checks the same equality against
the P4 memory-validation writer, which is the writer the P4 memory figures
were measured with — so that writer can be replaced by the library's later
without the file changing.

**Q4 — Does the worker count change the file?**
**No.** Four worker counts, one digest,
`9cc5a3e266b53553bc0e871be471857cc8387085f71727d18dffa4c533e1fa62`. The
test suite extends this to 144 encode cases and to worker counts from 1 to
16.

**Q5 — Does the chunk size change the file?**
**No.** The test suite varies the chunk size from 1 to 64 tiles with every
worker count from 1 to 16, and every combination produces the same bytes.

**Q6 — How much faster is it?**
**4.27× at eight workers** on the 16K case: 128962.99 ms → 30174.82 ms.
The scaling is sub-linear on this host (§3.1), and no linear-scaling claim
is made.

**Q7 — What is the baseline?**
**One worker of the same code**, on the same file, in the same process,
measured in the same run. Not a different program and not another machine.

**Q8 — What are the peak resident-set and heap figures, and what is in
them?**
**19,296,256 B** of resident set and **13,198,153 B** of heap for the
whole streaming scenario. Included: the tiles in flight, the payloads
waiting to be written, the container metadata, the report, the binary, the
stacks and the allocator's metadata. Excluded: the file on disk, the page
cache, other processes and the driver. The scope is recorded in the raw
JSON's `memory_scope` next to the numbers.

**Q9 — Is the ≤ 64 MB target met?**
**Yes, on both measures.** 19.3 MB of resident set and 13.2 MB of heap
against the 67,108,864-byte target.

**Q10 — Is the file written atomically?**
**Yes.** The streaming writer writes to a temporary name in the target's own
directory and renames it over the target, so a reader never sees a
half-written image. A failed encode removes the temporary file and leaves
the target untouched: two tests cover the size-limit failure and a failing
pixel source, and both assert that no file survives.

**Q11 — Is the binary format unchanged?**
**Yes.** The format is not touched at all. The new code produces files
through the same `Header`, `Directory` and `DirectoryEntry` structures the
old encoder used, and the five golden fixture tests pass unchanged, with the
same SHA-256 digests.

**Q12 — What is not measured, and what remains untested?**
The limits are in §6. The three that matter most: the scaling figure is one
run on a busy mobile part rather than a clean measurement on an idle
dedicated host; the pattern is synthetic rather than photographic; and the
streaming writer has been exercised against the batch encoder, but not
against a *disk failure* partway through a write.

## 5. Reproducing the evidence

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# The 16K acceptance case.
cargo run --release -p vpng-validation --bin vpng-encode-bench -- `
  --width 16384 --height 16384 --format rgba8 --tile-size 256 `
  --workers 1,2,4,8 --chunk-tiles 4 `
  --agreement-width 4096 --agreement-height 4096 `
  --work-dir "E:\VPNG\benchmarks\work" `
  --output "E:\VPNG\benchmarks\p5_encode_results.json"

# The tests, and the check that every quoted figure matches the raw data.
cargo test --workspace --all-targets
python benchmarks/verify_reports.py
```

The benchmark writes the 16K file once per worker count and deletes it
afterwards; `--keep-file` leaves it.

## 6. What this does not claim

* **No linear-scaling claim.** 4.27× on eight workers is the measured
  figure on a 10-core mobile part that was also running the rest of the
  toolchain. It is not a claim about a dedicated server.
* **No statistical confidence.** Each worker count is one run. The
  run-to-run variance is visible: an earlier run of the same benchmark on
  the same host measured the one-worker case at 176812 ms against this
  run's 128962.99 ms, before the scratch-buffer reuse was added.
* **No claim about photographic content.** The tile pattern is the same
  deterministic gradient the P4 and P5 benchmarks use, not a photograph.
  The *bytes* do not depend on the content — that is what the 144-case
  matrix and the codec-selection tests cover — but the *timings* do.
* **No claim that the encoder is fast.** 30174.82 ms for a 1 GiB image is
  not fast in the absolute sense; the claim is only that more workers make
  it faster and do not change the file.
* **No claim about a disk failure partway through the write.** The
  temporary file is removed on an encode failure and the target is left
  untouched, but a failure *during* the final rename is the operating
  system's business, not a property this milestone claims.
* **No consolidation of the two writers yet.** The P4 memory-validation
  writer in `crates/vpng-validation/src/stream.rs` and the library's
  streaming writer produce the same file — a test asserts it — but the
  P4 memory figures were measured with the former, so it is left alone
  until that matrix is re-run.

## 7. The acceptance gates

| # | Gate | Result | Evidence |
|---|---|---|---|
| 1 | One worker and N workers produce the same file, byte for byte | **PASS** | one digest across 4 worker counts; 144 encode cases; worker counts 1–16 |
| 2 | The chunk size does not change the file | **PASS** | chunk sizes 1–64 with every worker count |
| 3 | The streaming writer produces exactly the batch encoder's file | **PASS** | equal digests and equal sizes |
| 4 | The 16K image is encoded without ever being in memory | **PASS** | 19,296,256 B resident-set peak on a 1 GiB image |
| 5 | The memory stays inside the 64 MB target | **PASS** | 19.3 MB resident set, 13.2 MB heap |
| 6 | More workers make the encode faster | **PASS** | 128962.99 ms → 30174.82 ms, 4.27× |
| 7 | The binary format is unchanged | **PASS** | the golden fixture tests pass with the same digests |
| 8 | A failed encode leaves no file behind | **PASS** | two tests: the size-limit failure and a failing source |

## 8. Next phase

* **Phase A, Step 2 — the measurement foundation.** Multi-run benchmarks
  with median and p95, photographic tiles, and a latency matrix over the
  tile size, the viewport size and the worker count. The scaling figure in
  this report is one run on a busy host; that is exactly what Step 2
  replaces.
* **Phase A, Step 3 — the 750-image corpus pipeline.** The encoder is now
  4.27× faster, which is what makes a 750-image corpus affordable, and the
  parallel encoder makes the corpus run reproducible in reasonable time.
* **The streaming encoder in the CLI.** `vpng encode` still uses the batch
  encoder, so it needs the whole image in memory. It should gain a
  streaming mode that never does.
