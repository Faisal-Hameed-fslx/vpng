# P5 — Streaming and parallel decoding report

**Date:** 2026-11-19
**Deliverable:** the interactive engine in `crates/vpng-view`, its shared
reader in `crates/vpng-source`, its shared cache in `crates/vpng-cache`,
the acceptance benchmark in `crates/vpng-validation`, and the two raw
result files `benchmarks/p5_view_results.json` (16K) and
`benchmarks/p5_view_results_small.json` (2K).
**Status:** the acceptance test ran at **16K RGBA8** — it did not have to
be reported BLOCKED. Every figure below is read out of the raw JSON, and
`benchmarks/verify_reports.py` checks the quoted figures against it.

---

## 1. What was built

P5's objective is *interactive performance*, not raw throughput: open an
enormous image, navigate it, edit a small region and save it, without ever
loading the whole image into memory. The six stages in `docs/P5_DESIGN.md`
are now done:

| Stage | What it is | Where it lives |
|---|---|---|
| 1 — the streaming reader | the container metadata parsed once, tile payloads read on demand, with a byte counter | `crates/vpng-source` |
| 2 — the bounded scheduler | the request queue: `(class, distance, sequence)` priority, a hard bound, cancellation by viewport generation | `crates/vpng-view/src/scheduler.rs` |
| 3 — the parallel decoder | the worker pool: one reader per worker, the lock never held across a decode, one decode per tile | `crates/vpng-view/src/pool.rs`, `file_decoder.rs` |
| 4 — the predictive prefetch | the viewport, the prefetch ring, the direction-of-travel preference | `crates/vpng-view/src/viewport.rs` |
| 5 — the acceptance benchmark | one run, one report, every figure against the sequential baseline | `crates/vpng-validation/src/view_bench.rs`, `src/bin/vpng-view-bench.rs` |
| 6 — this report | the evidence, and the limits of it | `docs/P5_REPORT.md` |

Two other pieces changed along the way:

* The bounded, byte-budgeted tile cache moved out of `vpng-editing` into
  **`crates/vpng-cache`**, because two engines need it. The editor holds
  the tiles a session has touched; the viewer holds the tiles a viewport
  has shown. Each engine has its **own** cache instance with its own
  budget, so the viewer's prefetch cannot evict the editor's dirty tiles.
  `vpng_editing::TileCache` still resolves to the same type.
* The scheduler's `dispatch` and `peek` drop a request whose generation is
  stale instead of handing it to a worker, which matters once a generation
  can be advanced from another thread.

## 2. How it was measured

**The host.** `stable-x86_64-pc-windows-gnu`, rustc 1.100.0, 12 logical
CPUs, 7.7 GB of physical memory. The benchmark is built with the release
profile; the tests use the default profile.

**The file.** A 16384 × 16384 **RGBA8** image: **1,073,741,824 canonical
pixel bytes** (1 GiB), written tile by tile by
`vpng_validation::stream::write_streaming` — the writer never holds the
image — into a **140,431,266-byte** file with **4,096 tiles** of 256 × 256
pixels. The write took **152,660 ms**. The tile pattern is the same
deterministic pattern the P4 memory validation used.

**The baseline.** Every latency figure has a baseline measured in the
**same run, on the same host, against the same file**: the sequential
path — one tile at a time, no cache, no threads. A latency number without
that baseline would mean nothing here.

**The memory figures.** Included in the heap peak: the viewing cache's
decoded tiles, the payload buffers in flight, the container metadata and
the JSON report being assembled. Included in the resident-set peak: all of
that, plus the binary, the stacks and the allocator's metadata. Excluded
from both: the file on disk, the operating system's page cache, the memory
of other processes, and the benchmark driver's own memory. This scope is
recorded in the raw JSON as `memory_scope`, next to the numbers.

**The two workloads.** The 16K case is the acceptance case. The 2048 ×
2048 case is kept because it **disagrees** with the 16K case about
prefetch, and a finding that only holds at one size should say so. The
unfavourable run is kept, not edited out.

## 3. Results

### 3.1 Opening the file and the first viewport

| Figure | 16K (8 workers) | 2K (4 workers) |
|---|---|---|
| Open: header + directory parse | 13.66 ms | 15.09 ms |
| First viewport, parallel | **63.56 ms** | 17.70 ms |
| First viewport, sequential baseline | 118.96 ms | 29.07 ms |
| Parallel vs sequential | **1.87× faster** | 1.64× faster |
| Viewport | 1024 × 1024 px = 16 tiles | 512 × 512 px = 4 tiles |
| Tiles decoded | **16 of 4,096** | 4 of 64 |
| Only the viewport's tiles decoded | **true** | true |
| Tiles identical to the sequential decoder | **16 / 16** | 4 / 4 |
| Tile payload bytes read by the workers | 540,792 | 125,480 |
| Header + directory bytes, read once | 163,908 | 2,628 |
| Baseline's total bytes read | **704,700** | 128,108 |
| Cache peak | 4,194,304 B | 1,048,576 B |
| Heap peak | 12,245,903 B | 4,852,184 B |
| Resident-set peak | 15,015,936 B | 7,630,848 B |

The two I/O columns are the same number seen from two sides. The pool's
workers read 540,792 bytes of tile payload; the header and the directory
are read **once** and shared. The sequential baseline reads the same
header and directory plus the same payloads: 163,908 + 540,792 =
**704,700 bytes**, exactly what it reports. There is **no read
amplification** in the parallel path — it reads the same bytes as the
sequential one, and it reads them once.

### 3.2 Panning

The pan scenario sweeps the viewport 12 steps to the right across the
image (512 px per step at 16K), each pan advancing the viewport generation
so the old viewport's queued work is dropped.

| Figure | no prefetch | prefetch, radius 2 |
|---|---|---|
| Total panned latency | 415.94 ms | **318.32 ms** |
| Mean per pan | 34.66 ms | 26.53 ms |
| Worst pan | 72.34 ms | 67.82 ms |
| Tiles decoded | 104 | 132 |
| Visible hit rate | 0.4583 | **0.5260** |
| Visible hits / misses | 88 / 104 | 101 / 91 |
| Speculative hit rate | — | 0.2179 |
| Cancellation rate | 0.0000 | **0.6222** |
| Requests submitted / dispatched | 104 / 104 | 442 / 141 |
| Results discarded (viewport moved on) | 0 | 1 |
| Results **recovered** (wanted again) | 0 | **83** |
| Cache evictions | 0 | 4 |
| Resident-set peak | 35,987,456 B | 47,153,152 B |

Two things are worth naming explicitly:

* **The cancellation is doing its job.** 442 requests were submitted and
  141 were dispatched: the rest were dropped when the viewport moved on.
  The cancellation rate is reported, not hidden.
* **83 results were recovered.** A tile that is being decoded for an old
  viewport is often wanted again by the new one. The pixels do not depend
  on the viewport, so those results are kept rather than thrown away —
  and the keep is counted separately from the discard, so a report cannot
  confuse "the user panned away" with "the decoder is broken".

### 3.3 Does prefetch help? — the parameter sweep

The policy's parameters are exposed, so the benchmark measures them rather
than guessing. Each row is the **same pan sequence over the same file in
the same process**.

**16K, 12 pans, 8 workers:**

| Radius | Total latency | Visible hit rate | Tiles decoded | Requests cancelled |
|---|---|---|---|---|
| 0 (no prefetch) | 316.91 ms | 0.4583 | 104 | 0 |
| 1 | 353.80 ms | 0.4948 | 154 | 7 |
| **2** | **262.72 ms** | 0.5208 | 135 | 269 |
| 3 | 264.20 ms | 0.5365 | 139 | 592 |

**2K, 6 pans, 4 workers:**

| Radius | Total latency | Visible hit rate | Tiles decoded | Requests cancelled |
|---|---|---|---|---|
| **0 (no prefetch)** | **59.25 ms** | 0.4167 | 14 | 0 |
| 1 | 88.06 ms | 0.5000 | 24 | 11 |
| 2 | 111.74 ms | 0.5000 | 30 | 71 |
| 3 | 104.63 ms | 0.4583 | 28 | 175 |

**The finding is not a flat "prefetch helps".**

* At **16K**, prefetch at radius 2 reduces the total panned latency from
  316.91 ms to **262.72 ms (−17.1%)** and raises the visible hit rate from
  0.4583 to 0.5208. The best radius is **2**.
* At **2K**, prefetch **does not help**: no prefetch is the fastest at
  59.25 ms, and every radius is slower. The best radius is **0**.
* In both cases the visible hit rate goes **up** with prefetch. The cost
  is the speculative work itself, which competes with the visible tiles
  for the workers. On the small image there is little speculative work to
  gain from, because the tiles are cheap and mostly already resident, so
  the competition shows up as a loss.

The policy therefore ships with the parameters exposed and the measurement
recorded, **not** with a claim that prefetching is always a win. A caller
should measure its own workload; the benchmark does exactly that.

### 3.4 The localized edit

The P4 copy-on-write property must not weaken under P5, so the acceptance
test edits a 64 × 64 region of the 16K file, saves, and reopens.

| Figure | Value |
|---|---|
| Region | 64 × 64 at (8192, 8192) |
| Tiles dirtied | 1 |
| Tiles rewritten by the commit | **1** |
| Tiles copied verbatim | **4,095** |
| Untouched payloads byte-identical | **true** |
| Pixels read back exactly | **true** |
| Read | 9.29 ms |
| Commit | 296.35 ms |
| Saved file size | 140,431,735 bytes |

One tile of 4,096 is re-encoded; the other 4,095 payloads keep their bytes
exactly. The pixels come back exactly.

### 3.5 Memory against the target

The target is the same one P4 uses: **≤ 64 MB (67,108,864 bytes)** of CPU
working memory for localized work on a 16K image.

| Figure | Value |
|---|---|
| Process peak resident set, whole 16K run | **47,910,912 B (45.7 MiB)** |
| Process peak heap, whole 16K run | 43,764,750 B (41.7 MiB) |
| First viewport: heap peak | 12,245,903 B |
| First viewport: resident-set peak | 15,015,936 B |
| Pan with prefetch: resident-set peak | 47,153,152 B |
| Viewing cache budget | 33,554,432 B (32 MiB) |
| Largest cache peak | 33,554,432 B (at budget) |

The whole 16K run — writing the file, opening it, panning across it with
prefetch, editing it and saving it — peaks at **47.9 MB of resident set**,
against the 64 MB target. The largest single resident-set peak inside a
scenario is 47,153,152 B. The cache reaches its own budget of 32 MiB and
stops there: it is a bound, not a suggestion.

### 3.6 Tile throughput, derived

These are derived from the raw figures above, and are labelled as derived:

* First viewport, parallel: 16 tiles / 63.56 ms ≈ **252 tiles/s**.
* First viewport, sequential baseline: 16 tiles / 118.96 ms ≈ **135 tiles/s**.
* Pan with prefetch: 132 tiles / 318.32 ms ≈ **415 tiles/s** of wall-clock
  time, including the time spent waiting for the viewport.

The throughput figures are not a separate measurement and should not be
read as one: they are the same numbers divided by time.

### 3.7 The small case, in full

The 2K run is kept whole, including the figures that do not flatter the
engine: a 2,110,291-byte file with 64 tiles, 4 workers, a 512 × 512
viewport and 6 pans.

| Figure | no prefetch | prefetch, radius 2 |
|---|---|---|
| Total panned latency | 56.24 ms | 103.42 ms |
| Mean per pan | 9.37 ms | 17.24 ms |
| Worst pan | 13.63 ms | 30.50 ms |
| Tiles decoded | 14 | 33 |
| Cancellation rate | 0.0000 | 0.6316 |
| Results recovered | 0 | 17 |
| Resident-set peak | 9,641,984 B | 16,371,712 B |
| Whole run: peak resident set | 16,605,184 B | |
| Whole run: peak heap | 12,235,424 B | |

## 4. The twelve questions

**Q1 — Was the 16K case actually run, or blocked?**
**Run, not blocked.** A 16384 × 16384 **RGBA8** image — 1 GiB of canonical
pixels — was written, opened, panned across, edited and saved, on this
host, in one process. No 16K figure is inferred from a smaller image. The
raw file is `benchmarks/p5_view_results.json`; the whole run took
155,628 ms.

**Q2 — Does opening the file avoid allocating the whole image?**
**Yes.** The header and the directory are parsed once (13.66 ms for 4,096
tiles); no payload is read until a tile is asked for. The heap peak for
the first viewport is **12,245,903 bytes** and the resident-set peak is
**15,015,936 bytes**, on an image whose canonical pixels are
**1,073,741,824 bytes**. An engine that allocated the whole image would
have needed a gigabyte.

**Q3 — How long does the first viewport take, and against which baseline?**
**63.56 ms**, against **118.96 ms** for the sequential baseline measured
in the same run on the same file: **1.87× faster**. The baseline is the
path that existed before this milestone — one tile at a time, no cache, no
threads — not a strawman.

**Q4 — Are only the viewport's tiles decoded?**
**Yes, and it is counted, not asserted.** The 1024 × 1024 viewport covers
16 tiles of the 4,096-tile image; the pool decoded **exactly 16**, and the
report's `only_viewport_tiles_decoded` flag is true. The prefetch is off
for this scenario, so nothing speculative is counted in it.

**Q5 — What does a pan cost, and what was cancelled?**
**26.53 ms mean, 67.82 ms worst**, with prefetch at radius 2; 34.66 ms
mean without it. The cancellation rate is reported as **0.6222**: 442
requests were submitted, 141 dispatched, and the rest were dropped when
the viewport moved on. One decoded result was discarded because the user
panned away before it landed; **83 were kept** because the new viewport
wanted the same tiles.

**Q6 — Does prefetch help?**
**At 16K, yes; at 2K, no — and both runs are kept.** The radius sweep is
in §3.3. The best radius at 16K is 2 (a 17.2% reduction in total panned
latency); the best radius at 2K is 0, because prefetching only adds work
that competes with the visible tiles. The parameters are exposed and the
sweep is the measurement that chooses them.

**Q7 — What is the cache hit rate, and what is counted?**
Two hit rates are reported, because one would be misleading. The
**visible hit rate** counts only the tiles the user is looking at: 0.4583
without prefetch, 0.5260 with it. The **speculative hit rate** counts the
prefetch guesses: 0.2179. The combined figure (0.2965) is in the raw JSON
too, but it is not the interesting number — a prefetch that misses is a
guess that has not come true, not a failure of the cache.

**Q8 — What are the peak resident-set and heap figures, and what is in
them?**
Peak resident set **47,910,912 B**, peak heap **43,764,750 B**, for the
whole 16K run. The scope is stated in §2 and in the raw JSON's
`memory_scope`: the viewing cache, the payloads in flight, the container
metadata and the report; the binary, the stacks and the allocator's
metadata. Excluded: the file on disk, the page cache, other processes, and
the driver.

**Q9 — Is the ≤ 64 MB target met?**
**Yes, on both measures, at 16K.** 47.9 MB of resident set and 43.8 MB of
heap against the 67,108,864-byte target. The cache reaches its own 32 MiB
budget and stops there.

**Q10 — Does the edit still rewrite only the tiles it touches, and are the
pixels exact?**
**Yes.** One tile dirtied, one rewritten, 4,095 copied verbatim, the
untouched payloads byte-identical, and the pixels read back exactly. The
P4 property is unchanged: P5 does not weaken it.

**Q11 — Are the parallel results byte-identical to the sequential
decoder's?**
**Yes, measured two ways.** The benchmark decodes the same 16 tiles with
the pool and with the ordinary decoder and compares them: **16 of 16
identical, 0 different** (and 4 of 4 at 2K). The unit test
`the_pool_produces_the_same_pixels_as_the_sequential_decoder` does the
same for a 512 × 384 file with 48 tiles through four workers. The parallel
engine changes *when* a tile is decoded, never *what* it decodes.

**Q12 — What is not measured, and what remains untested?**
The limits are listed in §6. The three that matter most: the sweep is one
pan sequence in one direction on one host; the run-to-run variance between
separate scenario invocations is visible in the raw files and is not
smoothed away; and no claim is made about any other image format, tile
size, viewport size or worker count than the ones recorded in the
parameters.

## 5. Reproducing the evidence

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# The 16K acceptance case.
cargo run --release -p vpng-validation --bin vpng-view-bench -- `
  --width 16384 --height 16384 --format rgba8 --tile-size 256 `
  --workers 8 --viewport-width 1024 --viewport-height 1024 `
  --prefetch-radius 2 --pans 12 --pan-step 512 `
  --cache-bytes 33554432 --queue-capacity 256 `
  --work-dir "E:\VPNG\benchmarks\work" `
  --output "E:\VPNG\benchmarks\p5_view_results.json" --keep-file

# The small case, which disagrees with the 16K case about prefetch.
cargo run --release -p vpng-validation --bin vpng-view-bench -- `
  --width 2048 --height 2048 --tile-size 256 --workers 4 `
  --viewport-width 512 --viewport-height 512 --pans 6 --pan-step 256 `
  --cache-bytes 16777216 --queue-capacity 128 `
  --work-dir "E:\VPNG\benchmarks\work" `
  --output "E:\VPNG\benchmarks\p5_view_results_small.json"

# The tests, and the check that every quoted figure matches the raw data.
cargo test --workspace --all-targets
python benchmarks/verify_reports.py
```

The file the 16K case uses is 140,431,266 bytes on disk and is deleted at
the end of a run unless `--keep-file` is passed. `--reuse-file` skips the
write when the file is already there.

## 6. What this does not claim

* **No general "P5 is faster" claim.** The speedup is 1.87× on the first
  viewport of this file, with 8 workers, against the sequential baseline
  measured in the same run. It is not a claim about other images, other
  worker counts, or other hardware.
* **No claim that prefetch always helps.** §3.3 keeps the run where it
  hurts.
* **No claim about arbitrary content.** The tile pattern is the same
  deterministic pattern the P4 validation used, not a photograph, not
  noise, not a screenshot.
* **No claim of statistical confidence.** Each scenario is one run. The
  variance between separate invocations is visible in the raw files — the
  standalone no-prefetch pan (**415.94 ms**) and the sweep's radius-0 run
  (**316.91 ms**) are the same measurement on the same file and differ by
  roughly a quarter — so the comparisons in this report are taken from the
  sweep, which runs the radii back to back in one process.
* **No claim about the other pixel formats or tile sizes.** The
  acceptance case is RGBA8 with 256 × 256 tiles. The parallel-vs-sequential
  byte-identity test covers one file; the format-and-geometry matrix for
  parallel decode is not done.
* **No graphical editor.** The engine is a library. There is no window,
  no pan-and-zoom widget, and no UI in this repository.

## 7. The acceptance gates

| # | Gate | Result | Evidence |
|---|---|---|---|
| 1 | Write a 16K RGBA8 image without decoding it whole | **PASS** | 1 GiB canonical, 140,431,266-byte file, written tile by tile |
| 2 | Open it and request a small viewport; the full image is never allocated | **PASS** | heap peak 12,245,903 B, resident-set peak 15,015,936 B on a 1 GiB image |
| 3 | Both memory figures stay inside the 64 MB target | **PASS** | 47.9 MB resident set, 43.8 MB heap |
| 4 | Only the tiles the viewport needs are decoded | **PASS** | 16 of 4,096, counted |
| 5 | Pan rapidly; obsolete requests are cancelled and the rate is reported | **PASS** | cancellation rate 0.6222; 1 discarded, 83 recovered |
| 6 | The prefetch parameters are reported and the hit rates compared | **PASS** | the sweep in §3.3, including the losing radii |
| 7 | Edit a small region, save, reopen; the pixels are exact and the untouched payloads are byte-identical | **PASS** | 1 rewritten, 4,095 copied, pixels exact |
| 8 | First-viewport latency, throughput, hit rate, peak RSS and memory compliance are reported | **PASS** | §3.1, §3.5, §3.6 |

## 8. Next phase

* **P6 — the multiresolution preview pyramid.** P5's engine reads one
  level of an image at a time. A pyramid lets a reader pick a level, and
  the format needs a second grid of tiles to do it: a format revision, not
  a library change.
* **P7 — adaptive tiling.** The grid is still fixed at 256 × 256. A
  spatially adaptive layout would change both the compression and the
  shape of a viewport's tile set, and the prefetch policy would have to be
  re-measured on it.
* **The parallel-decode coverage matrix.** The byte-identity test covers
  one file. The format-and-geometry matrix for parallel decode is the
  next correctness item, and it is cheap to run.
* **The prefetch policy under other workloads.** The two runs here show
  the policy is workload-dependent. The honest next step is a small
  workload suite, not a larger prefetch radius.
