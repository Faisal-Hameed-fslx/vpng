# P3.5 — Compression research report

**Gate:** P3.5 (compression research) — **PASS**
**Date:** 2026-11-16
**Toolchain:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `x86_64-pc-windows-gnu`, release profile
**Raw data:** `benchmarks/p3_5_raw_results.json` (4.3 MB), `benchmarks/p3_5_comparison.csv` (459 rows)
**Full per-tile dumps:** `benchmarks/results/research-raw/*.json` (git-ignored, 10.4 MB)

> This report covers **one half** of the P3.5 + P4 milestone. The editing
> engine is reported separately in `docs/P4_EDITING_ENGINE_REPORT.md`. The two
> gates are independent: nothing in this report depends on the editing engine,
> and nothing in that report depends on these codecs.

---

## 1. What was built

Eleven codecs were measured on the same pixels: the four stable codecs
(ids 0–3), four context-adaptive residual models (CARC A–D), an LZ77
dictionary codec, and three hybrids.

| Codec | Id | Kind | Stable? | Where |
|---|---:|---|---|---|
| Raw | 0 | stored pixels | yes | `crates/vpng-codecs/src/raw.rs` |
| Palette | 1 | per-tile palette | yes | `crates/vpng-codecs/src/palette.rs` |
| Run-length | 2 | pixel-counted runs | yes | `crates/vpng-codecs/src/rle.rs` |
| Predictor + entropy | 3 | seven spatial predictors, arithmetic coder | yes | `crates/vpng-codecs/src/predictor.rs` |
| CARC A | `0x80` | context-adaptive residual, stable predictor | **research only** | `crates/vpng-codecs/src/experimental/carc.rs` |
| CARC B | `0x81` | context-adaptive residual, gradient classifier | **research only** | same |
| CARC C | `0x82` | context-adaptive residual, residual classifier | **research only** | same |
| CARC D | `0x83` | context-adaptive residual, combined classifier | **research only** | same |
| LZ77 | `0x90` | byte-level dictionary matcher | **research only** | `crates/vpng-codecs/src/experimental/lz77.rs` |
| Predictor → LZ77 | `0x91` | hybrid | **research only** | `crates/vpng-codecs/src/experimental/hybrid.rs` |
| LZ77 → entropy | `0x92` | hybrid | **research only** | same |
| Predictor → LZ77 → entropy | `0x93` | hybrid | **research only** | same |

The stable ids 4, 5 and 6 (LZ77, predictor+dictionary, context-adaptive
entropy) remain **reserved and unassigned**. No experimental id is reachable
from `get_codec`, none is listed in `IMPLEMENTED_CODEC_IDS`, and the
production decoder rejects any tile that names one with
`ErrorKind::UnsupportedFeature`. An experimental codec can therefore never
appear in a decodable file — see §7.

---

## 2. Method

**Identical pixels.** One canonicalisation pipeline produces the raw buffer
(`benchmarks/stress/canonicalize.py`); every codec and every baseline sees
the *same* bytes. The canonical digest is SHA-256 over
`"VPNG-CANON-v1\0" || le_u32(w) || le_u32(h) || u8(model) || u8(bits) || pixels`,
and each image's digest is checked against `benchmarks/corpus_manifest.json`
before it is measured.

**Identical tile layouts.** Every codec is measured on the same 256×256 tile
grid. A codec cannot choose a favourable tile size.

**Whole-file comparison.** A PNG or a WebP file is a whole file, so a payload
total is not comparable with it. The reported figure for every codec is the
size of the file that *would* exist: the codec's payload for every tile it
can represent, the raw codec's bytes for every tile it cannot, plus the
container framing (a 64-byte header, and a 40-byte directory entry plus a
4-byte CRC trailer per tile).

**Partial coverage is visible.** The palette codec cannot represent a tile
with more than 256 distinct pixel values. Its figure therefore includes the
raw fallback for the tiles it cannot encode, and the report flags
`covers_all_tiles: false` and the size of the fallback. A codec that cannot
encode every tile is never presented as if it had produced a file.

**Raw results are preserved.** The JSON keeps every per-tile measurement,
including the unfavourable ones: the tiles where a codec expands the data,
the tiles a codec could not encode, and the per-tile heap peaks.

**A note on the memory figures.** The per-tile `peak_encode_alloc_bytes` and
`peak_decode_alloc_bytes` recorded by the harness are **heap bytes tracked by
`vpng-research`'s own global allocator**, not the process's resident set.
They exclude the binary, the stacks and the operating system's overhead.
No process-RSS figure is claimed for the codec measurements, and no
throughput claim is made beyond the harness's own wall-clock timings.

---

## 3. Results

### 3.1 The comparison to read (the whole corpus)

The baseline encoders are skipped by default for an image above a
32-megapixel cap, because Pillow needs the whole frame in memory. The
15360×8640 photograph — the largest image and **96% of the corpus's
pixels** — is above that cap, so the corpus was re-measured with the cap
raised to 200 MP (`--max-baseline-pixels 200000000`). In that run **PNG,
WebP and JPEG XL all completed for every image**, so the headline table
covers **all nine images and every column has the same denominator**:
416,522,696 canonical bytes.

> **An honest note about a measurement that failed and then succeeded.**
> An earlier attempt at the same re-measurement failed on the largest
> photograph with Pillow's `can't allocate picture frame` — a host memory
> condition, not a property of the image. When the run was repeated with
> more free memory, the WebP encoder completed on the same frame at
> 101,462,746 bytes (0.2549×). The figure in the table is that successful
> measurement. The lesson is recorded rather than hidden: a baseline that
> failed on a 380 MB frame on a machine with ~1.3 GB of free RAM is not
> evidence that the baseline cannot handle the frame, and a study that
> quietly reports the failure as a property of the format would be wrong.

| Codec | Bytes | Ratio | Median | Worst | Images |
|---|---:|---:|---:|---:|---:|
| JPEG XL lossless (baseline) | 91,196,450 | **0.2189** | 0.2160 | 1.0827 | 9 |
| WebP lossless (baseline) | 107,261,534 | 0.2575 | 0.2549 | 0.9984 | 9 |
| CARC C (research) | 127,523,075 | 0.3062 | 0.2995 | 1.0170 | 9 |
| CARC D (research) | 130,155,579 | 0.3125 | 0.3052 | 1.0191 | 9 |
| Predictor + entropy (stable 3) | 133,500,355 | 0.3205 | 0.3120 | 1.0161 | 9 |
| CARC A (research) | 133,500,355 | 0.3205 | 0.3120 | 1.0161 | 9 |
| CARC B (research) | 139,231,075 | 0.3343 | 0.3264 | 1.0193 | 9 |
| PNG (baseline) | 143,484,520 | 0.3445 | 0.3384 | 1.0021 | 9 |
| Predictor→LZ77→entropy (research) | 181,170,505 | 0.4350 | 0.4304 | 1.0157 | 9 |
| LZ77→entropy (research) | 194,163,860 | 0.4662 | 0.4599 | 1.0150 | 9 |
| LZ77 (research) | 333,574,485 | 0.8009 | 0.7997 | 1.1732 | 9 |
| Run-length (stable 2) | 342,347,307 | 0.8219 | 0.8254 | 1.0024 | 9 |
| Predictor→LZ77 (research) | 346,839,309 | 0.8327 | 0.8286 | 1.2347 | 9 |
| Palette (stable 1, partial) | 369,128,419 | 0.8862 | 1.0002 | 1.0012 | 9 |
| Raw (stable 0) | 416,617,300 | 1.0002 | 1.0003 | 1.0012 | 9 |

The ratios are whole-corpus totals: each codec's file size summed over the
nine images, divided by the 416,522,696 canonical bytes those images
contain. The **median** and **worst** columns are the per-image ratios.

Three readings, all of them true:

* **CARC C beats PNG by 11.1%** (127,523,075 against 143,484,520) and beats
  the stable predictor codec by 4.5% (133,500,355).
* **CARC C is 18.9% worse than WebP lossless** (107,261,534).
* **JPEG XL lossless is the strongest baseline here**: 91,196,450 bytes,
  15.0% smaller than WebP lossless, and CARC C is 39.8% worse than it.
  No VPNG codec comes close to it on this corpus.

### 3.2 The same totals, without the baselines

Every baseline was measured on every image, so the JSON's `aggregate` and
`matched_aggregate` sections are **identical** in this run, and the
VPNG-only totals are the nine-image figures quoted in §3.1. They are
repeated here without the baseline rows so that the VPNG codecs can be read
on their own.

| Codec | Bytes | Ratio | Median | Worst |
|---|---:|---:|---:|---:|
| CARC C (research) | 127,523,075 | 0.3062 | 0.2995 | 1.0170 |
| CARC D (research) | 130,155,579 | 0.3125 | 0.3052 | 1.0191 |
| Predictor + entropy (stable 3) | 133,500,355 | 0.3205 | 0.3120 | 1.0161 |
| CARC A (research) | 133,500,355 | 0.3205 | 0.3120 | 1.0161 |
| CARC B (research) | 139,231,075 | 0.3343 | 0.3264 | 1.0193 |
| Predictor→LZ77→entropy (research) | 181,170,505 | 0.4350 | 0.4304 | 1.0157 |
| LZ77→entropy (research) | 194,163,860 | 0.4662 | 0.4599 | 1.0150 |
| LZ77 (research) | 333,574,485 | 0.8009 | 0.7997 | 1.1732 |
| Run-length (stable 2) | 342,347,307 | 0.8219 | 0.8254 | 1.0024 |
| Predictor→LZ77 (research) | 346,839,309 | 0.8327 | 0.8286 | 1.2347 |
| Palette (stable 1, partial) | 369,128,419 | 0.8862 | 1.0002 | 1.0012 |
| Raw (stable 0) | 416,617,300 | 1.0002 | 1.0003 | 1.0012 |

The palette codec's figure includes **346,415,560 bytes of raw fallback**:
it cannot represent a tile with more than 256 distinct pixel values, and on
this corpus it cannot encode most of the photographic and painted content.
Its `covers_all_tiles` flag is `false` in the raw results, and it is never
presented as if it had produced a file of its own.

### 3.3 By category

Each row covers the same images for every column: all the images of that
category in the corpus.

| Category | Images | CARC C | PNG | WebP | JPEG XL | Pred→LZ77→entropy | Raw |
|---|---:|---:|---:|---:|---:|---:|---:|
| photographs | 3 | 0.3017 | 0.3407 | 0.2554 | 0.2165 | 0.4320 | 1.0002 |
| digital paintings | 2 | 0.6841 | 0.7270 | 0.4987 | 0.4495 | 0.8240 | 1.0003 |
| procedural | 2 | 0.0473 | 0.0038 | 0.0010 | 0.0095 | 0.0115 | 1.0003 |
| screenshots | 1 | 0.0277 | 0.0023 | 0.0003 | 0.0006 | 0.0040 | 1.0003 |
| noise | 1 | 1.0170 | 1.0021 | 0.9984 | 1.0827 | 1.0157 | 1.0004 |

The category table is the uncomfortable part of the result and is kept in
full. On synthetic and structured content, PNG and WebP are one to two orders
of magnitude smaller than any VPNG codec: a screenshot at 0.0023 (PNG) and
0.0003 (WebP) against 0.0277 for the best VPNG candidate. On photographs, the
best VPNG candidate (CARC C, 0.3017) beats PNG (0.3407) by 11.5%, is behind
WebP (0.2554) by 18.1% and is behind JPEG XL (0.2165) by 39.4%. On paintings,
JPEG XL wins (0.4495), ahead of WebP (0.4987). The exception is noise, where
JPEG XL is the only encoder that expands the data (1.0827): on random bytes
its modular model has nothing to predict, and the lossless coding of the
model's residuals costs more than the pixels.

### 3.3b By resolution

| Resolution class | Images | CARC C | PNG | WebP | JPEG XL |
|---|---:|---:|---:|---:|---:|
| 512 | 4 | 0.1720 | 0.1343 | 0.1281 | 0.1446 |
| 1024 | 1 | 0.0277 | 0.0023 | 0.0003 | 0.0006 |
| 2048 | 3 | 0.5677 | 0.6104 | 0.3986 | 0.3537 |
| 16384 | 1 | 0.2995 | 0.3384 | 0.2548 | 0.2160 |

The 512 class is dominated by the two procedural images and the small
photograph; the 2048 class by the two paintings and the cat; the 16384 class
is the single large photograph. No resolution class is a distribution claim —
four classes over nine images is a breakdown, not a sample.

### 3.4 Worst cases

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

The standalone LZ77 codec **expands** a painting by 17% and a single tile by
23%; the predictor/LZ77 hybrid expands one tile by 25%. Those figures are in
the report because they are real: a dictionary codec with no entropy stage
pays for its token stream on data that has no repeated structure.

### 3.5 The largest image, broken out

The 15360×8640 photograph on its own. It is 96% of the corpus's pixels, so
it dominates the whole-corpus totals in §3.1; its own numbers are the
honest way to see what that dominance does. All three baselines were
measured on this frame: PNG at 134,734,803 bytes, WebP at 101,462,746 and
JPEG XL at 85,982,362.

> **This is the image whose first WebP baseline attempt failed.** The WebP
> row below is the **successful repeat** measurement, not the first one:
> the initial attempt on this frame died with Pillow's `can't allocate
> picture frame` under host memory pressure. The failure and its cause are
> recorded in §3.1's note above; the figure here is the re-run, taken with
> more free memory, on the same frame. Both facts are kept: the first
> attempt failed, and the repeat succeeded at 101,462,746 bytes.

| Codec | Bytes | Ratio |
|---|---:|---:|
| **JPEG XL lossless (baseline)** | **85,982,362** | **0.2160** |
| **WebP lossless (baseline)** | **101,462,746** | **0.2549** |
| CARC C (research) | 119,222,852 | 0.2995 |
| CARC D (research) | 121,496,780 | 0.3052 |
| Predictor + entropy (stable 3) | 124,207,779 | 0.3120 |
| CARC A (research) | 124,207,779 | 0.3120 |
| CARC B (research) | 129,937,103 | 0.3264 |
| **PNG (baseline)** | **134,734,803** | **0.3384** |
| Predictor→LZ77→entropy (research) | 171,365,202 | 0.4304 |
| LZ77→entropy (research) | 183,084,416 | 0.4599 |
| LZ77 (research) | 318,370,769 | 0.7997 |
| Run-length (stable 2) | 328,628,223 | 0.8254 |
| Predictor→LZ77 (research) | 332,446,771 | 0.8350 |
| Palette (stable 1, partial) | 353,877,666 | 0.8888 |
| Raw (stable 0) | 398,221,024 | 1.0002 |

On the corpus's largest and most representative image, CARC C is **11.5%
smaller than PNG**, **17.5% worse than WebP lossless** and **38.7% worse
than JPEG XL lossless**. This is a single image and it does not generalise;
the corpus-wide comparison in §3.1 is the one to read.

The four counts the editing report distinguishes — decoded, modified,
rewritten, preserved — do not apply to this study, and are not used here.
Every row in this table is a **whole-file** measurement: the encoder
produced a complete file, so the file's bytes *are* the result. Nothing is
"preserved" and nothing is "decoded but not rewritten"; the comparison is
between finished files. The editing report's terminology is defined in
`docs/P4_EDITING_ENGINE_REPORT.md` §3.0 and applies to tile-level edits,
not to this one.

---

## 4. The twelve questions

**Q1 — Were the experimental codecs implemented with an independent encoder and
decoder, exact reconstruction, a defined bitstream, bounded memory, no
cross-tile dependency, 8- and 16-bit support, malformed-stream handling,
golden vectors and no floating point in the bitstream path?**
**Yes, for all seven research codecs.** Each has its own `encode`/`decode`
pair behind the `TileCodec` trait. The bitstream is documented and pinned by
a golden hex vector: LZ77
`01000302010f0e100080020000010203040506070808090a1dd50602`, CARC
`0101082001040404040404048006…`, the hybrid
`01020120010101050505050501000302010f0e10008006004080789a9e9fb51d…`.
Reconstruction is exact on every corpus image (`exact: true` for every
candidate, every image, in the raw results). The payload header carries the
tile's own dimensions, so a tile never depends on another tile. Every codec
takes the format from the `EncodeContext`/`DecodeContext`, so 8-bit and
16-bit are both supported. The bitstream path is integer-only: the CARC
probability tables, the LZ77 token stream, the residual folding and the
arithmetic coder use integer arithmetic throughout; the only floating point
in the tree is in the *measurement* harness, never in a payload.

**Q2 — Do the experimental codecs stay out of the stable codec id space and out
of the production decoder?**
**Yes.** The research ids live in `0x80..=0xFF`; the stable ids 0–3 are
unchanged and 4–6 remain reserved. `get_codec` does not know about the
research ids, `IMPLEMENTED_CODEC_IDS` does not list them, and the encoder
cannot write one. A file that names one is rejected with
`ErrorKind::UnsupportedFeature`.

**Q3 — Does the benchmark measure every codec on identical pixels and identical
tile layouts?**
**Yes.** One canonicalisation, one 256×256 grid, one set of raw bytes per
image. The baselines are measured from the same raw file by
`benchmarks/corpus/baselines.py`, which is the single source of truth for
their settings (Pillow PNG `compress_level=6`; Pillow WebP lossless
`quality=100, method=4`; `cjxl -d 0` where installed). The ordinary corpus
runner (`run_corpus.py`) imports the same module, so the two cannot drift.

**Q4 — Are the raw results preserved, including unfavourable ones, and are the
JSON and CSV deliverables produced?**
**Yes.** `benchmarks/p3_5_raw_results.json` holds the per-image, per-tile,
per-category, per-resolution, aggregate and worst-case sections, plus a
matched-subset version of the aggregate and of the two grouped views
(`matched_aggregate`, `matched_per_category`, `matched_per_resolution`).
`benchmarks/p3_5_comparison.csv` holds the same numbers as a flat table with
a `scope` column (`image`, `category`, `resolution`, `matched-category`,
`matched-resolution`, `aggregate`, `matched-aggregate`, `worst_image`,
`worst_tile`). The full per-tile record
— per-tile timings and per-tile heap peaks — is kept verbatim under
`benchmarks/results/research-raw/`. Nothing is filtered: the expanding tiles,
the tiles the palette codec could not encode, and the empty baseline entries
are all present.

**Q5 — Does the report include per-image, per-category, per-tile,
per-resolution, aggregate and worst-case views?**
**Yes.** All six views are in the JSON and in the CSV, each in both an
all-images and a matched-subset form. In this run the two forms coincide
for every codec, because every baseline was measured on every image; the
matched form is kept so that a future run with a missing baseline still
reports a comparison whose denominators agree.

**Q6 — Are the baselines measured on the same pixels?**
**PNG, WebP and JPEG XL: yes, on all nine images.** The 15360×8640
photograph exceeds the default 32-megapixel baseline cap, so the corpus was
re-measured with the cap raised to 200 MP, where all three encoders
completed: PNG at 134,734,803 bytes (0.3384×), WebP at 101,462,746 bytes
(0.2549×) and JPEG XL at 85,982,362 bytes (0.2160×). Every column in
§3.1 therefore has the same denominator, 416,522,696 canonical bytes, and
no comparison in this report mixes denominators.

**Q7 — Is every reconstruction exact?**
**Yes.** Every candidate reports `exact: true` on every image and every tile
it could encode, in every one of the 9 images × 2,187 tiles measured. The
codec unit suites add 52 tests, including explicit malformed-stream
rejection and pinned golden vectors.

**Q8 — Are the improvements statistically and practically meaningful?**
**Partly.** CARC C is 4.5% smaller than the stable predictor codec on the
whole corpus (0.3062 against 0.3205) and 11.1% smaller than PNG (0.3445) —
practically meaningful. The predictor→LZ77→entropy hybrid is *worse* than the
stable predictor on the corpus (0.4350 against 0.3205) and only
wins on synthetic content (0.0115 against 0.0473 for CARC C on procedural
images). The sample is **nine images**,
not the 750-image corpus the roadmap
asks for; no statistical significance is claimed, and no distribution claim
follows from a seed corpus.

**Q9 — What are the worst cases, and does any codec expand the data?**
**Yes, two do.** The standalone LZ77 codec expands `commons-starry-night-1280`
by 17.3% and one of its tiles by 23.3%; the predictor→LZ77 hybrid expands
one tile by 24.9%. Every predictor-based codec expands the pure-noise image
by 1.5–1.9%. The selection logic never chooses a codec that expands the
tile — the candidate comparison is over *complete* payload sizes and Raw is
always a candidate — but the measurements are kept.

**Q10 — Do the existing golden files and test vectors remain byte-identical?**
**Yes.** The five golden fixtures are unchanged (17,092 / 324,668 / 780 /
396 / 2,348 bytes), the `vpng-testkit` golden test passes, and the stable
codecs' byte-level vectors are unchanged. The shared predictor internals
became `pub(crate)` so the research codecs reuse the *identical* predictor
choice; that is a visibility change only and the produced bytes are
identical.

**Q11 — Does the binary format change?**
**No.** No chunk, no field, no version, no size change. The research codecs
produce *payloads in memory*; the file format does not learn about them.

**Q12 — What is the recommendation, and what is explicitly not claimed?**
**Recommendation: do not promote any experimental codec.** CARC C is the
strongest research result and the only candidate that beats the stable
predictor codec on photographs while beating PNG as well, but it is 18.9%
*worse* than WebP lossless and 39.8% *worse* than JPEG XL lossless on the
corpus, it expands incompressible data, and it would need a format version
bump, a migration policy and compatibility tests before it could be
assigned a stable id. The stable set stays at ids 0–3. **Not claimed:** any
speed improvement (no throughput comparison is reported beyond the harness's
own timings), any statistical significance, any 750-image corpus result,
any claim that VPNG beats WebP or JPEG XL, and any claim that one encoder
setting (`cjxl -d 0`, Pillow's `quality=100`) is the best its format can do.

---

## 5. What would change the recommendation

1. **Measure JPEG XL at more effort levels.** `cjxl -d 0` uses the
   default effort. A sweep over `cjxl`'s effort and distance settings
   would show whether JPEG XL's 15.0% lead over WebP is a property of the
   format or of one configuration.
2. Run the study on the 750-image corpus the roadmap specifies; the seed
   corpus's per-category ratios are dominated by one large photograph.
3. Add an entropy stage to the dictionary path (LZ77→entropy is 0.4662, and
   the hybrid predictor→LZ77→entropy is 0.4350, against the standalone
   LZ77's 0.8009 — the entropy stage is what makes the dictionary path
   competitive).
4. Measure encode and decode throughput per codec; this study measured size,
   not speed.
5. Re-run the WebP baseline on the largest photograph on a machine with
   ample free memory. It is measured here (0.2549×), but it failed once
   under memory pressure, and a baseline that fails intermittently is not a
   baseline a product should depend on.

---

## 6. Reproducing the study

```powershell
cargo build --release --workspace
python benchmarks/corpus/run_research.py             # full measurement
python benchmarks/corpus/run_research.py --reuse-raw # re-aggregate the saved measurements
```

The runner writes `benchmarks/p3_5_raw_results.json` and
`benchmarks/p3_5_comparison.csv`. The `--reuse-raw` mode re-measures the
baselines and re-aggregates the saved per-image measurements, which is how
the tables above were produced without repeating the 12-minute measurement.
The corpus-wide baseline figures need the raised cap:

```powershell
python benchmarks/corpus/run_research.py --reuse-raw --max-baseline-pixels 200000000
```

---

## 7. Why the experimental codecs cannot reach a file

`get_codec` is the only way the decoder resolves a tile's codec id, and it
knows only ids 0–3. The research registry lives in
`vpng_codecs::experimental` and is reachable only from the research harness.
The encoder's candidate list comes from `IMPLEMENTED_CODEC_IDS`. Therefore:

* the encoder cannot write a research payload into a file;
* the decoder rejects a file that names one, with
  `ErrorKind::UnsupportedFeature`;
* the `vpng` CLI cannot produce or read such a file.

This is deliberate. A research id must not be assignable until a
specification update, a version bump, a migration policy and compatibility
tests exist for it. Nothing in this milestone changes that rule.
