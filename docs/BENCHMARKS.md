# Benchmarks

Every number in a report must name its hardware and corpus. No performance claim
is valid until it is measured here, with the raw run preserved.

## Method

1. `benchmarks/corpus_manifest.json` records every image, its provenance,
   license, dimensions, category, and SHA-256, so a run is reproducible.
2. Runs write machine-readable JSON into `benchmarks/results/` (git-ignored) and
   human reports into `benchmarks/reports/`.
3. Each record captures: CPU model, OS, RAM, Rust version, build profile and
   flags, worker count, input dimensions and format, tile size, codec choices,
   input bytes, output bytes, encode time, decode time, region-decode time,
   edit/commit time, peak RSS, internal allocation peak, and verification result.

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
  formats. *Runnable now (Raw and Palette are implemented).*
* **E2** Codec comparison per image and tile size. *Partially unblocked: Raw vs
  Palette are runnable; the rest wait on P2.*
* **E3** Hybrid vs best individual codec. *Partially unblocked: Raw vs Palette
  vs auto-selection are runnable.*
* **E4** Tile size 32/64/128/256/512/1024: size, random-access latency, edit cost,
  memory. *Raw-only portion runnable now.*
* **E5** Region decode at multiple sizes and locations; verify only intersecting
  tiles load. *Runnable now (Raw).*
* **E6** Editing locality on a 16K image. *Blocked on P4.*
* **E7** 10 MB success distribution per category. *Blocked on a corpus.*
* **E8** Streaming performance: time to first preview/visible tile, panning,
  zoom transitions, cache hit rate, worker utilization. *Blocked on P5/P6.*

## Reporting rules

Report mean, median, p95, and p99 where relevant; per-category, per-resolution,
and per-codec results; worst cases; failures and skipped cases. Aggregate results
only alongside per-category results. Never claim "VPNG is better" from one image
or one category.
