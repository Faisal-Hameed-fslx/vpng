# P5.3 — The 750-image corpus pipeline

**Date:** 2026-11-21
**Deliverable:** the builder `benchmarks/corpus/build_corpus.py`, the source
registry `benchmarks/corpus/sources.py`, the extended runner
`benchmarks/corpus/run_corpus.py`, the manifest
`benchmarks/corpus_manifest.json` and the raw result file
`benchmarks/p5_corpus_results.json`.
**Status:** the corpus **exists** — 750 images, every category filled to its
target — and the per-category distribution and the 10 MB success rate are
**measured**, not projected. Every figure below is read out of the raw JSON,
and `benchmarks/verify_reports.py` checks the quoted figures against it.

---

## 1. What this is

Phase A, Step 3 of the agreed plan: the **750-image corpus**.

Until now this repository had a **nine-image seed**. Every compression
figure in the history of the project — the codec study, the stress runs, the
streaming benchmarks — rests on those nine images, and §7 of
`docs/ROADMAP_STATUS.md` says so plainly: *no distribution claim, no
per-resolution distribution, no 10 MB success rate*.

This milestone replaces the seed with a real corpus, and it does three things
that a list of filenames does not do:

* **It fixes the bytes.** Every image is recorded by the SHA-256 of its
  canonical pixels and by the SHA-256 of the file it came from, together with
  the repository, the ref and the **commit** it was read at. A measurement
  can be tied to bytes, not to a filename.
* **It makes the corpus reproducible.** The builder takes the same images in
  the same order from the same commits, and the manifest records what it
  skipped and why. A second run over the same commits produces the same
  digests.
* **It measures the distribution the roadmap asked for.** The per-category
  size distribution, the per-resolution distribution and the share of images
  that land under the 10,000,000-byte ceiling are now measured on 750 images
  instead of asserted from nine.

The corpus is the input to everything that follows: promoting the reserved
codec ids 4–6, re-deriving the auto-selection thresholds (v0.8 and v0.20),
and evaluating the preview pyramid and adaptive tiling against a real
baseline rather than a seed.

**The headline finding is unfavourable, and it is the reason this report
exists.** On this corpus the `auto` codec writes **0.415×** the canonical
bytes; PNG writes **0.379×**, WebP lossless **0.242×** and JPEG XL lossless
**0.219×**. VPNG is behind PNG on **every one of the five categories**, and
behind WebP lossless by a wide margin. The corpus does what the roadmap
asked it to do: it turns "we compress well" into a number that can be
compared.

## 2. How the corpus was built

### 2.1 The rule: a corpus is a manifest of digests

The image bytes are **not** committed. They live under the git-ignored
`corpus-data/` tree, and what is committed is `benchmarks/corpus_manifest.json`
— the provenance record. Every entry carries:

| Field | What it fixes |
|---|---|
| `sha256` | the canonical pixels (the bytes every measurement is on) |
| `source_sha256` | the downloaded file's bytes |
| `source`, `source_id`, `source_path`, `source_ref`, `source_commit` | exactly where the file came from |
| `width`, `height`, `pixel_format`, `canonical_bytes` | the shape of the canonical buffer |
| `license`, `attribution` | what the image is and who to credit |
| `distinct_colors`, `tags`, `synthetic` | the facts the category rule was applied to |

Nothing is upscaled: the recorded dimensions are the native dimensions of the
file, and the builder rejects anything over 50 megapixels or 25 MB.

### 2.2 The sources

| Source | Repository | Commit | Category | Images |
|---|---|---|---|---:|
| imagenet-sample-images | `EliSchwartz/imagenet-sample-images` | `bc69043e2862` | photos | 297 |
| github-docs-images | `github/docs` | `d7487f5debb5` | ui | 99 |
| phaser-example-sprites | `photonstorm/phaser-examples` | `2e80150376e6` | pixel_art | 100 |
| opencv-calibration-photos | `opencv/opencv_extra` | `ea74ac270b50` | scans | 98 |
| threejs-textures | `mrdoob/three.js` | `0245b6dad700` | textures | 147 |
| seed (local) | the repository's own seed corpus | - | mixed | 9 |

The nine seed entries are the images this project has measured all along,
including the 15,360 × 8,640 photograph; they are carried forward with their
original provenance and their original digests rather than dropped.

**Licensing.** Each source keeps its own licence, and each entry records it:
the ImageNet sample images carry the ImageNet terms, the GitHub and VS Code
documentation images carry their upstream terms, and the OpenCV, Phaser,
three.js and Godot assets carry their repositories' terms. **Nothing is
redistributed**: the bytes stay in the git-ignored corpus tree, and only the
digests and the provenance are committed. The images are used as source
material for a local benchmark.

**The GitHub token is never written anywhere.** The builder reads it from
`$GITHUB_TOKEN` when one is present and falls back to the unauthenticated
API; the manifest records the commit, never the credential.

### 2.3 The categories, and the test behind the pixel-art one

The five categories are the plan's: **photos**, **ui**, **pixel art**,
**scans**, **textures**. Four of them are a property of where the image came
from, and the builder records the source for each image, so the assignment is
traceable rather than judged.

**Pixel art is not a guess.** An image from a sprite repository is accepted
into the pixel-art category only when it passes a measurable test: its long
edge is at most 512 pixels **and** it holds at most 64 distinct colours
(alpha excluded). The colour count is recorded per image. The test rejected
**49** candidates that the source directory contained, and those rejections
are counted in the build report rather than hidden.

### 2.4 The rules

* **The median.** The middle sample, or the mean of the two middle samples
  when the count is even.
* **The p95.** The **nearest-rank** percentile: the smallest sample at or
  above the 95th percentile. With ten samples it is the largest of the ten.
* **The standard deviation.** The sample one, with n − 1 in the denominator.
  These three rules are the ones the Rust benchmark harness uses, and they are
  pinned by `python benchmarks/corpus/run_corpus.py --self-test`, which CI
  runs.
* **Every image is checked against the manifest.** The runner re-canonicalises
  every image and compares the digest with the manifest's. A mismatch is
  reported, not averaged in.
* **Every file is checked after it is written.** Each encode is followed by
  `vpng check`, which verifies the container's own structural integrity.
* **The 10 MB ceiling is decimal.** 10,000,000 bytes, per ADR-0006.
* **The selection is deterministic.** Within a source, candidates are ordered
  by the SHA-256 of their path, so the same commit yields the same images in
  the same order.

### 2.5 What the build skipped, and why

The builder processed **817** candidate files and kept **750** of them. The
skips are counted in `corpus-data/build_report.json` and in the manifest:

| Reason | Count |
|---|---:|
| `not_pixel_art` — the image failed the size-and-colour test | 49 |
| `undecodable` — Pillow cannot decode the file, or its mode is not one of the four canonical formats | 23 |
| `duplicate` — the canonical pixels were already in the corpus | 4 |

Nothing was skipped silently, and nothing was counted twice. The build took
439.629 seconds and made 131 GitHub requests; the image bytes it kept occupy
1,450,363,330 canonical pixels' worth of data before compression.

## 3. The results

### 3.1 The corpus

| Category | Images | Real | Synthetic | Median pixels | Median long edge |
|---|---:|---:|---:|---:|---:|
| photos | 300 | 300 | 0 | 562,500 | 500 |
| pixel_art | 100 | 100 | 0 | 9,124 | 61 |
| scans | 100 | 100 | 0 | 1,232,640 | 856 |
| textures | 150 | 147 | 3 | 262,144 | 384 |
| ui | 100 | 99 | 1 | 2,126,172 | 1,550 |
| **total** | **750** | **746** | **4** | | |

### 3.2 What each codec does, per category

Ratios are encoded size divided by canonical pixel bytes, on the same pixels.
A codec's column is over the images it **applies** to: the palette codec
refuses any image with more than 256 distinct pixel values, so its column is
over the 194 images it could encode and not over all 750.

| Category | Images | auto | raw | palette | rle | predictor | PNG | WebP | JPEG XL |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| photos | 300 | 0.602 | 1.000 | 1.006 | 0.971 | 0.602 | 0.591 | 0.429 | 0.387 |
| pixel_art | 100 | 0.165 | 1.012 | 0.201 | 0.429 | 0.266 | 0.140 | 0.065 | 0.067 |
| scans | 100 | 0.346 | 1.000 | 1.004 | 0.798 | 0.363 | 0.277 | 0.176 | 0.137 |
| textures | 150 | 0.378 | 1.001 | 1.004 | 0.849 | 0.387 | 0.367 | 0.246 | 0.228 |
| ui | 100 | 0.063 | 1.000 | 0.342 | 0.069 | 0.067 | 0.031 | 0.012 | 0.019 |
| **all** | 750 | 0.415 | 1.000 | 0.336 | 0.839 | 0.439 | 0.379 | 0.242 | 0.219 |

**Read this table as the verdict it is.** VPNG's `auto` is behind PNG on every
category — 0.602 against 0.591 on photographs, 0.063 against 0.031 on
screenshots — and WebP lossless is ahead of both by a factor of about 1.7.
The `raw` column is the control: it is 1.000 because the raw codec stores the
canonical bytes and adds the container.

### 3.3 The size distribution

The auto codec's output size, in bytes. The p95 is the **nearest-rank**
percentile: the smallest sample at or above the 95th percentile.

| Category | Images | Median | p95 | Largest | Median ratio | p95 ratio |
|---|---:|---:|---:|---:|---:|---:|
| photos | 300 | 327,184 | 664,931 | 124,197,035 | 0.602 | 0.801 |
| pixel_art | 100 | 1,172 | 13,705 | 26,853 | 0.165 | 0.430 |
| scans | 100 | 591,466 | 1,375,693 | 3,111,079 | 0.346 | 0.553 |
| textures | 150 | 76,733 | 5,343,199 | 11,397,940 | 0.378 | 3.250 |
| ui | 100 | 129,610 | 449,272 | 908,244 | 0.063 | 0.169 |
| **all** | 750 | 261,057 | 1,238,363 | 124,197,035 | 0.415 | 0.801 |

The textures' p95 ratio of 3.250 is not a compression failure: it is the
**container overhead** on images of a few pixels. Eighteen images in the
corpus have a canonical size so small that the file is larger than the
pixels — the worst is a 1 × 1 texture at 37.0× — and they are in the corpus
because the corpus records what the format does, not only what it does well.

### 3.4 The 10 MB target

The ceiling is 10,000,000 bytes (decimal, ADR-0006).

| Category | Images | Under the ceiling | Share | Median | p95 |
|---|---:|---:|---:|---:|---:|
| photos | 300 | 299 | 99.7% | 327,184 | 664,931 |
| pixel_art | 100 | 100 | 100.0% | 1,172 | 13,705 |
| scans | 100 | 100 | 100.0% | 591,466 | 1,375,693 |
| textures | 150 | 149 | 99.3% | 76,733 | 5,343,199 |
| ui | 100 | 100 | 100.0% | 129,610 | 449,272 |
| **all** | 750 | 748 | 99.7% | 261,057 | 1,238,363 |

**748 of 750 images fit under 10,000,000 bytes (99.7%).** The two that do
not are named, not averaged away:

| Image | Category | Pixels | Canonical | auto | Ratio |
|---|---|---:|---:|---:|---:|
| `user-photo-15360x8640` | photos | 15,360 × 8,640 | 398,131,200 | 124,197,035 | 0.312 |
| `threejs-textures-46fe5a0e50af` | textures | 2,048 × 2,048 | 12,582,912 | 11,397,940 | 0.906 |

This is the distribution the roadmap asked for, and it is not a promise: the
10 MB target is a *measured success rate on this corpus*, and the encoder can
also enforce the ceiling and refuse honestly when it cannot be met
(`--max-size`, exit code 6).

### 3.5 The distribution by resolution

| Long edge | Images | auto (median) | PNG (median) | WebP (median) | Under 10 MB |
|---|---:|---:|---:|---:|---:|
| <=512 | 461 | 0.524 | 0.507 | 0.376 | 461/461 |
| 513-1024 | 131 | 0.407 | 0.332 | 0.218 | 131/131 |
| 1025-2048 | 134 | 0.176 | 0.097 | 0.055 | 133/134 |
| 2049-4096 | 21 | 0.049 | 0.028 | 0.012 | 21/21 |
| 4097-8192 | 2 | 0.396 | 0.366 | 0.245 | 2/2 |
| >8192 | 1 | 0.312 | - | - | 0/1 |

The ratio *improves* with resolution in the middle of the table, because the
per-file overhead is amortised over more pixels — and the one image above
8192 is the corpus's 16K photograph, for which the baseline encoders were
skipped (it is 132.7 megapixels, above the 32-megapixel baseline cap).

### 3.6 The worst cases

| Image | Category | Pixels | Canonical | auto | Ratio | PNG |
|---|---|---:|---:|---:|---:|---:|
| user-photo-15360x8640 | photos | 15,360x8,640 | 398,131,200 | 124,197,035 | 0.312 | - |
| threejs-textures-46fe5a0e50af | textures | 2,048x2,048 | 12,582,912 | 11,397,940 | 0.906 | 11,391,116 |
| threejs-textures-2b0d9924da39 | textures | 4,096x2,048 | 25,165,824 | 9,352,555 | 0.372 | 8,783,859 |
| threejs-textures-755cc8af847e | textures | 8,192x1,024 | 25,165,824 | 9,065,712 | 0.360 | 8,461,054 |
| threejs-textures-09a9aaad82bf | textures | 4,096x2,048 | 25,165,824 | 8,320,876 | 0.331 | 5,723,888 |
| imagenet-sample-images-3220ed00549f | photos | 2,400x1,600 | 11,520,000 | 5,930,446 | 0.515 | 5,547,190 |
| threejs-textures-336e3e0d664b | textures | 2,048x2,048 | 12,582,912 | 5,651,729 | 0.449 | 5,482,723 |
| threejs-textures-9eea7532fcc7 | textures | 2,048x2,048 | 12,582,912 | 5,492,168 | 0.436 | 5,151,318 |
| threejs-textures-679ff8af23cc | textures | 8,192x512 | 12,582,912 | 5,431,903 | 0.432 | 4,986,526 |
| threejs-textures-62325d8e4c19 | textures | 2,048x2,048 | 12,582,912 | 5,343,199 | 0.425 | 5,043,970 |

The corpus holds 1,450,363,330 canonical bytes; the auto codec wrote
453,767,460 bytes of files for them, a ratio of 0.313.

## 4. The twelve questions

**Q1 — How many images are in the corpus, and are they real?**
**750, and 746 of them are real images.** Photographs 300, screenshots 100,
pixel art 100, scans 100, textures 150. The four non-real images are the
project's own deterministic synthetic seed images (a checker, a gradient,
a noise field and a drawn UI), and the manifest marks every one of them
`synthetic`. No image is upscaled: the dimensions are the file's native
dimensions.

**Q2 — Where did the images come from, and what are the licences?**
From five public repositories, each pinned to a commit (§2.2), plus the
project's own nine-image seed. Each source's licence and attribution are
recorded per image in the manifest — the ImageNet terms for the photographs,
the upstream terms for the GitHub, VS Code, OpenCV, Phaser, three.js and Godot
assets. **No image is redistributed**: the bytes live under the git-ignored
`corpus-data/` tree and only the digests and provenance are committed. The
GitHub token is read from the environment and is never written to the
manifest.

**Q3 — What does the SHA-256 manifest prove?**
That the measurements were taken on known bytes. Every one of the 750 images
was re-canonicalised by the runner and compared with the manifest's digest:
**0 mismatches**. Every image also records the digest of the file it came
from and the commit it came from, so a reader can fetch those exact bytes and
get the same pixels. The digest is the proof, not an assertion, and every
digest is in `benchmarks/corpus_manifest.json`.

**Q4 — Is the corpus reproducible?**
**Yes, to the commit.** The builder resolves each ref to a commit, orders the
candidates by the SHA-256 of their path, takes them until the category's
target is met, and records what it skipped and why. Re-running it over the
same commits produces the same images in the same order with the same
digests. It does **not** claim that the upstream repositories have not moved:
the commits are recorded precisely so that a rebuild can pin them.

**Q5 — What does the size distribution look like?**
Per category, in §3.3. The median `auto` output is **327,184 bytes** for a
photograph, **591,466** for a scan, **129,610** for a screenshot, **76,733**
for a texture and **1,172** for a pixel-art sprite; the p95 across the whole
corpus is **1,238,363 bytes**. The medians and the p95s use the rules in
§2.4, and the p95 is the nearest-rank one.

**Q6 — How do the codecs compare, and against which baselines?**
**VPNG is behind.** Across the corpus, `auto` writes **0.415×** the canonical
bytes, PNG writes **0.379×**, WebP lossless **0.242×** and JPEG XL lossless
**0.219×**. The gap is in every category: photographs 0.602 against PNG's
0.591, screenshots 0.063 against 0.031, scans 0.346 against 0.277, pixel art
0.165 against 0.140, textures 0.378 against 0.367. The baseline figures come
from the **same canonical pixels** — Pillow's PNG at `compress_level=6`,
Pillow's lossless WebP at `quality=100, method=4`, and `cjxl -d 0` — so the
comparison has one denominator and no normalisation argument.

**Q7 — What is the 10 MB success rate?**
**748 of 750 images (99.7%)** land under the decimal 10,000,000-byte ceiling:
photographs 299/300 (99.7%), screenshots 100/100 (100.0%), pixel art
100/100 (100.0%), scans 100/100 (100.0%), textures 149/150 (99.3%). The two
failures are the 16K photograph at **124,197,035** bytes and one 2,048 ×
2,048 texture at **11,397,940** bytes. Both are named in §3.4, and the
encoder can enforce the ceiling and refuse honestly when it cannot be met.

**Q8 — How does the distribution change with resolution?**
It improves with size in the middle of the range — 0.524 at ≤512 pixels,
0.407 at 513–1024, 0.176 at 1025–2048, 0.049 at 2049–4096 — because the
per-file overhead is amortised over more pixels. It then gets worse again at
the top end, where the content itself is the problem rather than the
overhead: the single image above 8192 pixels is the 16K photograph at
**0.312×**, and the one texture that misses 10 MB reaches **0.906×**. The
corpus is mostly small (461 of 750 images are 512 pixels or less), so the
tail is thin and is reported as a tail.

**Q9 — What are the worst cases?**
§3.6 lists the ten largest files. The worst compression is the
`threejs-textures-46fe5a0e50af` texture at **0.906×**, which is also one of
the two images over the 10 MB ceiling. The worst *expansion* is not in that
table at all: eighteen tiny images have a file larger than their pixels, the
worst being a 1 × 1 texture at **37.0×**, because the container's fixed
overhead is larger than the image. That is a property of the format, and the
corpus records it instead of hiding it.

**Q10 — Does `auto` ever lose to a forced codec?**
**No, not once, in 750 images.** `auto` was never larger than the best of the
forced codecs on the same pixels, and it was **strictly smaller on 109
images**, saving **1,541,810 bytes** in total. It mixes codecs per tile, so
it can beat every single-codec run — that is the property the codec study
claimed on nine images, now measured on 750.

The palette codec is the interesting case: it **refused 556 of the 750
images**, because a tile with more than 256 distinct pixel values cannot be
palette-coded. That is a typed refusal (`unsupported feature: palette codec
cannot encode this tile`), not a silent fallback, and the `auto` path simply
picks another codec for that tile. The palette column in §3.2 is therefore
over the 194 images it applies to.

**Q11 — What is in the memory figures, and does this report make a memory
claim?**
**This report makes no memory claim at all.** It measures bytes, not memory.
The runner holds one image's canonical pixels at a time, because Pillow's
baseline encoders need the whole buffer; that is the driver's memory, not
the format's, and it is not a bound on anything. The *included* scope for
the figures that do exist here is the file bytes, the canonical byte count
and the encoded size. The *excluded* scope is the driver's working set, the
operating system's page cache, the other processes on the host and the
container overhead that the ratios do not subtract. The encoder's own
working-set bound is measured separately, in `docs/P5_ENCODER_REPORT.md`:
**19,296,256 bytes** of resident set on a 1 GiB image, against the 64 MB
target.

**Q12 — What is not measured, and what remains untested?**
The limits are in §6. The four that matter most: the corpus is **five
repositories chosen for coverage**, not a random sample of the world's
images; the category names are **proxies** — a "scan" here is often a
photographed calibration pattern rather than a flatbed scan; **four** images
are synthetic and **only three** images are larger than 4096 pixels, so the
large-image tail is two images, not a distribution; and the baseline encoders
were measured with **one setting each**, which is a fair comparison of *these
tools* and not a claim about the best each format can do.

## 5. Reproducing the evidence

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$env:GITHUB_TOKEN = "<optional; raises the API rate limit>"

# One: build the corpus. The builder resolves every ref to a commit,
# downloads the candidates, canonicalises them and writes the manifest.
# A second run over the same commits is a no-op.
python benchmarks/corpus/build_corpus.py --root corpus-data `
    --manifest benchmarks/corpus_manifest.json

# Two: measure it. Five codecs per image, plus PNG, WebP lossless and
# JPEG XL lossless on the same canonical pixels. About sixteen minutes
# with four images in flight.
cargo build --workspace --release
python benchmarks/corpus/run_corpus.py --root corpus-data `
    --manifest benchmarks/corpus_manifest.json `
    --jobs 4 --workers 4 --output benchmarks/p5_corpus_results.json

# Three: the tables and the check that every quoted figure matches the
# raw data.
python benchmarks/render_corpus_tables.py
python benchmarks/corpus/run_corpus.py --self-test
python benchmarks/verify_reports.py
```

The image bytes are not in the repository: `corpus-data/` is git-ignored, and
the manifest is the committed record of what was measured.

## 6. What this does not claim

* **No claim that the corpus is representative.** It is five repositories
  chosen for coverage of five kinds of content, plus the project's own seed.
  It is not a random sample, and the category names are proxies for content
  types rather than ground truth.
* **No claim that VPNG compresses well.** The measurements say the opposite
  of what the project would like: `auto` is behind PNG on every category and
  behind WebP lossless by a wide margin. The corpus exists to make that
  visible.
* **No claim about the largest images.** Three images are larger than 4096
  pixels, and the largest is one photograph. The p95 over 750 images is a
  p95 over a corpus that is mostly small.
* **No claim about other codecs' settings.** PNG is Pillow at
  `compress_level=6`, WebP is Pillow lossless at `quality=100, method=4`, and
  JPEG XL is `cjxl -d 0`. These are the same settings the P3.5 study used, so
  the two studies share a denominator; they are not each format's best
  possible output.
* **No claim about the baseline encoders on the 16K photograph.** It is
  132.7 megapixels, above the 32-megapixel baseline cap, so it was encoded by
  VPNG only. The `-` in §3.5 and §3.6 means *not measured*, not *failed*.
* **No memory claim.** See Q11.
* **No timing claim.** The corpus measures bytes. The timings are P5.2's
  job, with its own repetition rule.
* **No claim that the corpus is the final one.** Adding images is a matter
  of adding a source to `benchmarks/corpus/sources.py`; every figure in this
  report would then be re-derived, not patched.

## 7. The acceptance gates

| # | Gate | Result | Evidence |
|---|---|---|---|
| 1 | The corpus holds 750 images in the five planned categories | **PASS** | 300 photos, 100 screenshots, 100 pixel art, 100 scans, 150 textures |
| 2 | Every image is identified by a SHA-256 manifest | **PASS** | `benchmarks/corpus_manifest.json`, version 2: a canonical digest and a source digest per entry |
| 3 | Provenance and licence are recorded per entry | **PASS** | repository, ref, commit, path, licence and attribution on every entry |
| 4 | The selection is deterministic | **PASS** | candidates are ordered by the SHA-256 of their path; the skips are counted (49 + 23 + 4) |
| 5 | The corpus is reproducible from the recorded commits | **PASS** | the builder pins each ref to a commit and re-derives the same digests; `corpus-data/build_report.json` records what it did |
| 6 | The per-category distribution is measured | **PASS** | §3.2, §3.3 and §3.5, with medians and nearest-rank p95s |
| 7 | The 10 MB success rate is measured per category | **PASS** | §3.4: 748 of 750 (99.7%), with both failures named |
| 8 | Every image's bytes are verified against the manifest | **PASS** | 0 digest mismatches in 750 re-canonicalisations |
| 9 | Every file is verified by the container's own check | **PASS** | 0 non-zero `vpng check` exits across the run |
| 10 | The baselines share the denominator | **PASS** | PNG, WebP and JPEG XL are measured on the same canonical pixels, on 749 of 750 images; the one exception is named |
| 11 | The encoder's memory bound | **NOT CLAIMED** | this report measures bytes; the bound is P5.1's, measured separately |
| 12 | The corpus represents the world's images | **NOT CLAIMED** | five repositories chosen for coverage; §6 says what that is and is not |

## 8. What this corpus is for

**Promoting the reserved codec ids 4–6.** The plan's next step is to
implement LZ77, predictor-plus-dictionary and context-adaptive entropy as
stable ids 4, 5 and 6. They now have a real corpus to be measured on: 750
images, five content categories, and three lossless baselines with one
denominator. A codec that wins on nine seed images and loses on 750 will be
caught here rather than in a user's file.

**Re-deriving the auto-selection thresholds (v0.8 and v0.20).** The encoder
picks a codec per tile by a cost model. That model was fitted on the seed.
§3.2 is the table it has to beat, and the per-category medians are the
signal it has to separate.

**P6 and P7.** The preview pyramid and adaptive tiling change the container.
They will be evaluated against this corpus and against the photographic
latency baseline in `docs/P5_BENCH_REPORT.md`, not against a seed.

**A baseline that can be beaten.** The most useful number in this report is
the unfavourable one: **0.415×** against PNG's **0.379×** and WebP's
**0.242×**. A project that knows the size of its own gap can close it; a
project that has only measured itself cannot.

## 9. The next phase

Phase A is closed: the encoder is parallel and streaming, the photographic
baseline is repeated and trustworthy, and the corpus is real. **Phase B
starts now**, and it is the first phase that changes the container:

* **P6 — the multiresolution preview pyramid.** More than one grid of tiles,
  with the level a reader actually fetches measured rather than assumed. The
  baseline is §3.5 of `docs/P5_BENCH_REPORT.md`.
* **P7 — adaptive tiling.** Variable tile dimensions, so the predictor gets
  more context where the content wants it. The baseline is §3.2 of this
  report, per category.

They are done back to back and the container version is bumped **once**.
After that: the reserved codec ids 4–6 are promoted against this corpus, and
then Phase C — the Python bindings, the WASM package, HTTP Range streaming,
layers and masks, GPU-ready payloads, and the release cadence that leads to
the v1.0 freeze.
