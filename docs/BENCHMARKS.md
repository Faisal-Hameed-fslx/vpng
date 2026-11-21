# Benchmarks

Every number in a report must name its hardware and corpus. No performance claim
is valid until it is measured here, with the raw run preserved.

## Method

1. `benchmarks/corpus_manifest.json` records every image, its provenance,
   license, dimensions, category, and SHA-256, so a run is reproducible.
2. Runs write machine-readable JSON into `benchmarks/results/` (git-ignored);
   durable summaries go into this document and `docs/ROADMAP_STATUS.md`.
3. Each record captures: CPU model, OS, RAM, Rust version, build profile and
   flags, worker count, input dimensions and format, tile size, codec choices,
   input bytes, output bytes, encode time, decode time, region-decode time,
   edit/commit time, peak RSS, internal allocation peak, and verification result.
   The 2026-11-14 runs capture logical CPUs, RAM, build profile, dimensions,
   format, tile size, codec choices, byte counts, wall time, sampled peak RSS,
   and exactness; CPU model, OS build string, and internal allocation peaks are
   still missing.

### Repetition, from 2026-11-20

A single run is a sample, not a measurement. The P5.2 milestone (below) is the
first in this repository to meet these rules, and every later run is expected
to follow them:

1. **Every case is repeated** — ten runs by default, five at the minimum.
   Every sample is reported, not just the summary.
2. **The median and the p95 are reported**, not a mean and not a single
   figure. The median is the middle sample, or the mean of the two middle
   samples when the count is even. The p95 is the **nearest-rank**
   percentile: the smallest sample at or above the 95th percentile, so with
   ten samples it is the largest of the ten. The rule is written into the
   raw JSON beside the numbers, so a reader never has to guess.
3. **The sample standard deviation** is reported beside them, with n − 1 in
   the denominator.
4. **Every figure has a baseline** measured in the same run, on the same
   file, in the same process.
5. **The spread is stated as a percentage of the median.** A configuration
   whose run-to-run spread is larger than the difference between two
   configurations cannot be compared, and the report says so rather than
   letting the reader draw the conclusion.
6. **The bytes are checked.** Every encode records the SHA-256 of the file
   it wrote; a configuration that changed the bytes is reported as a defect,
   not as a faster result.

The reason the rule exists is in this repository's own history: the same
one-worker 16K encode measured 176,812 ms in one run and 128,963 ms in the
next, a 37% swing. A single run could not tell those two apart.

## Baselines

Compare exact pixel data against PNG, WebP lossless, JPEG XL lossless, FLIF where
a reliable implementation exists, and raw uncompressed bytes. Record encoder
versions, settings, and color interpretation. Never compare files with different
metadata or pixel semantics without explaining the normalization.

## Metrics

Exact pixel equality, canonical SHA-256 equality, file bytes, compression ratio,
bits per pixel, encode/decode throughput, random tile latency, region-decode
latency, edit latency, peak memory, corruption detection, and corpus success
rate under 10 MB.

## Required experiments (roadmap §17)

* **E1** Raw container overhead — header, directory, metadata overhead; all
  formats. *Runnable now (Raw, Palette, Run-length, and Predictor are
  implemented).*
* **E2** Codec comparison per image and tile size. *Runnable now on the
  9-image seed corpus below for Raw, Palette, Run-length, and Predictor;
  dictionary and context-adaptive entropy wait on P2.* **Done for the
  research codecs in the P3.5 study** — see "Measured: P3.5 codec
  research" below.
* **E3** Hybrid vs best individual codec. *Runnable on the seed corpus. Auto
  selection was no worse than the best single codec on all 9 images (9/9) and
  strictly smaller on 1 (the large photo), because it mixes codecs per tile.*
* **E4** Tile size 32/64/128/256/512/1024: size, random-access latency, edit cost,
  memory. *Raw-only portion runnable now.*
* **E5** Region decode at multiple sizes and locations; verify only intersecting
  tiles load. *Runnable now (Raw).*
* **E6** Editing locality on a 16K image. **Done** — see "Measured: P4
  editing locality and 16K memory" below.
* **E7** 10 MB success distribution per category. **Done** — see "Measured: the
  750-image corpus" below: 748 of 750 images (99.7%) land under the decimal
  10,000,000-byte ceiling, with the per-category distribution and both
  failures named.
* **E8** Streaming performance: time to first preview/visible tile, panning,
  zoom transitions, cache hit rate, worker utilization. *Blocked on P5/P6.*
  The P4 streaming **writer** is done (tile-by-tile, one tile in memory);
  the streaming *reader* and the preview surface are not.

## Measured: the 750-image corpus (2026-11-21)

The corpus the roadmap asked for now exists. Driver
`benchmarks/corpus/run_corpus.py` over `benchmarks/corpus_manifest.json`
(version 2: 750 images, each with a canonical SHA-256, a source SHA-256 and
the commit it was read at); builder `benchmarks/corpus/build_corpus.py`;
source registry `benchmarks/corpus/sources.py`. Raw run preserved in
`benchmarks/p5_corpus_results.json`; the full report, with the twelve
questions, is `docs/P5_CORPUS_REPORT.md`.

Categories: photographs 300, screenshots 100, pixel art 100, scans 100,
textures 150. 746 images are real, fetched from five public repositories at
pinned commits; 4 are the project's own deterministic synthetic seed images.
Baselines are Pillow PNG (`compress_level=6`), Pillow WebP lossless
(`quality=100, method=4`) and `cjxl -d 0`, all encoded from the **same**
canonical pixels, on 749 of the 750 images (the 16K photograph is above the
32-megapixel baseline cap and was encoded by VPNG only). Tile size 256.

Median ratio (encoded bytes ÷ canonical bytes, lower is better):

| Category | Images | auto | raw | palette | rle | predictor | PNG | WebP | JPEG XL |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| photos | 300 | 0.602× | 1.000× | 1.006× | 0.971× | 0.602× | 0.591× | 0.429× | 0.387× |
| pixel_art | 100 | 0.165× | 1.012× | 0.201× | 0.429× | 0.266× | 0.140× | 0.065× | 0.067× |
| scans | 100 | 0.346× | 1.000× | 1.004× | 0.798× | 0.363× | 0.277× | 0.176× | 0.137× |
| textures | 150 | 0.378× | 1.001× | 1.004× | 0.849× | 0.387× | 0.367× | 0.246× | 0.228× |
| ui | 100 | 0.063× | 1.000× | 0.342× | 0.069× | 0.067× | 0.031× | 0.012× | 0.019× |
| **all** | 750 | **0.415×** | 1.000× | 0.336× | 0.839× | 0.439× | **0.379×** | **0.242×** | **0.219×** |

The palette column is over the 194 images it applies to: the palette codec
refuses any image with more than 256 distinct pixel values, and refused 556
of the 750.

Findings, stated as limits rather than claims:

* **VPNG is behind PNG on every category**, and behind WebP lossless and
  JPEG XL lossless by a wide margin: 0.415× against PNG's 0.379×, WebP's
  0.242× and JPEG XL's 0.219× across the corpus. The seed had hinted at
  this; 750 images measure it.
* **The 10 MB target succeeds on 748 of 750 images (99.7%)** — photographs
  299/300, screenshots 100/100, pixel art 100/100, scans 100/100, textures
  149/150. The failures are named: the 16K photograph at 124,197,035 bytes
  and one 2,048 × 2,048 texture at 11,397,940 bytes.
* **Auto selection never lost to the best forced codec** on any of the 750
  images, and was strictly smaller on 109 of them (1,541,810 bytes in total).
  The property the nine-image seed showed now holds on a corpus.
* **Eighteen images expand**: their file is larger than their pixels, the
  worst a 1 × 1 texture at 37.0×. The container's fixed overhead is larger
  than the image, and the corpus records it rather than hiding it.
* The corpus is **mostly small** — 461 of the 750 images are 512 pixels or
  less on the long edge, and three are larger than 4096 — so the large-image
  tail is two images, not a distribution.
* The corpus is **five repositories chosen for coverage**, not a random
  sample, and the category names are proxies for content types.

## Measured: the preview pyramid and the adaptive partition (2026-11-21)

Phase B, P6 and P7, in one format revision (**v0.1 → v0.2**; the 64-byte
header and the 40-byte directory entry keep their layout). Driver
`vpng-preview-bench` in `crates/vpng-validation` for P6 and
`benchmarks/corpus/run_adaptive.py` for P7. Raw runs preserved in
`benchmarks/p6_preview_results.json` and `benchmarks/p7_adaptive_results.json`;
the full report, with the twelve questions, is `docs/P6_P7_REPORT.md`.

Host: Intel Core i5-1235U, 10 physical cores / 12 logical processors, 7.7 GB
RAM, Windows `x86_64-pc-windows-gnu`, `rustc 1.100.0`, `--release` build.
Tile size 256, eight workers, `auto` codec.

### P6 — the preview pyramid (5 runs per case)

Source: the corpus's own 15,360 × 8,640 RGB photograph (398,131,200 canonical
bytes) — the same file the P5.2 latency matrix used.

| Case | Median | p95 | Mean | Sample stdev | File bytes | Tiles |
|---|---:|---:|---:|---:|---:|---:|
| v0.1, no pyramid | 18590.46 ms | 20911.24 ms | 18545.80 ms | 1635.36 ms | 124,197,035 | 2,040 |
| v0.2, 3 derived levels | 26809.86 ms | 31703.01 ms | 27909.44 ms | 2170.57 ms | 164,873,298 | 2,725 |

The v0.1 file's SHA-256 is
`d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf` — **the
P5.2 baseline's tile-256 digest**, so the format revision moved the default
path by nothing. The pyramid's levels are 15,360 × 8,640 (2,040 tiles),
7,680 × 4,320 (510), 3,840 × 2,160 (135) and 1,920 × 1,080 (40).

Decoding one whole level, five runs each:

| Level | Median | p95 | Pixels |
|---|---:|---:|---:|
| 0 — the original | 12962.95 ms | 14014.20 ms | 398,131,200 |
| 1 | 3371.25 ms | 3396.86 ms | 99,532,800 |
| 2 | 813.89 ms | 938.06 ms | 24,883,200 |
| 3 | 197.94 ms | 203.31 ms | 6,220,800 |

Findings, stated as limits rather than claims:

* **The previews cost bytes and time**: +32.7% file size (the previews are
  40,648,863 bytes) and +44.2% encode time. The encoder's ratio uses the
  *original* pixels, so a pyramid can never flatter it.
* **The whole picture arrives 66× sooner at 1/64 scale**: 197.94 ms for the
  smallest level against 13062.32 ms for the full-resolution picture.
* **The full-resolution path pays a little**: the first viewport costs
  34.41 ms from the pyramid against 31.61 ms from the v0.1 file (+8.8% at
  the median), because the directory holds 685 more entries.
* **Memory, with its scope:** the pyramid encode peaked at **547,844,096 B**
  of resident set and **541,421,345 B** of heap, against the 528,768,000 B of
  pixels the design requires it to hold (the picture and its three previews).
  *Included:* the whole picture and every derived level held at once, tile
  buffers, encoded payloads, container metadata, the binary and the
  allocator's bookkeeping. *Excluded:* the pixel file on disk, the page
  cache, other processes and the driver's own memory. This is **not** the
  64 MB localized-edit target, which is a separate claim measured by the
  P4 memory matrix.

### P7 — adaptive tiling over the 750-image corpus

Every image was encoded twice — uniform grid and adaptive partition — at tile
size 256 with the `auto` codec, and the denominator is the same canonical
pixel bytes the corpus report used.

| | Uniform | Adaptive |
|---|---:|---:|
| Total bytes over the corpus | 453,767,460 | **452,762,594** |
| Median ratio (bytes ÷ canonical) | 0.415337 | **0.413257** |
| p95 ratio | 0.801071 | 0.801958 |
| Median tile count | 4 | **2** |
| Under the 10 MB ceiling | 748/750 | 748/750 |

| Category | Images | Uniform | Adaptive | Median delta | Smaller | Larger | Equal |
|---|---:|---:|---:|---:|---:|---:|---:|
| photos | 300 | 0.602 | 0.601 | −271 B | 179 | 105 | 16 |
| scans | 100 | 0.346 | 0.346 | −346.5 B | 68 | 32 | 0 |
| textures | 150 | 0.378 | 0.377 | 0 B | 49 | 31 | 70 |
| pixel art | 100 | 0.165 | 0.165 | 0 B | 4 | 4 | 92 |
| ui | 100 | 0.063 | 0.064 | +826.5 B | 30 | 69 | 1 |

Findings, stated as limits rather than claims:

* **Adaptive tiling is a compression wash**: −0.22% over the corpus
  (−1,004,866 bytes), 330 images smaller, 241 larger, 179 identical. The
  median image gains nothing.
* **It helps photographs and scans, and it *hurts* UI screenshots** — the
  category where a uniform grid is already nearly optimal.
* **The claim it does support is about the partition**: half the tiles at the
  median (4 → 2), which is a smaller directory and fewer requests to serve a
  region.
* **No pixels changed and no container broke**: 750 of 750 adaptive files
  decoded exactly (`vpng verify`) and 0 failed `vpng check`. The partition
  is validated as an exact cover before the file is written.
* The largest single win is the 16K photograph itself: 124,197,035 →
  123,202,141 bytes (−994,894 B) with 2,040 → 608 tiles. The largest single
  loss is `threejs-textures-a9aefa111bdb` at +127,952 bytes.

## Measured: large-image stress (2026-11-15, with Predictor)

Host: 12 logical CPUs, 7,916 MB physical RAM, Windows `x86_64-pc-windows-gnu`,
`rustc 1.100.0`, `--release` build, tile size 256. Driver:
`benchmarks/stress/run_stress.ps1`; raw run preserved in
`benchmarks/results/stress_20261115_130649.json`. Peak RSS is sampled every
~20 ms, so it is a lower bound on the true peak. Canonical SHA-256 and `verify`
were checked for every row; `check` returned 0 (all payload CRCs valid).

| Image | Format | Canonical bytes | Codec | Output bytes | Ratio | Encode ms | Verify ms | Peak RSS MB |
|---|---|---:|---|---:|---:|---:|---:|---:|
| `1006677.jpg` 15360×8640 | rgb8 | 398,131,200 | raw | 398,212,868 | 1.000× | 2,205 | 2,077 | 1,142.6 |
| `1006677.jpg` 15360×8640 | rgb8 | 398,131,200 | predictor | 124,199,623 | 0.312× | 54,823 | 14,906 | 882.0 |
| `1006677.jpg` 15360×8640 | rgb8 | 398,131,200 | auto | 124,197,035 | 0.312× | 56,789 | 14,998 | 882.0 |
| 16K gradient 16384×16384 | gray8 | 268,435,456 | raw | 268,599,364 | 1.001× | 1,512 | 1,090 | 772.0 |
| 16K gradient 16384×16384 | gray8 | 268,435,456 | predictor | 8,024,132 | 0.0299× | 19,995 | 8,388 | 523.3 |
| 16K gradient 16384×16384 | gray8 | 268,435,456 | auto | 8,024,132 | 0.0299× | 26,078 | 10,779 | 523.3 |
| 8K pattern 8192×8192 | rgba8 | 268,435,456 | raw | 268,476,484 | 1.000× | 1,448 | 1,547 | 771.7 |
| 8K pattern 8192×8192 | rgba8 | 268,435,456 | predictor | 7,564,524 | 0.0282× | 22,967 | 8,492 | 523.6 |
| 8K pattern 8192×8192 | rgba8 | 268,435,456 | auto | 7,564,524 | 0.0282× | 22,520 | 8,017 | 523.6 |

Findings:

* The predictor codec changes the picture completely on this corpus. The photo
  goes from 308,132,486 bytes (auto, 2026-11-14) to **124,197,035** bytes
  (0.774× → **0.312×**). The 16K gray8 gradient goes from 268,599,364 bytes
  (1.001×, every tile Raw) to **8,024,132** bytes (**0.0299×**), and the 8K
  RGBA8 pattern from 268,476,484 (1.000×, every tile Raw) to **7,564,524**
  (**0.0282×**).
* The photo's auto file uses Predictor for 2,038 tiles and Palette for 2 of
  2,040; mixing beats the best single codec (forced predictor,
  124,199,623 bytes) by 2,588 bytes.
* The **cost of the measurement is visible**: the predictor runs take 20–55 s
  to encode where Raw takes 1.4–2.2 s, because every row is measured under all
  seven predictors before one is kept. Decode stays in the same range as Raw
  (8.4–14.9 s), and peak RSS is *lower* than the Raw path on every row, because
  the payload and its directory are smaller than the raw tile bytes.
* `check` cost 64–262 ms and a 512×512 region decode 416–553 ms on these files;
  the region path does not decode unrelated tiles.
* **16K RGBA8 was not run.** At 1 GiB of canonical pixels it needs at least
  3.4 GiB of live buffers for encode plus decode; the highest free physical
  memory observed was 1,445 MB, so the harness skipped it with that reason.
  A recorded 16K RGBA8 run waits on P6 streaming/allocation work.

### Previous stress run (2026-11-14, without Predictor)

Kept for comparison. Host and driver are the same; the earlier run had
2,040 tiles encoded with Palette (330) and Run-length (1,710).

| Image | Codec | Output bytes | Encode ms | Verify ms | Peak RSS MB |
|---|---|---:|---:|---:|---:|
| `1006677.jpg` 15360×8640 | raw | 398,212,868 | 3,564 | 1,999 | 1,142.9 |
| `1006677.jpg` 15360×8640 | auto | 308,132,486 (0.774×) | 3,748 | 2,336 | 980.1 |
| 16K gradient 16384×16384 | raw | 268,599,364 | 5,363 | 965 | 772.0 |
| 16K gradient 16384×16384 | auto | 268,599,364 (1.001×) | 4,308 | 937 | 772.3 |
| 8K pattern 8192×8192 | raw | 268,476,484 | 1,413 | 925 | 771.7 |
| 8K pattern 8192×8192 | auto | 268,476,484 (1.000×) | 1,408 | 841 | 773.0 |

## Measured: seed corpus (2026-11-15, with Predictor)

Nine images in five categories from `benchmarks/corpus_manifest.json`; driver
`benchmarks/corpus/run_corpus.py`; baselines Pillow 11.3.0 PNG
(`compress_level=6`) and WebP lossless (`quality=100, method=4`), both encoded
from the same canonical pixels. Ratios are encoded bytes divided by canonical
pixel bytes (lower is better). Images above 32 MP were encoded by VPNG only.
JPEG XL and FLIF were not compared **in this run**: no JPEG XL encoder was
installed on this host at the time (one is now — see the P3.5 study below).
Raw run
preserved in `benchmarks/results/corpus_20261115_130406.json`.

| Image | Category | Dimensions | VPNG auto | VPNG raw | VPNG predictor | PNG | WebP |
|---|---|---|---:|---:|---:|---:|---:|
| `commons-great-wave-1280` | digital_paintings | 1280×883 | 0.704× | 1.000× | 0.704× | 0.627× | 0.424× |
| `commons-starry-night-1280` | digital_paintings | 1280×1013 | 0.800× | 1.000× | 0.800× | 0.814× | 0.564× |
| `synthetic-noise-256-rgba8` | noise | 256×256 | 1.000× | 1.000× | 1.016× | 1.002× | 0.998× |
| `commons-cat-1280` | photographs | 1280×1709 | 0.509× | 1.000× | 0.509× | 0.481× | 0.288× |
| `commons-example-1024` | photographs | 172×178 | 0.287× | 1.001× | 0.287× | 0.264× | 0.184× |
| `user-photo-15360x8640` | photographs | 15360×8640 | 0.312× | 1.000× | 0.312× | — | — |
| `synthetic-checker-512-rgba8` | procedural | 512×512 | 0.030× | 1.000× | 0.030× | 0.003× | 0.000× |
| `synthetic-gradient-512-rgb8` | procedural | 512×512 | 0.079× | 1.000× | 0.079× | 0.006× | 0.002× |
| `synthetic-ui-1024x768-rgb8` | screenshots | 1024×768 | 0.014× | 1.000× | 0.027× | 0.002× | 0.000× |

Category medians (auto / raw / predictor / PNG / WebP):

| Category | Images | auto | raw | predictor | PNG | WebP |
|---|---:|---:|---:|---:|---:|---:|
| digital_paintings | 2 | 0.752× | 1.000× | 0.752× | 0.721× | 0.494× |
| noise | 1 | 1.000× | 1.000× | 1.016× | 1.002× | 0.998× |
| photographs | 3 | 0.312× | 1.000× | 0.312× | 0.372× | 0.236× |
| procedural | 2 | 0.054× | 1.000× | 0.054× | 0.004× | 0.001× |
| screenshots | 1 | 0.014× | 1.000× | 0.027× | 0.002× | 0.000× |

Findings, stated as limits rather than claims:

* Auto selection was again no worse than the best forced codec on all 9 images
  (`auto <= best` 9/9), confirming that per-tile mixing never loses to a single
  codec. It is strictly better on 1 image: on the large photo, auto uses
  Predictor for 2,038 tiles and Palette for 2, beating forced predictor by
  2,588 bytes.
* The **predictor codec is the new workhorse**: it wins outright on the photo
  (0.312×, from 0.774× before), the cat (0.509×, from 0.830×), the great wave
  (0.704×, from 0.985×), the small example (0.287×, from 0.372×), and the
  gradient (0.079×, from 0.793×).
* The **Raw fallback still holds**: on the noise image a forced predictor run
  expands to 1.016×, and auto correctly rejects it and falls back to Raw at
  1.000×. On the screenshot image forced predictor gives 0.027× while Run-length
  gives 0.014×, and auto correctly picks Run-length.
* VPNG still loses to PNG, WebP lossless and JPEG XL lossless on photographs
  and paintings. The
  dictionary (id 4), predictor+dictionary (id 5), and context-adaptive entropy
  (id 6) codecs do not exist yet, and the current entropy coder prices a
  residual's sign and magnitude planes without any context modelling of the
  image itself.
* Nine images across five categories are a **seed**, not the 750-image target.
  No distribution, per-resolution, or 10 MB success claim follows from it.

### Previous seed-corpus run (2026-11-14, without Predictor)

Kept for comparison. Host, corpus, and baselines are the same.

| Image | Category | Dimensions | VPNG auto | VPNG raw | PNG | WebP |
|---|---|---|---:|---:|---:|---:|
| `commons-great-wave-1280` | digital_paintings | 1280×883 | 0.985× | 1.000× | 0.627× | 0.424× |
| `commons-starry-night-1280` | digital_paintings | 1280×1013 | 1.000× | 1.000× | 0.814× | 0.564× |
| `synthetic-noise-256-rgba8` | noise | 256×256 | 1.000× | 1.000× | 1.002× | 0.998× |
| `commons-cat-1280` | photographs | 1280×1709 | 0.830× | 1.000× | 0.481× | 0.288× |
| `commons-example-1024` | photographs | 172×178 | 0.372× | 1.001× | 0.264× | 0.184× |
| `user-photo-15360x8640` | photographs | 15360×8640 | 0.774× | 1.000× | — | — |
| `synthetic-checker-512-rgba8` | procedural | 512×512 | 0.032× | 1.000× | 0.003× | 0.000× |
| `synthetic-gradient-512-rgb8` | procedural | 512×512 | 0.793× | 1.000× | 0.006× | 0.002× |
| `synthetic-ui-1024x768-rgb8` | screenshots | 1024×768 | 0.014× | 1.000× | 0.002× | 0.000× |

## Measured: P3.5 codec research (2026-11-16)

Host: 12 logical CPUs, 7,916 MB physical RAM, Windows
`x86_64-pc-windows-gnu`, `rustc 1.100.0`, `--release` build, tile size 256.
Driver: `benchmarks/corpus/run_research.py` (binary `vpng-research`).
Baselines from `benchmarks/corpus/baselines.py`: Pillow PNG
(`compress_level=6`), Pillow WebP lossless (`quality=100, method=4`),
`cjxl -d 0` where installed. Raw results:
`benchmarks/p3_5_raw_results.json` (4.3 MB) and
`benchmarks/p3_5_comparison.csv` (459 rows); the full per-tile record is
under `benchmarks/results/research-raw/`.

Eleven codecs were measured on the **same canonical pixels** and the
**same tile layout**. The reported figure for every codec is the size of
the *file* it would produce: the payload, plus the raw fallback for the
tiles a codec cannot represent, plus the container framing.

### The whole corpus (9 images, 416,522,696 canonical bytes)

The baselines are skipped by default above a 32-megapixel cap, so the corpus
was re-measured with `--max-baseline-pixels 200000000`. In that run **PNG,
WebP and JPEG XL all completed for every image**, so every column below has
the same denominator. An earlier attempt failed on the largest photograph with
Pillow's `can't allocate picture frame` — a host memory condition; it
succeeded when the run was repeated with more free memory. The failure is
recorded here rather than quietly dropped, and the successful measurement is
the one quoted.

| Codec | Bytes | Ratio | Median | Worst |
|---|---:|---:|---:|---:|
| JPEG XL lossless (baseline) | 91,196,450 | **0.2189** | 0.2160 | 1.0827 |
| WebP lossless (baseline) | 107,261,534 | 0.2575 | 0.2549 | 0.9984 |
| CARC C (research) | 127,523,075 | 0.3062 | 0.2995 | 1.0170 |
| CARC D (research) | 130,155,579 | 0.3125 | 0.3052 | 1.0191 |
| Predictor + entropy (stable 3) | 133,500,355 | 0.3205 | 0.3120 | 1.0161 |
| CARC A (research) | 133,500,355 | 0.3205 | 0.3120 | 1.0161 |
| CARC B (research) | 139,231,075 | 0.3343 | 0.3264 | 1.0193 |
| PNG (baseline) | 143,484,520 | 0.3445 | 0.3384 | 1.0021 |
| Predictor→LZ77→entropy (research) | 181,170,505 | 0.4350 | 0.4304 | 1.0157 |
| LZ77→entropy (research) | 194,163,860 | 0.4662 | 0.4599 | 1.0150 |
| LZ77 (research) | 333,574,485 | 0.8009 | 0.7997 | 1.1732 |
| Run-length (stable 2) | 342,347,307 | 0.8219 | 0.8254 | 1.0024 |
| Predictor→LZ77 (research) | 346,839,309 | 0.8327 | 0.8286 | 1.2347 |
| Palette (stable 1, partial) | 369,128,419 | 0.8862 | 1.0002 | 1.0012 |
| Raw (stable 0) | 416,617,300 | 1.0002 | 1.0003 | 1.0012 |

### Per category

Each row covers the same images for every column: all the images of that
category in the corpus.

| Category | Images | CARC C | PNG | WebP | JPEG XL | Pred→LZ77→entropy | Raw |
|---|---:|---:|---:|---:|---:|---:|---:|
| photographs | 3 | 0.3017 | 0.3407 | 0.2554 | 0.2165 | 0.4320 | 1.0002 |
| digital paintings | 2 | 0.6841 | 0.7270 | 0.4987 | 0.4495 | 0.8240 | 1.0003 |
| procedural | 2 | 0.0473 | 0.0038 | 0.0010 | 0.0095 | 0.0115 | 1.0003 |
| screenshots | 1 | 0.0277 | 0.0023 | 0.0003 | 0.0006 | 0.0040 | 1.0003 |
| noise | 1 | 1.0170 | 1.0021 | 0.9984 | 1.0827 | 1.0157 | 1.0004 |

### Per resolution

| Resolution class | Images | CARC C | PNG | WebP | JPEG XL |
|---|---:|---:|---:|---:|---:|
| 512 | 4 | 0.1720 | 0.1343 | 0.1281 | 0.1446 |
| 1024 | 1 | 0.0277 | 0.0023 | 0.0003 | 0.0006 |
| 2048 | 3 | 0.5677 | 0.6104 | 0.3986 | 0.3537 |
| 16384 | 1 | 0.2995 | 0.3384 | 0.2548 |

### Worst cases

| Codec | Worst image | Ratio | Worst tile | Tile ratio |
|---|---|---:|---|---:|
| LZ77 | `commons-starry-night-1280` | **1.1732** | tile 16 | 1.2333 |
| Predictor→LZ77 | `commons-starry-night-1280` | **1.2347** | tile 7 | 1.2487 |
| CARC B | `synthetic-noise-256-rgba8` | 1.0193 | tile 0 | 1.0189 |
| CARC D | `synthetic-noise-256-rgba8` | 1.0191 | tile 0 | 1.0187 |
| CARC C | `synthetic-noise-256-rgba8` | 1.0170 | tile 0 | 1.0166 |
| Predictor + entropy | `synthetic-noise-256-rgba8` | 1.0161 | tile 0 | 1.0157 |
| Run-length | `synthetic-noise-256-rgba8` | 1.0024 | tile 0 | 1.0020 |
| Raw | `commons-example-1024` | 1.0012 | — | 1.0000 |

### The largest image, broken out

The 15360×8640 photograph on its own. It is 96% of the corpus's pixels, so
it dominates the whole-corpus totals above; its own numbers are the honest
way to see what that dominance does. All three baselines were measured on
this frame.

| Codec | Bytes | Ratio |
|---|---:|---:|
| JPEG XL lossless (baseline) | 85,982,362 | 0.2160 |
| WebP lossless (baseline) | 101,462,746 | 0.2549 |
| CARC C (research) | 119,222,852 | 0.2995 |
| CARC D (research) | 121,496,780 | 0.3052 |
| Predictor + entropy (stable 3) | 124,207,779 | 0.3120 |
| PNG (baseline) | 134,734,803 | 0.3384 |
| Predictor→LZ77→entropy (research) | 171,365,202 | 0.4304 |
| Raw (stable 0) | 398,221,024 | 1.0002 |

Findings, stated as limits rather than claims:

* **CARC C is the strongest research result**: 4.5% smaller than the
  stable predictor codec and 11.1% smaller than PNG across the corpus,
  and 11.5% smaller than PNG on the largest photograph. It is **18.9%
  worse than WebP lossless**.
* **On synthetic and structured content, PNG and WebP are one to two orders
  of magnitude smaller than any VPNG codec.** A screenshot is 0.0023× (PNG)
  and 0.0003× (WebP) against 0.0277× for the best VPNG candidate. This is
  the uncomfortable part of the result and it is kept in full.
* **Two codecs expand the data**: standalone LZ77 expands the Starry Night
  painting by 17.3% and one of its tiles by 23.3%; the predictor→LZ77
  hybrid expands one tile by 24.9%. Every predictor-based codec expands
  the pure-noise image by 1.5–1.9%. The selection logic never chooses an
  expanding codec — the candidate comparison is over *complete* payload
  sizes and Raw is always a candidate — but the measurements are kept.
* **The entropy stage is what makes the dictionary path competitive**:
  LZ77→entropy is 0.4662 against standalone LZ77's 0.8009.
* **Nine images are a seed, not the 750-image corpus.** No distribution,
  per-resolution, or 10 MB success claim follows from this study, and no
  statistical significance is claimed.
* **No throughput comparison was run.** The study measured size, not speed.
* **No experimental codec is promoted.** The research ids cannot reach a
  file; see `docs/P4_COMPATIBILITY_REPORT.md`.

## Measured: P4 editing locality and 16K memory (2026-11-16)

Host: 12 logical CPUs, 7,916 MB physical RAM (677 MB free at the start),
Windows `x86_64-pc-windows-gnu`, `rustc 1.100.0`, `--release` build.
Drivers: `vpng-editing-bench` and `benchmarks/memory/run_memory.py`. Raw
results: `benchmarks/p4_editing_results.json` and
`benchmarks/p4_memory_results.json`. Reports:
`docs/P4_EDITING_ENGINE_REPORT.md` and `docs/P4_MEMORY_VALIDATION.md`.

### Editing locality (E6)

A 64×64 edit on a 2048×2048 RGBA8 image with a 256-pixel tile grid
(64 tiles, 16 MiB of pixels, a 4,309,019-byte file).

The four tile counts are **different things** — see
`docs/P4_EDITING_ENGINE_REPORT.md` §3.0 for the definitions. **Decoded** is
not **rewritten**: the edit decodes the tile it patches, and the commit
re-encodes the tile it modified.

| Quantity | Value |
|---|---:|
| Tiles **decoded** (payload read and turned back into pixels) | **1** |
| Tiles **modified** (pixels changed; the dirty set) | **1** (262,144 bytes = 1.5625% of the image) |
| Tiles **rewritten** (encoded payload replaced at commit) | **1** |
| Tiles **preserved** (original payload copied byte for byte) | **63** |
| Region read / write | 10.3 ms / 0.10 ms |
| Commit | 46.0 ms |
| Payload bytes copied / re-encoded | 4,239,148 / 67,312 |
| Identical payloads before vs after | 63 (only the edited tile changed) |

The `localized-edit` scenario performs no commit, so it rewrites and
preserves nothing; the `payload-reuse` scenario commits, and reports 1
rewritten and 63 preserved.

### 16K memory matrix

Target: **≤ 64 MB** of CPU working memory for a localized edit. Each case
runs in its own process, so the resident-set high-water mark belongs to
that case alone. **12 PASS, 0 FAIL, 0 BLOCKED.**

| Case | Canonical | File | Write | Write heap | Write RSS | Edit RSS Δ | Edit heap | Decoded tiles |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 4096² `gray8` | 16 MB | 2.4 MB | 2.2 s | 1.69 MB | 4.91 MB | 2.14 MB | 2.12 MB | 1 |
| 4096² `rgb8` | 48 MB | 6.8 MB | 6.0 s | 4.59 MB | 7.87 MB | 5.39 MB | 5.76 MB | 1 |
| 4096² `rgba8` | 64 MB | 8.9 MB | 7.6 s | 5.94 MB | 9.89 MB | 8.12 MB | 7.67 MB | 1 |
| 4096² `rgba16` | 128 MB | 10.0 MB | 12.9 s | 8.75 MB | 12.55 MB | 12.33 MB | 12.48 MB | 1 |
| 8192² `gray8` | 64 MB | 9.8 MB | 9.4 s | 1.70 MB | 5.07 MB | 2.73 MB | 2.12 MB | 1 |
| 8192² `rgb8` | 192 MB | 27.4 MB | 24.2 s | 4.61 MB | 7.94 MB | 3.85 MB | 5.77 MB | 1 |
| 8192² `rgba8` | 256 MB | 35.9 MB | 31.1 s | 5.97 MB | 9.74 MB | 7.62 MB | 7.68 MB | 1 |
| 8192² `rgba16` | 512 MB | 40.5 MB | 68.3 s | 8.78 MB | 12.66 MB | 13.12 MB | 12.49 MB | 1 |
| 16384² `gray8` | 256 MB | 39.8 MB | 30.9 s | 1.74 MB | 5.73 MB | 2.98 MB | 2.16 MB | 1 |
| 16384² `rgb8` | 768 MB | 110.2 MB | 84.0 s | 4.66 MB | 8.92 MB | 4.97 MB | 5.80 MB | 1 |
| 16384² `rgba8` | 1,024 MB | 144.6 MB | 113.0 s | 6.02 MB | 10.07 MB | 8.76 MB | 7.72 MB | 1 |
| 16384² `rgba16` | **2,048 MB** | 163.3 MB | 199.5 s | 8.83 MB | 13.15 MB | **13.16 MB** | **12.52 MB** | 1 |

Findings, stated as limits rather than claims:

* **The memory figures are flat across the resolutions.** The 4096² and the
  16384² RGBA16 cases both use ~12.5 MB of heap, even though the second
  image's pixels are **16× larger**. The engine's memory is a function of
  the tile size and the budgets, not of the image's size.
* **The worst case in the matrix uses 21% of the 64 MB target** on the
  resident-set delta and 20% on the heap.
* **Exactly one tile is decoded, exactly one is modified, exactly one is
  rewritten, and the rest are preserved** in every case; 1023 of 1024
  payloads are copied verbatim in the 16K cases.
* **The write times are 2–200 s** and are dominated by the predictor
  codec's arithmetic coding. No speed claim is made, and the earlier
  "16K RGBA8 was skipped for insufficient free memory" note in the stress
  section above is now superseded: the streaming writer and the file-backed
  editor handle that case in 12.4 MB of resident set.

## Reporting rules

Report mean, median, p95, and p99 where relevant; per-category, per-resolution,
and per-codec results; worst cases; failures and skipped cases. Aggregate results
only alongside per-category results. Never claim "VPNG is better" from one image
or one category.
