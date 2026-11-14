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
  formats. *Runnable now (Raw, Palette, and Run-length are implemented).*
* **E2** Codec comparison per image and tile size. *Partially unblocked: Raw,
  Palette, and Run-length run on the 9-image seed corpus below; predictor,
  dictionary, and entropy wait on P2.*
* **E3** Hybrid vs best individual codec. *Runnable on the seed corpus. Auto
  selection was no worse than the best single codec on all 9 images (9/9) and
  strictly smaller on 4, because it mixes codecs per tile.*
* **E4** Tile size 32/64/128/256/512/1024: size, random-access latency, edit cost,
  memory. *Raw-only portion runnable now.*
* **E5** Region decode at multiple sizes and locations; verify only intersecting
  tiles load. *Runnable now (Raw).*
* **E6** Editing locality on a 16K image. *Blocked on P4.*
* **E7** 10 MB success distribution per category. *Blocked on the full corpus; the
  9-image seed cannot support a distribution claim.*
* **E8** Streaming performance: time to first preview/visible tile, panning,
  zoom transitions, cache hit rate, worker utilization. *Blocked on P5/P6.*

## Measured: large-image stress (2026-11-14)

Host: 12 logical CPUs, 7,916 MB physical RAM, Windows `x86_64-pc-windows-gnu`,
`rustc 1.100.0`, `--release` build, tile size 256. Driver:
`benchmarks/stress/run_stress.ps1`. Peak RSS is sampled every ~20 ms, so it is a
lower bound on the true peak. Canonical SHA-256 and `verify` were checked for
every row; `check` returned 0 (all payload CRCs valid).

| Image | Format | Canonical bytes | Codec | Output bytes | Ratio | Encode ms | Verify ms | Peak RSS MB |
|---|---|---:|---|---:|---:|---:|---:|---:|
| `1006677.jpg` 15360×8640 | rgb8 | 398,131,200 | raw | 398,212,868 | 1.000× | 3,564 | 1,999 | 1,142.9 |
| `1006677.jpg` 15360×8640 | rgb8 | 398,131,200 | auto | 308,132,486 | 0.774× | 3,748 | 2,336 | 980.1 |
| 16K gradient 16384×16384 | gray8 | 268,435,456 | raw | 268,599,364 | 1.001× | 5,363 | 965 | 772.0 |
| 16K gradient 16384×16384 | gray8 | 268,435,456 | auto | 268,599,364 | 1.001× | 4,308 | 937 | 772.3 |
| 8K pattern 8192×8192 | rgba8 | 268,435,456 | raw | 268,476,484 | 1.000× | 1,413 | 925 | 771.7 |
| 8K pattern 8192×8192 | rgba8 | 268,435,456 | auto | 268,476,484 | 1.000× | 1,408 | 841 | 773.0 |

Findings:

* The photo's auto file uses Palette for 330 tiles and Run-length for 1,710 of
  2,040 tiles; mixing codecs beats the best single codec (Run-length,
  328,620,067 bytes) by 20.5 MB.
* The 16K gray8 gradient and the 8K RGBA8 pattern fall back to Raw for every
  tile: the gradient has more than 256 values per tile (Palette not
  representable) and no runs long enough to pay for RLE's overhead.
* `check` cost 197–404 ms and a 512×512 region decode 180–402 ms on these files;
  the region path does not decode unrelated tiles.
* **16K RGBA8 was not run.** At 1 GiB of canonical pixels it needs at least
  3.4 GiB of live buffers for encode plus decode; the highest free physical
  memory observed was 1,712 MB, so the harness skipped it with that reason.
  A recorded 16K RGBA8 run waits on P6 streaming/allocation work.

## Measured: seed corpus (2026-11-14)

Nine images in five categories from `benchmarks/corpus_manifest.json`; driver
`benchmarks/corpus/run_corpus.py`; baselines Pillow 11.3.0 PNG
(`compress_level=6`) and WebP lossless (`quality=100, method=4`), both encoded
from the same canonical pixels. Ratios are encoded bytes divided by canonical
pixel bytes (lower is better). Images above 32 MP were encoded by VPNG only.
JPEG XL and FLIF were not compared: no encoder is installed here.

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

Category medians (auto / raw / PNG / WebP):

| Category | Images | auto | raw | PNG | WebP |
|---|---:|---:|---:|---:|---:|
| digital_paintings | 2 | 0.993× | 1.000× | 0.721× | 0.494× |
| noise | 1 | 1.000× | 1.000× | 1.002× | 0.998× |
| photographs | 3 | 0.774× | 1.000× | 0.372× | 0.236× |
| procedural | 2 | 0.412× | 1.000× | 0.004× | 0.001× |
| screenshots | 1 | 0.014× | 1.000× | 0.002× | 0.000× |

Findings, stated as limits rather than claims:

* Auto selection was no worse than the best forced codec on all 9 images
  (`auto <= best` 9/9), confirming that per-tile mixing never loses to a single
  codec.
* VPNG currently loses to PNG and WebP on photographs and paintings. Only Raw,
  Palette, and Run-length exist; there is no predictor, dictionary, or entropy
  coder yet, which is exactly why those codecs are next.
* VPNG wins where its implemented codecs match the content: flat/synthetic UI
  (RLE), few-colour tiles (Palette), and smooth regions of the large photo.
* Nine images across five categories are a **seed**, not the 750-image target.
  No distribution, per-resolution, or 10 MB success claim follows from it.

## Reporting rules

Report mean, median, p95, and p99 where relevant; per-category, per-resolution,
and per-codec results; worst cases; failures and skipped cases. Aggregate results
only alongside per-category results. Never claim "VPNG is better" from one image
or one category.
