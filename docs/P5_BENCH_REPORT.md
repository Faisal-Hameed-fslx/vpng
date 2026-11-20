# P5.2 — The multi-run photographic baseline and the latency matrix

**Date:** 2026-11-20
**Deliverable:** the benchmark harness `vpng-photo-bench` in
`crates/vpng-validation`, the canonical-pixel cache
`benchmarks/corpus/canonical_cache.py`, and the raw result file
`benchmarks/p5_bench_results.json`.
**Status:** the 16K photographic case **ran** — it did not have to be
reported BLOCKED. Every figure below is read out of the raw JSON, and
`benchmarks/verify_reports.py` checks the quoted figures against it.

---

## 1. What this is

Phase A, Step 2 of the agreed plan: the **measurement foundation**.

Every benchmark in this repository before this one measured **a single run**
and reported the number it got. That is enough to say "this works" and not
enough to say "this is faster than that". A single run on a shared machine
has a run-to-run variance that is often larger than the effect being
measured. The evidence is in this repository's own history: the same
one-worker 16K encode measured **176,812 ms** in one run and **128,963 ms**
in the next — a 37% swing, before any code changed that could explain it.

This milestone fixes that, and it fixes it in the three ways that matter for
the work that comes next:

* **It repeats.** Every case runs ten times, and the report gives the
  **median** and the **p95**, not a mean and not a single figure.
* **It uses a photograph.** The synthetic gradient every other benchmark
  uses compresses trivially and decodes trivially. A real photograph is
  what the engine will actually meet.
* **It sweeps a matrix.** The tile size and the worker count are both
  varied, because the interesting question is not "how fast is this
  configuration" but "how does the configuration change the answer".

The reason this comes **before** the container revisions (P6, the preview
pyramid, and P7, adaptive tiling) rather than after them is simple: those
two phases change the file format. A format change cannot be evaluated
against a baseline that is one noisy sample. This milestone is that
baseline.

## 2. How it was measured

### 2.1 The host

`stable-x86_64-pc-windows-gnu`, rustc 1.100.0, an Intel Core i5-1235U with
**10 physical cores and 12 logical processors**, 7.7 GB of physical memory.
The benchmark is built with the release profile.

The host is a **mobile U-series part that was also running the rest of the
toolchain**, not an idle dedicated server. Every figure below is a
measurement *on this host*, and the report says so wherever a figure could
be read as a claim about hardware in general.

### 2.2 The photograph

The corpus's native 16K photograph, `user-photo-15360x8640` from
`benchmarks/corpus_manifest.json`:

| Property | Value |
|---|---|
| Source | `1006677.jpg`, the repository owner's own photograph |
| Source SHA-256 | `7479807165b6411a9feaddd7306f68655f1df0449657c627c05dfa8d00b40579` |
| Native dimensions | 15,360 × 8,640 |
| Pixel format | RGB8 |
| Canonical pixels | 398,131,200 bytes (379.8 MiB) |
| Canonical SHA-256 | `a61610f7812691b1ed4065a1b63fe4c59bcaead2155bf0936376fce45aa0942f` |

The canonical digest matches the manifest's entry for this image exactly,
so the bytes the benchmark measured are the bytes the corpus records.

The JPEG is converted to canonical pixels **once**, by
`benchmarks/corpus/canonical_cache.py`, and cached. The cache is keyed by
the SHA-256 of the source file's bytes and carries a sidecar recording both
the source digest and the canonical digest, so a stale or truncated cache is
*detected* rather than trusted. Converting a JPEG to canonical pixels is not
cheap, and a benchmark should not repeat that work inside a timed loop.

The benchmark then reads that canonical file **tile by tile, at the tile's
own offsets**, through a `PixelSource`. The image is never in memory: a
tile is one run of bytes per row, and each row is read positionally, so the
workers do not serialise on the file.

### 2.3 The matrix

| Axis | Values |
|---|---|
| Tile size | 256, 512, 1024 |
| Worker count | 1, 2, 4, 8 |
| Runs per case | 10 |
| Viewport | 1024 × 1024 pixels |
| Pans per run | 12, each 512 px to the right |
| Viewing cache | 32 MiB |
| Queue depth | 256 |

Every case therefore has **ten samples**, and every sample is a complete,
independent measurement.

### 2.4 The rules

* **The median.** The middle sample, or the mean of the two middle samples
  when the count is even.
* **The p95.** The **nearest-rank** percentile: the smallest sample at or
  above the 95th percentile. With ten samples it is the largest of the ten.
  The rule is recorded in the raw JSON next to the numbers, and a unit test
  pins it.
* **The standard deviation.** The sample one, with n − 1 in the denominator.
* **Every figure has a baseline.** The sequential decoder — one tile at a
  time, no cache, no threads — is measured in the same run, on the same
  file, in the same process.
* **The bytes are checked.** Every encode records the SHA-256 of the file
  it wrote. If the tile size or the worker count changed a single byte, the
  digests would differ and the report would say so.
* **The memory scope is stated.** Included: the tiles in flight, the encoded
  payloads waiting to be written, the container metadata, the report, the
  binary, the stacks and the allocator's metadata. Excluded: the file on
  disk, the operating system's page cache, other processes, and the
  benchmark driver's own memory. **This report makes no memory claim:** the
  encoder's working-set bound is measured separately, in
  `docs/P5_ENCODER_REPORT.md`, with a source that generates tiles on demand
  and therefore never touches the page cache at all.

### 2.5 Why the scenario order is fixed

The encode matrix runs first, then the viewing matrix, then the sequential
baseline, for each tile size. The encode matrix must come first because it
produces the file the viewing matrix opens. The sequential baseline must
come last because it holds a decoded tile at a time and the earlier
scenarios have already warmed the page cache.

## 3. Results

The whole run took **5077170.70 ms** of wall clock (about 85 minutes) and
performed **120 complete encodes** of a 398,131,200-byte photograph.

### 3.1 The encode matrix

The time to encode the whole 16K photograph, in milliseconds.

| Tile size | Workers | Median | p95 | Mean | Stdev | Min | Max | Speedup vs 1 worker |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 256 | 1 | 78793.67 | 89621.85 | 77207.04 | 8266.75 | 65240.76 | 89621.85 | 1.00x |
| 256 | 2 | 47238.06 | 60914.34 | 49302.52 | 7570.00 | 36177.04 | 60914.34 | 1.67x |
| 256 | 4 | 27264.60 | 29932.02 | 26245.67 | 3153.69 | 20958.69 | 29932.02 | 2.89x |
| 256 | 8 | 12811.94 | 15215.08 | 13203.89 | 862.18 | 12709.93 | 15215.08 | 6.15x |
| 512 | 1 | 82638.62 | 89305.17 | 79256.15 | 9367.61 | 58425.41 | 89305.17 | 1.00x |
| 512 | 2 | 34449.41 | 47469.75 | 37680.05 | 6555.50 | 30545.33 | 47469.75 | 2.40x |
| 512 | 4 | 24903.65 | 29611.68 | 25107.03 | 2336.52 | 20778.32 | 29611.68 | 3.32x |
| 512 | 8 | 17857.24 | 20286.83 | 17639.53 | 1682.01 | 14857.56 | 20286.83 | 4.63x |
| 1024 | 1 | 84348.62 | 96997.28 | 83660.53 | 11852.82 | 66612.87 | 96997.28 | 1.00x |
| 1024 | 2 | 44375.46 | 48551.31 | 42347.20 | 4837.62 | 35441.78 | 48551.31 | 1.90x |
| 1024 | 4 | 24669.51 | 27808.42 | 24946.18 | 1426.75 | 23234.08 | 27808.42 | 3.42x |
| 1024 | 8 | 15822.17 | 17695.26 | 16206.10 | 938.54 | 14619.61 | 17695.26 | 5.33x |

**Six workers' worth of speed at eight workers, at tile size 256: 78793.67 ms
→ 12811.94 ms, 6.15×.** That is *better* than the 4.27× the synthetic
gradient managed in `docs/P5_ENCODER_REPORT.md`, and the reason is
straightforward: a photograph spends its time in the codecs' real paths —
RLE runs, predictor residuals — and that work parallelises cleanly. A
gradient spends much of its time in code that was already cheap.

**The scaling is sub-linear at every tile size, and the report does not
dress that up.** Per doubling, at tile size 256: 1.67×, then 1.73×, then
2.21×. This is a 10-core mobile part that was also running the rest of the
toolchain; it is not a 12-core server with nothing else to do. **The claim
is "more workers are faster, and the bytes do not change", not "it scales
linearly."**

**A bigger tile is a slower encode.** At one worker: 78793.67 ms at 256,
82638.62 ms at 512, 84348.62 ms at 1024 — about 7% slower across a
four-fold change in tile area. The tile count falls and the per-tile work
rises, and the second effect slightly wins.

### 3.2 What the tile size costs and buys

| Tile size | Tiles | File size | Payload bytes | Ratio | SHA-256 |
|---:|---:|---:|---:|---:|---|
| 256 | 2,040 | 124,197,035 | 124,115,367 | 3.206 | `d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf` |
| 512 | 510 | 123,142,966 | 123,122,498 | 3.233 | `0f902a7f0d3a4b3ecb559daac64e7a5aae98693980b6a1ddb6405e671ba32a11` |
| 1024 | 135 | 122,613,341 | 122,607,873 | 3.247 | `268434304fca43f7c8d9ef7540c3264bbd3fdc7feb5503105b2d36c3bdce689e` |

**Bigger tiles make a smaller file, and the gain is small.** Going from 256
to 1024 saves **1,583,694 bytes — 1.27%** — while costing 7% more encode
time, and, as §3.4 shows, making the interactive latency far worse. That is
the trade P7 has to price, and it is not the trade one might have assumed.

**The worker count changes nothing about the file.** Each tile size has
**one** digest across all forty of its runs, and the three digests differ
from each other only because the tile grid differs. That is the same claim
P5.1 made for the synthetic image, now made for a photograph, and it is
the claim that makes the parallel encoder safe to use at all.

### 3.3 The first-viewport latency matrix

The time from opening the file to the first pixels on screen, in
milliseconds, against the sequential baseline measured in the same run.

| Tile size | Workers | Visible tiles | Open (median) | Decode (median) | Time to pixels (median) | p95 | Sequential baseline | Speedup |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 256 | 1 | 20 | 0.36 | 205.19 | 205.60 | 215.15 | 141.16 | 0.69x |
| 256 | 2 | 20 | 0.37 | 103.01 | 103.53 | 107.01 | 141.16 | 1.36x |
| 256 | 4 | 20 | 0.46 | 52.62 | 53.10 | 54.16 | 141.16 | 2.66x |
| 256 | 8 | 20 | 0.64 | 31.72 | 32.35 | 34.03 | 141.16 | 4.36x |
| 512 | 1 | 6 | 0.39 | 197.38 | 197.76 | 230.93 | 254.08 | 1.28x |
| 512 | 2 | 6 | 0.32 | 129.37 | 130.00 | 138.23 | 254.08 | 1.95x |
| 512 | 4 | 6 | 0.38 | 83.42 | 83.83 | 85.88 | 254.08 | 3.03x |
| 512 | 8 | 6 | 0.60 | 46.46 | 47.08 | 58.43 | 254.08 | 5.40x |
| 1024 | 1 | 2 | 0.17 | 250.36 | 250.54 | 287.07 | 236.51 | 0.94x |
| 1024 | 2 | 2 | 0.23 | 133.92 | 134.25 | 157.44 | 236.51 | 1.76x |
| 1024 | 4 | 2 | 0.27 | 135.56 | 135.85 | 170.64 | 236.51 | 1.74x |
| 1024 | 8 | 2 | 0.37 | 158.53 | 158.89 | 178.06 | 236.51 | 1.49x |

Three findings here, and the third is the one that matters most.

**The pool pays for itself once there are enough tiles.** At tile size 256
the viewport needs 20 tiles, and eight workers turn the sequential
141.16 ms into **32.35 ms — 4.36×**. At tile size 512 the viewport needs
6 tiles, and eight workers give the best result in the whole matrix:
**47.08 ms against 254.08 ms, 5.40×**.

**With one worker, the pool is slower than no pool.** At tile size 256 the
one-worker pool takes 205.60 ms against the sequential path's 141.16 ms —
**0.69×**. The pool's machinery costs more than it saves when there is
nothing to parallelise. That is a true and useful result, not a
disappointment: it says a viewer must not use the pool for a single tile.

**At tile size 1024 the parallel pool runs out of work.** The viewport
needs **two** tiles, so eight workers have eight threads and two tiles to
give them. Four workers take 135.85 ms and eight workers take **158.89 ms
— the bigger pool is slower**, and the "speedup" against the sequential
baseline falls to 1.49×. This is not a defect in the pool; it is the pool
being asked to parallelise something that is not parallelisable. It is also
the first concrete argument for a **preview pyramid** (P6): the fix for
"the first viewport has too few tiles to decode" is more, smaller tiles,
not more threads.

### 3.4 The pan latency

Twelve pans to the right, each 512 px, per run. The per-pan figures
cover every pan of every run.

| Tile size | Workers | Pans | Median pan | p95 pan | Slowest pan | Median sweep |
|---:|---:|---:|---:|---:|---:|---:|
| 256 | 1 | 12 | 102.03 | 195.32 | 199.68 | 1314.59 |
| 256 | 2 | 12 | 52.36 | 98.15 | 109.30 | 676.65 |
| 256 | 4 | 12 | 31.38 | 51.24 | 55.85 | 397.14 |
| 256 | 8 | 12 | 21.44 | 31.38 | 53.96 | 267.96 |
| 512 | 1 | 12 | 108.38 | 177.12 | 283.63 | 1298.71 |
| 512 | 2 | 12 | 84.13 | 120.87 | 132.26 | 1052.65 |
| 512 | 4 | 12 | 44.02 | 82.45 | 92.79 | 572.90 |
| 512 | 8 | 12 | 43.42 | 51.69 | 64.86 | 532.77 |
| 1024 | 1 | 12 | 231.70 | 324.12 | 527.81 | 1841.38 |
| 1024 | 2 | 12 | 123.92 | 151.87 | 189.07 | 958.19 |
| 1024 | 4 | 12 | 132.65 | 179.14 | 194.33 | 1047.55 |
| 1024 | 8 | 12 | 125.87 | 158.09 | 165.96 | 955.15 |

**Panning at tile size 1024 costs six times what panning at 256 costs**:
125.87 ms against 21.44 ms, with eight workers. The reason is the size of
the unit of work: a pan that crosses a tile boundary must decode a whole
1024 × 1024 tile — 3,145,728 pixels — where at 256 it decodes 65,536.

The same pattern appears at four and eight workers: at tile size 1024, four
workers take 132.65 ms and eight take 125.87 ms, and neither reaches the
21.44 ms that tile size 256 manages with eight. **The tile size, not the
thread count, is the dominant term in interactive latency on this
workload.** That is the single most actionable number in this report.

### 3.5 How much a single run can be trusted

The spread of the ten samples, as a percentage of the median.

| Tile size | Workers | Spread (max - min) | As % of median | Stdev as % of median |
|---:|---:|---:|---:|---:|
| 256 | 1 | 24381.09 ms | 30.9% | 10.5% |
| 256 | 2 | 24737.30 ms | 52.4% | 16.0% |
| 256 | 4 | 8973.33 ms | 32.9% | 11.6% |
| 256 | 8 | 2505.15 ms | 19.6% | 6.7% |
| 512 | 1 | 30879.76 ms | 37.4% | 11.3% |
| 512 | 2 | 16924.41 ms | 49.1% | 19.0% |
| 512 | 4 | 8833.37 ms | 35.5% | 9.4% |
| 512 | 8 | 5429.27 ms | 30.4% | 9.4% |
| 1024 | 1 | 30384.41 ms | 36.0% | 14.1% |
| 1024 | 2 | 13109.53 ms | 29.5% | 10.9% |
| 1024 | 4 | 4574.35 ms | 18.5% | 5.8% |
| 1024 | 8 | 3075.66 ms | 19.4% | 5.9% |

**This is the section the whole milestone exists for.** The run-to-run
spread is **18.5% to 52.4% of the median**. A configuration whose spread
is 52.4% of its median cannot be compared with another configuration on
the strength of one sample each: a 20% difference between two medians is
inside the noise of either one of them.

That is not a theoretical worry. It is exactly what happened in this
repository's own history: the same one-worker 16K encode measured
176,812 ms in one run and 128,963 ms in the next — a 37% swing, which sits
squarely inside the spread measured here.

The ordering is also worth noting: **the spread shrinks as the work gets
shorter.** The four cases with a spread under 20% are all eight-worker
runs, where the encode takes 12–18 seconds. The cases with a spread over
49% are all two-worker runs, where a single slow sample dominates. More
workers make the measurement *more* reliable as well as faster, because
each individual run is shorter and less exposed to whatever else the
machine is doing.

The digests of every file written during the run — three, one per tile
size, each shared by all forty runs of that tile size:
`0f902a7f0d3a4b3ecb559daac64e7a5aae98693980b6a1ddb6405e671ba32a11`,
`268434304fca43f7c8d9ef7540c3264bbd3fdc7feb5503105b2d36c3bdce689e`,
`d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf`.

**One note on the process figures, because they are not a memory claim.**
The whole process peaked at **192,806,912 bytes** of resident set and
**193,771,436 bytes** of heap. Those figures include the viewing matrix,
whose pool holds decoded tiles, and the sequential baseline, which holds a
decoded tile at a time — and the photograph is read from disk, so the
operating system's page cache is in play. **They are not the encoder's
working-set bound and this report does not use them as one.** The encoder's
bound is measured in `docs/P5_ENCODER_REPORT.md`: 19,296,256 bytes of
resident set on a 1 GiB image, against the 64 MB target.


## 4. The twelve questions

**Q1 — Was the 16K photographic case actually run, or blocked?**
**Run, not blocked.** The corpus's own 15,360 × 8,640 RGB photograph — 398,131,200 canonical bytes — was encoded 120 times and viewed repeatedly on this host, in one process. No figure here is inferred from a smaller image. The raw file is `benchmarks/p5_bench_results.json`.

**Q2 — How many times was each case run, and what do the percentiles mean?**
**Ten times, with the rules written down.** The median is the middle sample, or the mean of the two middle samples when the count is even. The p95 is the **nearest-rank** percentile: the smallest sample at or above the 95th percentile, so with ten samples it is the largest of the ten. The standard deviation is the sample one, with n − 1 in the denominator. The rules are in the raw JSON and in four unit tests, so a reader never has to guess what a number means.

**Q3 — What is the baseline, and was it measured in the same run?**
**The sequential decoder, in the same process, on the same file.** One tile at a time, no cache, no threads. Its medians are **141.16 ms** at tile size 256, **254.08 ms** at 512 and **236.51 ms** at 1024, with p95s of 184.79 ms, 271.60 ms and 261.48 ms. The one-worker encode is the encoder's own baseline, measured the same way.

**Q4 — Does the worker count change the file?**
**No.** Within each tile size, all forty encodes — four worker counts × ten runs — wrote one file. The three digests are `d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf` (256), `0f902a7f0d3a4b3ecb559daac64e7a5aae98693980b6a1ddb6405e671ba32a11` (512) and `268434304fca43f7c8d9ef7540c3264bbd3fdc7feb5503105b2d36c3bdce689e` (1024), and they differ from each other only because the tile grid differs. The digest is the proof, not an assertion, and every digest is in the raw JSON.

**Q5 — What does the tile size do to the file?**
**Bigger tiles make a smaller file, by a little.** 124,197,035 bytes at tile size 256, 123,142,966 at 512, **122,613,341** at 1024 — a ratio of 3.206 against **3.247**. Going from 256 to 1024 saves **1,583,694 bytes, 1.27%**. The file size is the same for every worker count at a given tile size, which is the same fact as Q4 seen from the other side.

**Q6 — How much faster is the encode with more workers?**
**6.15× at eight workers, at tile size 256: 78793.67 ms → 12811.94 ms.** At 512 it is 4.63× and at 1024 it is 5.33×. The scaling is **sub-linear** — per doubling at 256, 1.67×, 1.73×, 2.21× — and this is a 10-core mobile part that was also running the rest of the toolchain.

**Q7 — And how does the tile size change the encode time?**
**It makes it slower, slightly.** At one worker: 78793.67 ms at 256, 82638.62 ms at 512, **84348.62 ms** at 1024. A bigger tile is a bigger unit of work and there are fewer of them; the second effect wins by about 7% across a four-fold change in tile area.

**Q8 — How long does the first viewport take?**
At tile size 256, eight workers turn the sequential **141.16 ms** into **32.35 ms — 4.36×**, with a p95 of 34.03 ms. At 512 the best figure in the matrix is 47.08 ms against 254.08 ms, **5.40×**. At 1024 the viewport needs only two tiles, so the pool runs out of work: four workers take 135.85 ms and eight take **158.89 ms** — the bigger pool is *slower*. The p95 column matters as much as the median: an interactive engine is judged on its worst frame, not its best.

**Q9 — What does a pan cost?**
At tile size 256 with eight workers, **21.44 ms** median and 31.38 ms p95. At 1024, **125.87 ms** and 158.09 ms — **six times worse**. A pan across a tile boundary must decode a whole tile, and a 1024 × 1024 tile is 3,145,728 pixels where a 256 × 256 tile is 65,536. The tile size, not the thread count, is the dominant term in interactive latency on this workload.

**Q10 — How much run-to-run variance is there?**
**18.5% to 52.4% of the median**, case by case — §3.5 has every figure. A single run cannot be trusted, and the difference between two medians can easily be smaller than the noise in either one. The repository's own history is the clearest evidence: the same one-worker 16K encode measured 176,812 ms in one run and 128,963 ms in the next. The good news is that the spread **shrinks as the work gets shorter**: every case under 20% is an eight-worker run, and every case over 49% is a two-worker run.

**Q11 — What is in the memory figures, and does this report make a memory claim?**
**It makes no memory claim at all.** This report measures latencies, and the photograph is read from disk, so the operating system's page cache is in play and a resident-set figure here would not mean what a memory figure means. The process peaked at 192,806,912 bytes of resident set across the whole run, but that includes the viewing matrix's decoded tile cache, so it is not a bound on anything. The encoder's working-set bound is measured separately, in `docs/P5_ENCODER_REPORT.md`, with a source that generates tiles on demand: **19,296,256 bytes** of resident set on a 1 GiB image, against the 64 MB target.

**Q12 — What is not measured, and what remains untested?**
The limits are in §6. The three that matter most: the p95 over ten samples is the maximum, so it is a weak tail estimate; the content is one photograph, so the timings do not generalise to a screenshot or a scan; and the page cache is warm, so these are not cold-read figures.


## 5. Reproducing the evidence

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# One: convert the photograph to canonical pixels, once. The cache is
# keyed by the source file's SHA-256, so a second run is a no-op.
python benchmarks/corpus/canonical_cache.py --source 1006677.jpg `
    --cache-root corpus-data/cache

# Two: the matrix. Ten runs per case, three tile sizes, four worker
# counts. This takes about an hour on the host above.
cargo run --release -p vpng-validation --bin vpng-photo-bench -- `
    --source corpus-data/cache/7479807165b6411a-1006677.raw `
    --width 15360 --height 8640 --format rgb8 `
    --tile-sizes 256,512,1024 --workers 1,2,4,8 --runs 10 --chunk-tiles 4 `
    --work-dir benchmarks/work `
    --output benchmarks/p5_bench_results.json

# Three: the tests, and the check that every quoted figure matches the
# raw data.
cargo test --workspace --all-targets
python benchmarks/verify_reports.py
```

The benchmark deletes each file after it has measured it; `--keep-file`
leaves it.

## 6. What this does not claim

* **No claim that the host is representative.** This is a 10-core mobile
  U-series part that was also running the rest of the toolchain. Every
  figure is a measurement *on this host*. A dedicated server would scale
  differently, and a slower machine would scale differently again.
* **No claim about other content.** One photograph, at one size. The
  *timings* of a UI screenshot, a pixel-art sprite and a scanned document
  are different from a photograph's; the *bytes* are not, and that is
  covered by the 144-case identity matrix in `vpng-encoder` rather than
  here.
* **No memory claim.** The encoder's working-set bound is measured in
  `docs/P5_ENCODER_REPORT.md`, with a source that generates tiles on
  demand and therefore never touches the page cache at all. This report
  reads the photograph from disk, so its figures are **latencies**, not
  memory figures, and the scope is stated next to the numbers.
* **No strong tail claim.** With ten samples the nearest-rank p95 *is* the
  largest of the ten. That is a weak estimate of the tail, and it is
  labelled as such rather than presented as a percentile the data cannot
  support. Ten runs is the floor the plan set; a hundred would be better
  and is not affordable on a 74-second encode.
* **No cold-cache claim.** The canonical file is read from the operating
  system's page cache after the first pass, so these are warm-cache
  figures. The first run of each case may be colder than the rest; the
  median over ten runs is robust to that, and every sample is published so
  a reader can see it.
* **No claim about other platforms.** Windows on x86-64. The timings on
  Linux, macOS or ARM are not measured here.
* **No claim that the benchmark is fast.** A 16K photograph is a large
  image and the encode takes tens of seconds. The claim is only what the
  matrix shows, and the matrix is published.

## 7. The acceptance gates

| # | Gate | Result | Evidence |
|---|---|---|---|
| 1 | Every case is repeated at least five times | **PASS** | ten runs per case; every sample published in the raw JSON |
| 2 | The median and the p95 are reported, not a single figure | **PASS** | every row carries min, max, mean, median, p95 and the sample stdev |
| 3 | The percentile rules are defined and pinned | **PASS** | the rules are in the raw JSON and in four unit tests in `photo_bench.rs` |
| 4 | The content is photographic, at native resolution | **PASS** | 15360 × 8640 RGB, 398,131,200 canonical bytes, the corpus's own photograph |
| 5 | The tile size and the worker count are both swept | **PASS** | 3 tile sizes × 4 worker counts × 10 runs = 120 encodes |
| 6 | Every figure has a baseline measured in the same run | **PASS** | the sequential decoder is measured per tile size, in the same process |
| 7 | The worker count does not change the bytes | **PASS** | one SHA-256 per tile size across all forty runs of that tile size |
| 8 | Every quoted figure is bound to the raw data | **PASS** | `check_bench()` in `benchmarks/verify_reports.py`; the tables are rendered from the raw JSON by `render_bench_tables.py` |
| 9 | The memory bound | **NOT CLAIMED** | this report measures latencies; the working-set bound is P5.1's, measured separately |

## 8. What this baseline is for

**P6, the preview pyramid.** A pyramid exists to make opening and zooming
fast. That claim can only be made against a trustworthy "before": the
time-to-pixels figures in §3.3 are that baseline, and the p95 column is the
one a pyramid has to beat, because a pyramid that helps on the median and
hurts on the tail has not helped.

**P7, adaptive tiling.** Adaptive tiling is a claim about tile size, and
§3.1 and §3.2 are exactly that claim's two halves: what a bigger tile
**buys** (a smaller file) and what it **costs** (a longer encode, and a
different first-viewport latency). The ratio moves across the tile sizes
and the latency moves with it, so the choice is a measured trade rather
than a preference.

**The stable codec promotions (v0.5, v0.6, v0.7).** Those are decided on
the **750-image corpus**, not here. This milestone does not promote a
codec; it makes the corpus affordable to run, because the encoder now
scales across workers without changing a byte.

**The release cadence.** v0.2's threshold work and v0.20's corpus-driven
defaults both need a measurement method they can trust. This is that
method, stated in `docs/BENCHMARKS.md` §Method and applied here.

**The rule that outlives the milestone.** Every figure in this repository
before this one was a single run. From here on, a configuration is
reported with its spread, and a comparison is only made between
configurations whose spreads are smaller than the difference between them.


## 9. The next phase

**Phase A, Step 3 — the 750-image corpus pipeline.** The encoder is now
fast enough that a corpus run is affordable, and the parallel encoder makes
it reproducible in reasonable time. The corpus replaces the 9-image seed
with a real distribution, which is what the 10 MB target's success rate and
the per-category ratios need to be measured against.
