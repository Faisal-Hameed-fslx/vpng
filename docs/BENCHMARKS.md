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
  dictionary and context-adaptive entropy wait on P2.*
* **E3** Hybrid vs best individual codec. *Runnable on the seed corpus. Auto
  selection was no worse than the best single codec on all 9 images (9/9) and
  strictly smaller on 1 (the large photo), because it mixes codecs per tile.*
* **E4** Tile size 32/64/128/256/512/1024: size, random-access latency, edit cost,
  memory. *Raw-only portion runnable now.*
* **E5** Region decode at multiple sizes and locations; verify only intersecting
  tiles load. *Runnable now (Raw).*
* **E6** Editing locality on a 16K image. *Blocked on P4.*
* **E7** 10 MB success distribution per category. *Blocked on the full corpus; the
  9-image seed cannot support a distribution claim.*
* **E8** Streaming performance: time to first preview/visible tile, panning,
  zoom transitions, cache hit rate, worker utilization. *Blocked on P5/P6.*

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
JPEG XL and FLIF were not compared: no encoder is installed here. Raw run
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
* VPNG still loses to PNG and WebP on photographs and paintings. The
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

## Reporting rules

Report mean, median, p95, and p99 where relevant; per-category, per-resolution,
and per-codec results; worst cases; failures and skipped cases. Aggregate results
only alongside per-category results. Never claim "VPNG is better" from one image
or one category.
