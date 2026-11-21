# P6 and P7 — the preview pyramid and the adaptive partition

**Date:** 2026-11-21
**Deliverables:** the format revision `docs/VPNG_SPEC_0_2.md` and
`docs/decisions/ADR-0014-levels-and-partitions.md`; the benchmark harness
`vpng-preview-bench` in `crates/vpng-validation`; the corpus runner
`benchmarks/corpus/run_adaptive.py`; and the raw result files
`benchmarks/p6_preview_results.json`, `benchmarks/p7_adaptive_results.json`
and `benchmarks/p6_memory_results.json`.
**Status:** both cases **ran** — neither had to be reported BLOCKED. Every
figure below is read out of the raw JSON, and `check_preview()` and
`check_adaptive()` in `benchmarks/verify_reports.py` bind the quoted figures
to it.

---

## 1. What this is

Phase B of the agreed plan: **P6, the preview pyramid**, and **P7, adaptive
tiling**, back to back, with **one** container version bump.

The version bump is **v0.1 → v0.2**. It does not move a single byte of the
layout:

* the main header is still 64 bytes with the same fields at the same
  offsets;
* the directory entry is still 40 bytes with the same fields at the same
  offsets;
* the file layout is still header, directory, payloads.

What v0.2 changes is two *rules*:

1. **The directory entry's `flags` field carries the level index.** The low
   byte is the level (0 is the original, 1 and up are derived previews); the
   upper byte is reserved and must be zero.
2. **A level's tiles may have different sizes.** A level's tiles are an
   *exact partition* of the level's rectangle — every pixel covered once —
   instead of one uniform grid.

Everything else is the same container, the same codecs and the same lossless
promise. The default encoder path still writes **v0.1** bytes: the golden
fixtures do not move, and §3.1 shows that the v0.1 file this milestone
produced is **the same file, byte for byte**, that the P5.2 baseline
produced.

---

## 2. How it was measured

### 2.1 The host

`stable-x86_64-pc-windows-gnu`, rustc 1.100.0, an Intel Core i5-1235U with
**10 physical cores and 12 logical processors**, 7.7 GB of physical memory.
The benchmarks are built with the release profile.

The host is a **mobile U-series part that was also running the rest of the
toolchain**, not an idle dedicated server. Every figure below is a
measurement *on this host*.

### 2.2 The photographs

**P6** uses the corpus's own 15,360 × 8,640 RGB photograph —
**398,131,200** canonical bytes, the same file the P5.2 latency matrix used,
so the two reports can be read side by side.

**P7** uses the **whole 750-image corpus** — photographs 300, screenshots
100, pixel art 100, scans 100, textures 150 — with the **same denominator**
the P5 corpus report used: the canonical pixel bytes of the same image.

### 2.3 The rules

* **Every case is repeated.** P6 runs each case **five** times; the floor
  the plan set is five, and the p95 over five samples is therefore the
  largest of the five, which the report says rather than hides.
* **The median.** The middle sample, or the mean of the two middle samples
  when the count is even.
* **The p95.** The **nearest-rank** percentile: the smallest sample at or
  above the 95th percentile. With five samples it is the largest of the five.
* **The standard deviation.** The sample one, with n − 1 in the denominator.
* **The bytes are checked.** Every encode records the SHA-256 of the file it
  wrote, and P7 decodes every adaptive file and compares it with the
  original pixels.

### 2.4 What is in the memory figures

**Included:** the whole original picture and every derived level held at
once, the tile payloads in flight, the encoded payloads waiting to be
written, the container metadata, the report, the binary, the stacks and the
allocator's own bookkeeping.

**Excluded:** the canonical pixel file on disk, the operating system's page
cache, the memory of other processes, and the benchmark driver's own memory.

The pyramid encoder **has** to hold the whole picture: a preview is computed
from the pixels below it, and the adaptive planner has to look at the picture
to choose the tile sizes. That is a real cost and §3.2 states it in bytes.
The **default** path — a uniform grid, no pyramid — never materialises the
picture, and keeps the bounded-memory streaming property that P5.1 measured.

---

## 3. Results

### 3.1 P6 — the preview pyramid

| Case | Median | p95 | Mean | Sample stdev | File | Tiles |
|---|---:|---:|---:|---:|---:|---:|
| v0.1, no pyramid | **18590.46 ms** | 20911.24 ms | 18545.80 ms | 1635.36 ms | **124,197,035 B** | 2,040 |
| v0.2, 3 derived levels | **26809.86 ms** | 31703.01 ms | 27909.44 ms | 2170.57 ms | **164,873,298 B** | 2,725 |

The pyramid file's SHA-256 is
`fc384c8219b52e2f816fa5e383c48bfc8ffdff41d8a5a654eb14035b648b3dbd`, and the
v0.1 file's is
`d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf`.

**That v0.1 digest is the P5.2 baseline's tile-256 digest.** The same file,
byte for byte, as the measurement this milestone is compared against — so
the format revision moved the default path by **nothing**.

The pyramid holds four levels:

| Level | Size | Tiles | Payload bytes |
|---|---|---:|---:|
| 0 — the original | 15,360 × 8,640 | 2,040 | 124,115,367 |
| 1 — derived | 7,680 × 4,320 | 510 | 31,300,377 |
| 2 — derived | 3,840 × 2,160 | 135 | 7,470,762 |
| 3 — derived | 1,920 × 1,080 | 40 | 1,877,724 |

**What it costs.** The previews add **40,648,863** bytes, so the pyramid
file is **32.7%** bigger than the same picture without it (164,873,298
against 124,197,035), and the encode takes **44.2%** longer
(18590.46 ms → 26809.86 ms). The ratio the encoder reports is computed
against the **original** pixels, so a pyramid can never flatter the number.

**What it buys.** Decoding a whole level, five runs each:

| Level | Median | p95 | Pixels |
|---|---:|---:|---:|
| 0 — the original | 12962.95 ms | 14014.20 ms | 398,131,200 |
| 1 | 3371.25 ms | 3396.86 ms | 99,532,800 |
| 2 | 813.89 ms | 938.06 ms | 24,883,200 |
| 3 | 197.94 ms | 203.31 ms | 6,220,800 |

The smallest preview — the whole picture at 1/64 of the pixels — is
**197.94 ms** (p95 203.31 ms) against **13062.32 ms** (p95 13290.72 ms) for
the whole picture at full resolution: **66× less time to something the
viewer can put on screen**. It is a smaller picture, and the report says so:
the preview is 1,920 × 1,080, not 15,360 × 8,640.

**What it does to the full-resolution path.** The first viewport — open the
file, decode the 20 tiles of a 1024 × 1024 window in the middle — costs
**31.61 ms** (p95 35.92 ms) from the v0.1 file and **34.41 ms** (p95
37.31 ms) from the pyramid's level 0. The pyramid's directory holds 685 more
entries, and the median difference is **+8.8%**. It is small, it is real,
and it is measured.

### 3.2 P6 — the memory of a pyramid encode

The benchmark process peaked at **547,844,096 bytes** of resident set and
**541,421,345 bytes** of heap.

The picture itself is 398,131,200 bytes and the three previews add
130,636,800, so the materialised pixels are 528,768,000 bytes — and the
measured heap peak is about 12.7 MB above that, which is the tile buffers,
the payloads and the allocator's own overhead. The number is the honest
consequence of the design: **a pyramid encoder holds the picture**.

**This is not the 64 MB editing target.** That target is about a *localized
edit* of a 16K image, and it is measured separately in §3.4.

### 3.3 P7 — adaptive tiling over the 750-image corpus

Every image was encoded twice: once with the uniform grid, once with the
adaptive partition, both at tile size 256 with the `auto` codec and **the
same denominator** — the canonical pixel bytes of the same image.

| | Uniform | Adaptive |
|---|---:|---:|
| Total bytes over the corpus | 453,767,460 | **452,762,594** |
| Median ratio (bytes ÷ canonical) | 0.415337 | **0.413257** |
| p95 ratio | 0.801071 | 0.801958 |
| Median tile count | 4 | **2** |
| Under the 10 MB ceiling | 748/750 | 748/750 |

**Adaptive tiling is a wash, and the report keeps the wash.** Over the whole
corpus it saves **1,004,866 bytes — 0.22%**. The median image saves nothing
at all; the mean saving is 1,339.8 bytes.

| Outcome | Images |
|---|---:|
| Smaller | **330** |
| Larger | **241** |
| Identical | 179 |

Per category:

| Category | Images | Uniform | Adaptive | Median delta | Smaller | Larger | Equal |
|---|---:|---:|---:|---:|---:|---:|---:|
| photos | 300 | 0.602 | 0.601 | −271 B | 179 | 105 | 16 |
| scans | 100 | 0.346 | 0.346 | −346.5 B | 68 | 32 | 0 |
| textures | 150 | 0.378 | 0.377 | 0 B | 49 | 31 | 70 |
| pixel art | 100 | 0.165 | 0.165 | 0 B | 4 | 4 | 92 |
| **ui** | 100 | **0.063** | **0.064** | **+826.5 B** | 30 | 69 | 1 |

**Adaptive tiling helps photographs and scans, and it *hurts* UI
screenshots** — the one category where a uniform grid is already nearly
optimal, because a screenshot is mostly flat rectangles that the palette and
run-length codecs handle well at any tile size, while the planner's larger
tiles push some tiles past the palette codec's 256-colour limit.

The biggest single win is the 16K photograph itself: **124,197,035 → 123,202,141
bytes (−994,894 B, −0.80%)**, with the tile count falling from **2,040** to
**608**. The biggest single loss is `threejs-textures-a9aefa111bdb`, at
**+127,952 bytes**.

The adaptive partition is not a compression win in general. What it *is* is a
partition with **half the tiles** at the median (4 → 2), which is a smaller
directory and fewer requests to serve a region.

### 3.4 The editing path, after the refactor

The editing engine moved from the grid to the partition model, so the 64 MB
localized-edit target was **re-measured**, not assumed. The matrix is the
P4 one — 4096², 8192² and 16384² × {gray8, rgb8, rgba8, rgba16}, one case
per process, tile size 512 — and the result is **12 PASS / 0 FAIL / 0
BLOCKED**.

| Resolution | Format | File bytes | Write heap peak | Write working-set peak | Edit heap peak | Edit working-set growth | Tiles rewritten / copied |
|---|---|---:|---:|---:|---:|---:|---:|
| 4096² | RGBA8 | 9,318,919 | 6,230,180 | 10,358,784 | 8,048,772 | 8,278,016 | 1 / 63 |
| 4096² | Gray8 | 2,545,200 | 1,768,069 | 5,111,808 | 2,219,035 | 2,301,952 | 1 / 63 |
| 4096² | RGB8 | 7,093,080 | 4,810,172 | 8,155,136 | 6,040,378 | 6,426,624 | 1 / 63 |
| 4096² | RGBA16 | 10,537,385 | 9,174,012 | 13,213,696 | 13,083,643 | 13,729,792 | 1 / 63 |
| 8192² | RGBA8 | 37,664,751 | 6,260,455 | 10,166,272 | 8,060,736 | 7,991,296 | 1 / 255 |
| 8192² | Gray8 | 10,325,664 | 1,782,540 | 5,312,512 | 2,230,169 | 2,781,184 | 1 / 255 |
| 8192² | RGB8 | 28,678,686 | 4,835,267 | 8,261,632 | 6,052,408 | 6,389,760 | 1 / 255 |
| 8192² | RGBA16 | 42,505,880 | 9,203,721 | 13,266,944 | 13,096,094 | 13,762,560 | 1 / 255 |
| 16384² | RGBA8 | 151,673,619 | 6,314,573 | 10,665,984 | 8,107,397 | 8,712,192 | 1 / 1,023 |
| 16384² | Gray8 | 41,722,203 | 1,820,628 | 5,996,544 | 2,276,302 | 3,067,904 | 1 / 1,023 |
| 16384² | RGB8 | 115,535,824 | 4,884,215 | 8,347,648 | 6,098,653 | 5,709,824 | 1 / 1,023 |
| **16384²** | **RGBA16** | 171,237,665 | **9,256,343** | **13,193,216** | **13,142,626** | **13,770,752** | 1 / 1,023 |

The worst case in the matrix is the 16384 × 16384 RGBA16 image — 2 GiB of
canonical pixels — and its edit costs **13,770,752 bytes** of resident-set
growth against the **67,108,864-byte** target: **20.5%** of the budget. Every
case rewrites **one** tile and copies the rest byte for byte, at every
resolution.

**What is in these figures.** *Included:* the decoded-tile cache, the undo
history, the dirty tiles, the region buffers, the tile payloads the writer
read, and the process's own code and stacks. *Excluded:* the operating
system's page cache, the file on disk, the memory other processes use, and
the temporary file's space. The writer never holds the whole image: it
generates one tile, encodes it, writes it and moves on.

The raw run is `benchmarks/p6_memory_results.json`.

---

## 4. The twelve questions

**Q1 — Was the 16K photographic case actually run, or blocked?**
**Run, not blocked.** The corpus's own 15,360 × 8,640 RGB photograph —
398,131,200 canonical bytes — was encoded ten times in the preview benchmark
(five with the pyramid, five without) and every level of the pyramid was
decoded five times, on this host, in one process. The adaptive case was run
over **all 750 corpus images**, each encoded twice and decoded once. The raw
files are `benchmarks/p6_preview_results.json` and
`benchmarks/p7_adaptive_results.json`.

**Q2 — How many times was each case run, and what do the percentiles mean?**
**Five times for P6, once per image for P7.** The median is the middle
sample, or the mean of the two middle samples when the count is even. The
p95 is the **nearest-rank** percentile: the smallest sample at or above the
95th percentile, so with five samples the p95 *is* the largest of the five.
The standard deviation is the sample one, with n − 1 in the denominator. The
rules are in the raw JSON and in the unit tests of `photo_bench.rs`.

**Q3 — What is the baseline, and was it measured in the same run?**
**The P5.2 baseline, and it is the same file.** The v0.1 file this benchmark
wrote has the SHA-256
`d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf`, which is
the P5.2 tile-256 digest, and the same 124,197,035 bytes. The first-viewport
and whole-picture latencies were measured **in the same process, on the same
files**, as the pyramid's.

**Q4 — Does the pyramid change the original?**
**No.** Level 0 is the original and it is exact: the decoder reproduces the
source pixels byte for byte, and the test suite asserts it on a mixed
picture, on a gradient and on a 16K-scale case. A derived level is never
used to reconstruct the original — it is marked as derived in the entry's
`flags` field, and the decoder's `decode_all` reads level 0 only.

**Q5 — What does the pyramid cost?**
**+32.7% bytes and +44.2% encode time** on the 16K photograph: 164,873,298
bytes against 124,197,035, and 26809.86 ms against 18590.46 ms. The previews
themselves are 40,648,863 bytes. The encoder's ratio uses the original
pixels as the numerator, so a pyramid always scores worse, never better.

**Q6 — What does the pyramid buy?**
**The whole picture, 66× sooner.** The smallest level decodes in **197.94 ms**
(p95 203.31 ms) against **13062.32 ms** for the full-resolution picture — at
1/64 of the pixels. Level 2 is 813.89 ms and level 1 is 3371.25 ms, so a
viewer can pick the level that matches the window it is filling.

**Q7 — Does the pyramid slow the full-resolution path down?**
**A little.** The first viewport costs **34.41 ms** from the pyramid's level
0 against **31.61 ms** from the v0.1 file — **+8.8%** at the median, with
p95s of 37.31 ms and 35.92 ms. The pyramid's directory holds 685 more
entries. This is below the run-to-run spread of the P5.2 matrix (18.5% to
52.4% of the median), but it is a cost, not a wash, and it is reported as
one.

**Q8 — What does adaptive tiling do to the file?**
**Almost nothing: −0.22% over the corpus.** 453,767,460 bytes become
452,762,594, and the median ratio goes from 0.415337 to 0.413257. 330 images
get smaller, 241 get larger and 179 are identical. The median image gains
nothing at all.

**Q9 — Where does adaptive tiling help, and where does it hurt?**
**Photographs and scans get smaller; UI screenshots get bigger.** The
photograph category's median ratio goes from 0.602 to 0.601 and 179 of its
300 images shrink; the UI category's goes from 0.063 to 0.064, with 69 of
its 100 images growing and a median increase of 826.5 bytes. Pixel art is
unchanged on 92 of its 100 images. The planner's larger tiles help a
photograph's flat regions and hurt a screenshot's flat rectangles, which the
palette codec already handles perfectly at any size.

**Q10 — Does adaptive tiling change the pixels, or break the container?**
**No: 750 of 750 exact, and 0 check failures.** Every adaptive file was
decoded and compared with the original pixels (`vpng verify`: 750 passes),
and every file passed the container's own integrity check (`vpng check`: 0
failures). The partition is validated as an exact cover before the file is
written, so a partition with a gap or an overlap cannot be written at all.

**Q11 — What is in the memory figures, and does this report make a memory
claim?**
**It makes one memory claim about the pyramid, and it is a negative one.**
The pyramid encode peaked at **547,844,096 bytes** of resident set and
**541,421,345 bytes** of heap, against the 528,768,000 bytes of pixels the
design requires it to hold: the difference is the tile buffers, the payloads
and the allocator's overhead. The scope is stated in §2.4 — the file on disk,
the page cache, other processes and the driver are **excluded**. The default
v0.1 path is unaffected: it never materialises the picture, and P5.1 measured
its bounded working set at 19,296,256 bytes. The **64 MB localized-edit
target** is a different claim and it was **re-measured after the partition
refactor**: 12 cases, **12 PASS / 0 FAIL / 0 BLOCKED**, the worst being the
16384 × 16384 RGBA16 image at **13,770,752 bytes** of resident-set growth
against the 67,108,864-byte budget — **20.5%** of it (§3.4).

**Q12 — What is not measured, and what remains untested?**
The limits are in §6. The four that matter most: the p95 over five samples
is the maximum, so it is a weak tail estimate; the pyramid's benefit is
measured on **one** photograph, and a screenshot's previews would be built
from flat rectangles; the adaptive case was measured with the **default**
threshold of 8 and the default tile size of 256, and no other setting was
swept; and the corpus run measured the **file size**, not the decode
latency of an adaptive partition, which is left to the next phase.

---

## 5. Reproducing the evidence

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# One: the pyramid. Five runs per case, on the corpus's own photograph.
cargo run --release -p vpng-validation --bin vpng-preview-bench -- `
    --source corpus-data/cache/7479807165b6411a-1006677.raw `
    --width 15360 --height 8640 --format rgb8 `
    --tile-size 256 --workers 8 --runs 5 --pyramid-levels 3 `
    --work-dir benchmarks/work/preview `
    --output benchmarks/p6_preview_results.json

# Two: the adaptive partition over the whole corpus, with the same
# denominator the corpus report used.
cargo build --release --workspace
python benchmarks/corpus/run_adaptive.py --jobs 2 `
    --output benchmarks/p7_adaptive_results.json

# Three: the editing memory matrix, re-run after the partition refactor.
python benchmarks/memory/run_memory.py `
    --output benchmarks/p6_memory_results.json

# Four: the tests, and the check that every quoted figure matches the
# raw data.
cargo test --workspace --all-targets
python benchmarks/verify_reports.py
```

---

## 6. What this does not claim

* **No claim that the pyramid makes a viewer fast.** It makes the *whole
  picture* available 66× sooner, at 1/64 of the resolution. Whether a
  viewer wants that is a product decision, and the measurement here is only
  the latency.
* **No claim that adaptive tiling compresses better.** It does not: the
  corpus says −0.22%, and on UI screenshots it is *worse*. The claim it
  supports is about the **partition**: half the tiles at the median.
* **No memory claim for the pyramid encoder beyond what is stated.** It
  holds the picture, and §3.2 says so with the scope attached.
* **No strong tail claim.** Five samples, so the nearest-rank p95 is the
  largest of the five.
* **No claim about other content for P6.** One photograph, at one size. The
  adaptive case *was* run over 750 images, so its figures do generalise
  across the corpus's five categories.
* **No claim about other platforms.** Windows on x86-64.
* **No claim that the format is frozen.** v0.2 is a revision, and the
  release plan still has the codec table, the metadata chunks and the v1.0
  freeze ahead of it.

---

## 7. The acceptance gates

| # | Gate | Result | Evidence |
|---|---|---|---|
| 1 | Every timing case is repeated at least five times | **PASS** | P6: five runs per case, every sample published in the raw JSON. P7 measures deterministic bytes rather than time — the corpus is the repetition: 750 images, each encoded both ways, and each result re-verified against the source pixels |
| 2 | The median and the p95 are reported, not a single figure | **PASS** | every row carries min, max, mean, median, p95 and the sample stdev |
| 3 | The percentile rules are defined and pinned | **PASS** | the rules are in the raw JSON and in the unit tests of `photo_bench.rs` |
| 4 | The baseline is the same file, measured in the same run | **PASS** | the v0.1 digest equals the P5.2 baseline's; the viewport and whole-picture latencies are measured in the same process |
| 5 | The default path's bytes do not move | **PASS** | `d029a7ebc1e0347cb…` — the P5.2 tile-256 digest, and the golden fixtures still pass unchanged |
| 6 | The original is exact, whatever the pyramid holds | **PASS** | `v2_formats.rs`: the pyramid file decodes to the original byte for byte, and a derived level never replaces it |
| 7 | A derived level is marked and strictly smaller | **PASS** | the entry's `flags` carries the level; the decoder rejects a level that is not strictly smaller than the one below it |
| 8 | The adaptive partition covers every pixel exactly once | **PASS** | the sweep-line validator rejects a gap or an overlap before a file is written; 750 of 750 corpus images decode exactly |
| 9 | The container's own check passes on every file | **PASS** | 0 failures across the 750 adaptive files |
| 10 | The 64 MB localized-edit target | **PASS** | the P4 memory matrix, re-run after the partition refactor: **12 PASS / 0 FAIL / 0 BLOCKED**, the worst case 13,770,752 B of resident-set growth against the 67,108,864 B budget — §3.4, `benchmarks/p6_memory_results.json` |
| 11 | Every quoted figure is bound to the raw data | **PASS** | `check_preview()` and `check_adaptive()` in `benchmarks/verify_reports.py` |
| 12 | A v0.1 file is still readable, and a newer minor version is still refused | **PASS** | `v2_formats.rs` and `header.rs`: v0.1 files decode unchanged (the golden fixtures do too), a v0.1 file that claims a pyramid is `UnsupportedFeature`, and the minor-version check — `minor > FORMAT_MINOR` — is the same check a v0.1-only build made, so a v0.1 reader refuses a v0.2 file |

---

## 8. What is next

The two features are in, measured and documented. What they leave open:

* **The pyramid's product value.** The measurement says the whole picture is
  available 66× sooner at 1/64 scale. Whether a viewer should use it is the
  next phase's decision, and it needs the *viewing* path — the viewport
  does not know about levels yet.
* **The adaptive threshold.** One setting (8) was measured on 750 images.
  A sweep over the threshold, and over the tile size, would say whether 8 is
  the right default or an arbitrary one.
* **The codecs.** The corpus still says VPNG is behind PNG on every
  category (0.415 against 0.379 before this milestone; 0.413 after it).
  The next release rows promote the stable codec ids 4–6, and that is where
  the gap is closed.
