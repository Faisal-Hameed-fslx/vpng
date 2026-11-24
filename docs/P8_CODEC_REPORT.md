# P8 — promoting the stable codecs 4, 5 and 6

**Date:** 2026-11-22
**Deliverables:** the format revision `docs/VPNG_SPEC_0_3.md` and
`docs/decisions/ADR-0015-promoting-codecs-4-to-6.md`; the promoted codecs in
`crates/vpng-codecs` (ids 4, 5 and 6); the codec-set policy in
`vpng_codecs::CodecSet`; the corpus runners
`benchmarks/corpus/run_codec_research.py` and
`benchmarks/corpus/run_codec_corpus.py`; and the raw result files
`benchmarks/p8_codec_corpus_results.json` and
`benchmarks/p8_codec_production_results.json`.
**Status:** the case **ran** — it did not have to be reported BLOCKED. Every
figure below is read out of the raw JSON, and `check_codecs()` in
`benchmarks/verify_reports.py` binds the quoted figures to it.

---

## 1. What this is

Step 5 of the agreed plan: **promote the stable codec ids 4–6**, using the
750-image corpus as the evidence.

Three codecs that were research code become stable:

| id | codec | what it is |
|---:|---|---|
| 4 | `lz77` | tile-local LZ77 dictionary over the tile's raw bytes |
| 5 | `predictor+dictionary` | the stable predictor's residuals, folded to unsigned bytes, then LZ77 |
| 6 | `context-entropy` | the stable predictor's residuals, priced against a gradient-conditioned probability table |

They are the pipelines ADR-0004 reserved under exactly those ids, and they
are the three the corpus measurement picked.

The format's minor version goes to **3**, and the version bump moves not a
single byte of the layout: the main header is still 64 bytes with the same
fields at the same offsets, and the directory entry is still 40 bytes with
the same fields at the same offsets.

What v0.3 changes is one *rule*:

> A directory entry may name codec id 4, 5 or 6 **only if** the file's minor
> version is 3 or newer. A v0.1 or v0.2 file that names one of them is
> malformed, and every reader rejects it with `UnsupportedFeature`.

The version states what the file **contains**, not what the encoder was
allowed to do. A writer that permits the extended codec set but ends up
choosing only the original four writes a **v0.1** file. The default encoder
path permits only the original four, so the published bytes do not move — and
§3.1 shows that they did not.

---

## 2. How it was measured

### 2.1 The host

`stable-x86_64-pc-windows-gnu`, rustc 1.100.0, an Intel Core i5-1235U with
**10 physical cores and 12 logical processors**, 7.7 GB of physical memory.
The binaries are the release profile. The corpus run used **4 encoder
workers** and **one image at a time**, because the host is memory-pressured
and an earlier two-image run died on an allocation failure. Every command
that failed was retried, and the retry count is published: **0 encodes needed
a retry** in the production run.

### 2.2 The corpus

The **same 750 images** the P5 corpus report used — photographs 300,
screenshots 100, pixel art 100, scans 100, textures 150 — read from
`benchmarks/corpus_manifest.json` and canonicalised to the same raw pixels.
The denominator is the **canonical pixel bytes**, the same number the P5
corpus report divides by, so the two reports can be read side by side.

### 2.3 Two measurements, not one

**The research harness** (`run_codec_research.py`) measures every candidate
on every image, in memory, and answers *what would each codec cost*. It never
writes a VPNG file: the research codecs have no stable id, and a file that
carried one would be malformed by definition.

**The production run** (`run_codec_corpus.py`) runs the real `vpng` binary
twice per image — `--codec-set stable` and `--codec-set extended` — and
answers *what does the shipped encoder actually write*. Each file is:

* checked with `vpng check` (the header and directory checksums);
* **verified** with `vpng verify`, which decodes every tile and compares the
  result with the original pixels — so a file that is smaller but not
  lossless fails the run rather than being reported as a win;
* digested with SHA-256, so two runs can be compared byte for byte.

The stable run is also compared against the **P5 corpus baseline**, which is
the proof that the promotion did not move the published bytes.

### 2.4 The rules

* **Every image is measured.** All 750, in both sets: 1,500 files written,
  checked, verified and digested.
* **The median.** The middle sample, or the mean of the two middle samples
  when the count is even.
* **The p95.** The **nearest-rank** percentile: the smallest sample at or
  above the 95th percentile.
* **The standard deviation.** The sample one, with n − 1 in the denominator.
* **The bytes are checked.** Every file carries its SHA-256, and the stable
  run is compared with the P5 baseline file by file.

### 2.5 What is in the memory figures

**Included:** the whole picture while it is being encoded and decoded, the
tile payloads in flight, the encoded payloads waiting to be written, the
container metadata, the binary, the stacks and the allocator's own
bookkeeping.

**Excluded:** the canonical pixel file on disk, the operating system's page
cache, the memory of other processes, and the benchmark driver's own memory.

This report makes **no memory claim** about the corpus run: the corpus is a
*size* measurement, and the driver did not sample the process's resident set
per image. The memory numbers that do exist are the ones already published —
the streaming encoder's **19,296,256 bytes** (P5.1) and the localized-edit
matrix's **12 PASS / 0 FAIL / 0 BLOCKED** with a worst case of
**13,770,752 bytes** of resident-set growth (P6/P7). Neither was re-measured
in this milestone, and §6 says so.

---

## 3. Results

### 3.1 The default path did not move — not one byte

**750 of 750 stable files are byte-identical to the P5 corpus baseline.**
Every stable file's size matches the P5 run's `auto` size, image for image.

The 16K photograph is the sharpest case: its stable file is
**124,197,035 bytes** with the SHA-256
`d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf` — **the
P5.2 baseline's tile-256 digest**, the same file the preview milestone
measured against. All 750 stable files report minor version **1**; the
golden fixtures pass unchanged.

### 3.2 The headline: −10.46% over the corpus, and never worse

| Category | n | Stable bytes | Extended bytes | Delta | Delta % | Median ratio (stable → extended) | Smaller | Larger | Equal |
|---|---:|---:|---:|---:|---:|---|---:|---:|---:|
| **overall** | **750** | **453,767,460** | **406,294,626** | **−47,472,834** | **−10.46%** | 0.415337 → **0.357749** | **643** | **0** | 107 |
| photos | 300 | 234,827,986 | 220,022,669 | −14,805,317 | −6.30% | 0.601730 → 0.552488 | 297 | 0 | 3 |
| pixel_art | 100 | 365,091 | 309,965 | −55,126 | −15.10% | 0.165392 → 0.165392 | 39 | 0 | 61 |
| scans | 100 | 67,871,353 | 55,647,808 | −12,223,545 | −18.01% | 0.346336 → 0.283930 | 100 | 0 | 0 |
| textures | 150 | 133,872,854 | 115,710,014 | −18,162,840 | −13.57% | 0.378297 → 0.346149 | 107 | 0 | 43 |
| ui | 100 | 16,830,176 | 14,604,170 | −2,226,006 | −13.23% | 0.063064 → 0.052882 | 100 | 0 | 0 |

**No image is larger under the extended set.** 643 are smaller, 107 are the
same size, and none is worse. That is the property the promotion needed: the
selector can only pick a promoted codec when its payload is strictly smaller,
so the extended selection is never worse *per tile* — and the corpus confirms
it end to end.

The 107 unchanged images are the ones where no promoted codec helped. They
are **v0.1 files**, because the version says what the file contains: the
encoder permitted the extended set and still chose only the original four.
The other 643 files are **v0.3**.

### 3.3 The candidates, measured on their own

The research harness measured each candidate over all 750 images. The
totals are whole-file figures — the payload of every tile plus the container
framing — and the median is the ratio to the canonical pixels:

| Candidate | id after promotion | Total bytes | Median ratio | p95 ratio |
|---|---:|---:|---:|---:|
| `context-entropy` | **6** | **410,048,571** | **0.403363** | 0.796793 |
| `predictor` | 3 | 459,004,556 | 0.438941 | 0.821072 |
| `pred-lz77` | **5** | 940,212,603 | 0.793161 | 1.227140 |
| `lz77` | **4** | 955,849,483 | 0.870301 | 1.191931 |
| `rle` | 2 | 935,209,171 | 0.839388 | 1.015947 |
| `palette` | 1 | 1,205,064,250 | 1.000315 | 1.008102 |
| `raw` | 0 | 1,450,772,090 | 1.000405 | 1.093426 |

**`context-entropy` is the best codec on the corpus**, and it is the one the
selector picks most: 7,094 of the 8,944 tiles in the extended run. The
dictionary codecs (ids 4 and 5) are **worse on their own** than the stable
predictor — the LZ77 dictionary's median ratio is 0.870301 against the
predictor's 0.438941 — but they are **not useless**: they win on content the
predictor handles badly, and they are chosen for 515 and 314 tiles
respectively. A UI screenshot, for example, is mostly flat rectangles and
repeating runs, which is exactly what a dictionary codes well and a
predictor codes badly.

The three promoted codecs are therefore a package: one is the general
winner, and two earn their ids on the content the winner is weakest on.

### 3.4 What the extended set chose

| Codec | Tiles chosen (extended) | Tiles chosen (stable) |
|---|---:|---:|
| `context-entropy` | 7,094 | 0 |
| `predictor` | 214 | 7,430 |
| `lz77` | 515 | 0 |
| `predictor+dictionary` | 314 | 0 |
| `rle` | 706 | 1,243 |
| `palette` | 88 | 258 |
| `raw` | 13 | 13 |

### 3.5 The cost: 3.44× the encode time

| Run | Encode time over the 750 images |
|---|---:|
| `--codec-set stable` | **122,542 ms** |
| `--codec-set extended` | **421,618 ms** |
| ratio | **3.44×** |

On the 643 images where the two sets actually differ, the ratio is 3.48×
(119,206 ms against 414,787 ms). The research harness, which measures the
candidates rather than the selector, put the same trade-off at 2.57× over its
own subset.

**This is the reason the extended set is not the default.** A 10.46% size
gain for 3.44× the encode time is a trade, not a free win, and turning it on
by default would also move every published byte and every golden fixture.
The set is available today to anyone who asks for it:
`vpng encode … --codec-set extended`.

### 3.6 The 16K photograph

| | Stable | Extended |
|---|---:|---:|
| File size | 124,197,035 B | **119,214,234 B** |
| Ratio to canonical | 0.311950 | **0.299435** |
| Tiles | 2,040 | 2,040 |
| Codecs | predictor 2,038, palette 2 | context-entropy 2,037, predictor 2, palette 1 |
| Minor version | 1 | **3** |

**−4,982,801 bytes**, the largest single win in the corpus. The extended
file's SHA-256 is
`1f65287112a8b253abd88b1d1fbb1f53e851cf67feed6e3b090aade5f7edd8d0`.

### 3.7 The size ceiling, and the files that miss it

The 10,000,000-byte ceiling: **748 of 750 under it, in both sets** — the same
two failures as the P5 corpus report:

| Image | Stable | Extended |
|---|---:|---:|
| `user-photo-15360x8640` | 124,197,035 B | 119,214,234 B |
| `threejs-textures-46fe5a0e50af` | 11,397,940 B | 10,868,574 B |

The extended set narrows both, and neither crosses the ceiling.

### 3.8 The projection and the production run agree exactly

The research harness predicted the two totals without writing a file: stable
**453,767,460** bytes and extended **406,294,626** bytes. The production
encoder wrote **453,767,460** and **406,294,626**. The projection was exact,
to the byte, over 1,500 files.

### 3.9 Determinism

**20 images were encoded twice with different worker counts** — 4 workers for
the main run, 1 worker for the check — and **all 20 digests matched**.
The full 144-case matrix over every pixel format, several sizes and several
tile geometries is in the test suite, and it now runs for the extended set
too.

### 3.10 A defect the corpus run found

The first production run **failed**: `vpng check` refused every extended file
with *"tile 0 uses codec id 6 (context-entropy), which format v0.1 does not
define"*.

The cause was a real bug, not a measurement problem. The **batch** encoder
computed the file's minor version from what the file contained, but the
**streaming writer** — the path the `vpng encode` command actually uses —
built its header itself and knew only about v0.1 and v0.2. So the streaming
writer wrote a file that named codec id 6 and claimed to be a v0.1 file: a
malformed file that every reader rejects.

The rule now lives in **one** function (`vpng_encoder::content_minor`) that
both paths call, and the test suite pins the agreement over 144 encode cases
per codec set. The corpus run is what found it; the unit tests had not,
because they only ever exercised the default codec set.

---

## 4. The twelve questions

**Q1 — Was the corpus actually run, or blocked?**
**Run, not blocked.** All **750** images were encoded twice with the
production encoder, checked, verified and digested, and all **750** were also
measured by the research harness. The raw files are
`benchmarks/p8_codec_production_results.json` and
`benchmarks/p8_codec_corpus_results.json`. The run took 738.799 s for the
production pass and 699.454 s for the research pass.

**Q2 — How many times was each case run, and what do the percentiles mean?**
**Once per image per set, and the corpus is the repetition.** 750 images ×
2 sets = 1,500 files, every one of them checked, verified and digested. The
median is the middle sample, or the mean of the two middle samples when the
count is even. The p95 is the **nearest-rank** percentile: the smallest
sample at or above the 95th percentile. The standard deviation is the sample
one, with n − 1 in the denominator. The rules are in the raw JSON and in the
unit tests of `photo_bench.rs`.

**Q3 — What is the baseline, and was it measured in the same run?**
**The P5 corpus baseline, and it is the same file.** The stable run's 750
files match the P5 run's 750 files byte for byte, and the 16K photograph's
stable digest is the P5.2 tile-256 digest
`d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf`. The
comparison was made **in the same run**, file by file, and the result is in
the raw JSON as `stable_matches_p5`.

**Q4 — Do the promoted codecs change the pixels?**
**No.** `vpng verify` decodes every tile and compares the result with the
original pixels: **0 verify failures** across all 1,500 files. Every file
also passed the container's own integrity check: **0 check failures**.

**Q5 — What does the promotion buy?**
**−47,472,834 bytes, −10.46%, over the whole corpus, and never a byte worse
on any image.** 643 images smaller, 107 the same, 0 larger. The median ratio
to the canonical pixels goes from 0.415337 to 0.357749. Every category gains:
scans −18.01%, pixel art −15.10%, textures −13.57%, UI −13.23%, photos
−6.30%.

**Q6 — Which codec did the work?**
**`context-entropy` (id 6), and it is the best codec on the corpus.** It was
chosen for 7,094 of the 8,944 tiles. On its own it is the best candidate
(410,048,571 bytes over the corpus, a median ratio of 0.403363, against the
stable predictor's 459,004,556 and 0.438941). The two dictionary codecs
(ids 4 and 5) are worse on their own but win on content the predictor handles
badly — they were chosen for 515 and 314 tiles, mostly in the UI category.

**Q7 — What does the promotion cost?**
**3.44× the encode time**: 421,618 ms against 122,542 ms over the 750
images, 3.48× over the 643 images where the sets differ. The extended set
therefore stays **opt-in** (`--codec-set extended`), because a 10% size gain
for a 3.4× encode-time cost is a trade, and because turning it on by default
would move every published byte and every golden fixture. The decode cost
was not measured separately in this milestone — §6 says so.

**Q8 — Does the promotion move the default path's bytes?**
**No. Not one byte.** 750 of 750 stable files match the P5 corpus baseline,
all 750 report minor version 1, and the 16K photograph's digest is the P5.2
baseline's. The version is **content-determined**: a file that ends up using
only the original four codecs is a v0.1 file even when the extended set was
permitted — which is exactly what happened to the 107 unchanged images.

**Q9 — Is the format still binary-compatible?**
**Yes.** The 64-byte header and the 40-byte directory entry keep their exact
layouts; the minor version is the only thing that changes. A v0.1 file is
still read exactly as before, the golden fixtures pass unchanged, and a v0.3
file is a v0.1 file with three more codec ids and a different version byte.
A v0.1 or v0.2 file that names id 4, 5 or 6 is rejected with
`UnsupportedFeature` — the reader never attempts a best-effort decode.

**Q10 — Are the research codecs still out of reach?**
**Yes.** The research ids `0x80`, `0x81`, `0x83`, `0x92` and `0x93` are not
in the implemented set, the production encoder cannot reference them, and the
decoder rejects any file that names one with `UnsupportedFeature`. The test
suite checks the whole `0x80..=0xFF` range.

**Q11 — What is in the memory figures, and does this report make a memory
claim?**
**It makes no memory claim of its own.** The corpus run measured file sizes,
not memory, and the driver did not sample the process's resident set per
image; §2.5 states what would have been included and what would have been
excluded had it done so. The memory numbers that exist are the ones already
published and are not re-measured here: the streaming encoder's
**19,296,256 bytes** (P5.1) and the localized-edit matrix's **12 PASS / 0
FAIL / 0 BLOCKED**, worst case **13,770,752 bytes** of resident-set growth
against the 67,108,864-byte budget (P6/P7). Neither the streaming writer's
bounded-memory property nor the editing engine's was changed by this
milestone: the promoted codecs add a bounded model state per tile (17 KiB
for 8-bit samples, 34 KiB for 16-bit) and a bounded dictionary (at most
1 MiB), both independent of the tile size.

**Q12 — What is not measured, and what remains untested?**
The limits are in §6. The four that matter most: the **decode** cost of the
promoted codecs was not measured in the production run, only in the research
harness (where `context-entropy` decoded the corpus in 88.947 s against the
predictor's 61.219 s); the **encode-time cost** was measured as a total over
the corpus, not as a distribution per image; the **default stays the stable
set**, so the "should the default change?" question is answered with a
measurement of the trade-off and not with a change; and the corpus run
measured the **file size**, not the latency of viewing a file that uses the
promoted codecs.

---

## 5. Reproducing the evidence

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# One: the candidates, measured on their own over the whole corpus.
cargo build --release --workspace
python benchmarks/corpus/run_codec_research.py --jobs 1 --reuse-raw `
    --output benchmarks/p8_codec_corpus_results.json

# Two: the production encoder, over the same corpus, with real files that
# are checked, verified and digested.
python benchmarks/corpus/run_codec_corpus.py --jobs 1 --workers 4 `
    --determinism-samples 20 `
    --output benchmarks/p8_codec_production_results.json

# Three: the tests, and the check that every quoted figure matches the
# raw data.
cargo test --workspace --all-targets
python benchmarks/verify_reports.py
```

---

## 6. What this does not claim

* **No claim that the default changed.** The default codec set is still the
  original four, and the published bytes did not move. The extended set is
  opt-in, and §3.5 gives the price of turning it on.
* **No claim that VPNG now beats PNG.** The P5 corpus report said VPNG was
  behind PNG on every category, and this milestone does not re-measure the
  baselines. The 10.46% is against VPNG's own stable set, not against PNG,
  WebP or JPEG XL.
* **No decode-latency claim.** The promoted codecs' decode cost was measured
  only in the research harness, on payloads rather than on files.
* **No memory claim for the corpus run.** §2.5 and Q11 say what would have
  been included and what was not measured.
* **No claim that every image improves.** 107 images are unchanged; the
  claim is that **none** is worse.
* **No claim about other platforms.** Windows on x86-64.
* **No claim that the format is frozen.** v0.3 is a revision; the release
  plan still has the metadata chunks, the ecosystem work and the v1.0 freeze
  ahead of it.

---

## 7. The acceptance gates

| # | Gate | Result | Evidence |
|---|---|---|---|
| 1 | The corpus actually ran, at its full size | **PASS** | 750 of 750 images, two sets, 1,500 files; `benchmarks/p8_codec_production_results.json` |
| 2 | Every file is lossless | **PASS** | `vpng verify` on all 1,500 files: **0 failures** |
| 3 | Every file passes the container's own check | **PASS** | `vpng check` on all 1,500 files: **0 failures** |
| 4 | The default path's bytes do not move | **PASS** | 750 of 750 stable files match the P5 baseline byte for byte; the 16K digest is the P5.2 baseline's; the golden fixtures pass unchanged |
| 5 | The selection never loses to the stable set | **PASS** | 643 smaller, 107 equal, **0 larger** |
| 6 | The promoted codecs are reachable from the file | **PASS** | 643 of the 750 extended files are v0.3 files that name id 4, 5 or 6 and decode exactly |
| 7 | An older version may not name a promoted codec | **PASS** | `v3_formats.rs`: a v0.1 or v0.2 file that names id 4, 5 or 6 is rejected with `UnsupportedFeature` |
| 8 | The research ids can never reach a file | **PASS** | `v3_formats.rs` checks the whole `0x80..=0xFF` range; the production encoder cannot reference them |
| 9 | The version says what the file contains | **PASS** | the 107 files that chose only the original four are v0.1 files; the 643 that chose a promoted codec are v0.3 |
| 10 | The worker count does not change the bytes | **PASS** | 20 images re-encoded with 1 worker instead of 4: 20 of 20 digests match; the 144-case matrix runs for the extended set too |
| 11 | Every quoted figure is bound to the raw data | **PASS** | `check_codecs()` in `benchmarks/verify_reports.py` |
| 12 | The binary layout is unchanged | **PASS** | `docs/VPNG_SPEC_0_3.md`: the 64-byte header and the 40-byte entry keep their exact layouts; only the minor version and the codec-id rule change |

---

## 8. What is next

The codecs are in, measured and documented. What they leave open:

* **The encode-time cost.** 3.44× is a lot to pay for 10%. The next question
  is whether the selector can *predict* which promoted codec will win, so it
  only tries the promising one — the measurement in §3.3 says `context-entropy`
  wins 79% of the tiles, and the predictor wins most of the rest.
* **The decode latency.** The promoted codecs are measured for size, not for
  how fast a viewer can decode them. `vpng-view` has to be measured over
  files that actually use them.
* **The default.** The evidence is in §3.5 and the decision is recorded in
  ADR-0015: the extended set stays opt-in. If the encode cost comes down,
  the decision is revisited with the same corpus and the same rules.
* **The rest of the plan.** The ecosystem phase — Python, WASM, HTTP Range,
  layers, GPU payloads and the UI — then the v0.2 → v0.23 release cadence,
  and the v1.0 freeze last.
