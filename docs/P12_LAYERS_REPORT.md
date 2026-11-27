# P12 — Layers and masks (format revision v0.4)

**Date:** 2026-11-27
**Deliverables:** the layer table in `crates/vpng-container/src/layers.rs`; the
v0.4 header fields in `crates/vpng-container/src/header.rs`; the layer index
in the directory's `flags` in `crates/vpng-container/src/directory.rs`; the
layer derivation in `crates/vpng-decoder/src/layer.rs`; the compositing in
`crates/vpng-decoder/src/composite.rs`; the layered encoder in
`crates/vpng-encoder/src/layers.rs` and `src/stream.rs`; the CLI's layer
commands in `crates/vpng-cli/src/layers.rs` and the strict JSON reader in
`src/json.rs`; the Python surface in `crates/vpng-python/src/lib.rs`; the
WebAssembly exports in `crates/vpng-wasm/src/ffi.rs`; the measurement driver
`benchmarks/corpus/run_layers_corpus.py` and the memory driver
`benchmarks/layers/measure_layer_memory.py`; the raw result files
`benchmarks/p12_layers_results.json` and
`benchmarks/p12_layers_memory_results.json`; the specification
`docs/VPNG_SPEC_0_4.md`; and the decision record
`docs/decisions/ADR-0018-layers-and-masks.md`.
**Status:** the case **ran** — it did not have to be reported BLOCKED. Every
figure below is read out of the raw JSON, and `check_layers()` in
`benchmarks/verify_reports.py` binds the quoted figures to it.

---

## 1. What this is

Phase C step 3 of the agreed plan: **the format's layer model.**

A v0.4 container holds one or more **layers**. Layer 0 is the **base layer**
— the picture itself. Every other layer is a named grid of tiles that sits on
the same canvas, with its own rectangle, its own opacity and, when the format
has an 8-bit alpha channel, its own **mask**: another layer whose alpha
channel says how much of this one shows through.

### 1.1 The binary layout did not move — the reserved bytes did

The 64-byte header and the 40-byte directory entry keep their **exact**
layouts. What changed is that two fields that were *reserved* now have a job:

* the header's last eight bytes — reserved in v0.1–v0.3, and required to be
  zero there — are now the **layer table's offset and length** (u32 each);
* the directory entry's `flags` field — whose low byte carried the level
  index since v0.2 — now also carries the **layer index** in bits 8..11.

Nothing moved. Every byte that existed before is at the offset it was at, and
the reserved bytes of an old file are still exactly zero, so **no old file is
affected**. A v0.1, v0.2 or v0.3 file still reads exactly as it did.

### 1.2 The default path writes the same bytes

The default encoder path writes no layer table, so the header's table fields
are zero, the feature flags are zero and the minor version is still 1. The
**corpus total is 453,767,460 bytes** — the published stable baseline's, to
the byte — and the 16K photograph's file digest is
`d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf`, the same
digest the P5, P6, P8, P9, P10 and P11 reports quote. The v0.4 work moved no
published byte.

### 1.3 What a layer is, and what it is not

A layer is **one grid of tiles that can be decoded on its own**. Layers are
encoded **independently**: no layer predicts from another, and no state
crosses a layer boundary. The consequence is useful and measured below: adding
an overlay does not change the base layer's bytes.

A layer's **tiles carry canvas coordinates**, so a reader can place them
without knowing where the layer sits; the layer's **pixels** live in the
layer's own rectangle, so the pixels of a 32×16 layer at `(8, 8)` are a 32×16
image.

### 1.4 The compositing is straight alpha and integer

The blend is straight-alpha "source over". The weight of a pixel is

```text
w = source_alpha * opacity * coverage / (max * 255 * max)
```

carried as a **fraction**, not as a floating-point number, so the result does
not depend on the host's floating-point behaviour. Every operation is
integer, so the same file composites to the same bytes on every platform. The
alpha channel blends on its own, as the weight itself.

**A one-layer file composites to exactly its own pixels.** The blend of a
single layer is the identity, not an approximation. That is measured on
**750 of 750** corpus images below.

---

## 2. How it was measured

### 2.1 The host

`stable-x86_64-pc-windows-gnu`, rustc 1.100.0, an Intel Core i5-1235U with
**10 physical cores and 12 logical processors**, 7.7 GB of physical memory.
CPython 3.13.0. The CLI binary is the **release** profile. Tile size 256,
**4 encoder workers**, one image at a time.

### 2.2 The corpus

The same **750-image corpus** the P5, P6, P7, P8, P9, P10 and P11 reports
used: **300** photographs, **100** pixel-art images, **100** scans, **150**
textures and **100** UI screenshots. The canonical pixels come from the same
cache, so the bytes the layered files are built from are the bytes the other
reports encoded.

The canonical formats are `rgb8` (595 images), `rgba8` (106), `gray8` (41)
and `graya8` (8). The **114** images whose format has an alpha channel can
carry a mask; the other 636 cannot, and the encoder refuses to write one for
them.

### 2.3 The method

For each image, three files are built with the **real `vpng` binary** from
the cached canonical pixels:

1. **plain** — the ordinary `vpng encode`, which writes a v0.1 file. This is
   the baseline the layer table's overhead is measured against.
2. **one** — a one-layer v0.4 file: the same pixels, in a layer table.
3. **two** — a two-layer v0.4 file: the base, plus an **overlay** covering
   the top-left quadrant whose colour channels are **inverted** and whose
   alpha is 255.

The overlay is inverted on purpose. An 8-bit sample `v` becomes `255 - v`,
and no integer `v` equals `255 - v`, so the composite is *provably* different
from the base rather than probably different. That is what makes the
"the composite really did composite" check meaningful.

For the **114** images with an alpha channel, a third file is built as well:
the same overlay, behind a **mask** whose coverage ramps from 0 at the left
edge to 255 at the right.

Every file is then:

* checked with `vpng check`, which CRCs every payload;
* **composited** with `vpng composite`, and the resulting bytes are compared
  with the canonical pixels, byte for byte;
* for the two-layer file, the **base layer** is decoded on its own with
  `vpng decode-layer 0` and compared with the canonical pixels, byte for
  byte;
* listed with `vpng layers --json`, which is where the per-layer byte
  accounting comes from — the numbers in the table are read out of the file,
  not out of the encoder.

### 2.4 The percentile rules

The same rules the other reports use: the median is the middle sample, or the
mean of the two middle samples when the count is even; the p95 is the
**nearest-rank** percentile; the standard deviation is the **sample** one,
with n − 1 in the denominator.

### 2.5 What is in the byte figures

This measurement reports **bytes in files**, not process memory.

* **Included**: the complete size of every file that was written, header,
  directory, layer table and payloads alike.
* **Excluded**: the decoded pixels, the raw files the composites were written
  to, the driver's own allocations, and every process working set.

The layer table's overhead is therefore the **whole** difference between the
plain file and the one-layer file: the table's bytes, and nothing else is
missing from either side.

### 2.6 What is in the timing figures

**There are none in this report.** This milestone measures bytes and
equality, not speed. The corpus pass took **664.418 s** wall-clock for 750
images and three files each, and that figure is a driver total, not a
benchmark: it includes the Python driver's own work and the writes of the
raw files.

### 2.7 What is in the memory figures

The memory figures come from a **separate, single-image measurement**
(`benchmarks/layers/measure_layer_memory.py`) on the 16K photograph, and they
are **process** figures.

* **Measured**: the process's **peak working set**, taken from the operating
  system's own `GetProcessMemoryInfo` while the process was alive. It is the
  process's high-water mark.
* **Included**: the operating system, the loader, the binary's text and data,
  the Rust heap, every buffer the command allocated, and the file it read.
* **Excluded**: nothing. The figure is the whole process.
* **Not a target claim**: the **64 MB localized-editing target** applies to
  *editing* — rewriting one tile of a 16K image. A composite blends the whole
  canvas, so it holds the whole canvas. These two numbers are not the same
  kind of number and are not compared to each other.

---

## 3. What was measured

### 3.1 The headline

| | |
|---|---|
| Images measured | **750** of 750 |
| One-layer files whose composite is **exactly** the base's pixels | **750** of 750 |
| Two-layer files whose **base layer** is exactly the original's pixels | **750** of 750 |
| Two-layer files whose composite **differs** from the base | **750** of 750 |
| Files that failed `vpng check` | **0** |
| Images with an alpha format, given a mask | **114** |
| Masked files that failed `vpng check` | **0** |
| Masked composites that differ from the unmasked composite | **114** of 114 |
| Plain (v0.1) total | **453,767,460** bytes — the published stable baseline's |
| One-layer (v0.4) total | **453,812,460** bytes |
| The layer table's cost, over 750 files | **45,000** bytes — **0.009917%** |
| Canonical pixel bytes | **1,450,363,330** bytes |

The plain total is the published baseline's, to the byte. That is the
cross-check that matters: **the layered work did not move the default path.**

### 3.2 What a layer table costs

The overhead is **exactly 60 bytes per file**, on every one of the 750
images, with no exceptions:

| | bytes |
|---|---|
| The table's header — count, descriptor length, CRC32C, one reserved word | 16 |
| One descriptor — name length, kind, rectangle, opacity, mask index, two byte counts | 40 |
| The name `base` | 4 |
| **The whole table** | **60** |

So the one-layer file is the plain file plus 60 bytes, and the **layer index
in the directory entry's `flags` costs nothing** — it is a field that was
already there and was zero.

Per category, the overhead's share of the file varies, and the variation is
the interesting part:

| Category | Images | Plain (v0.1) | One layer (v0.4) | Table overhead | Overhead share |
|---|---|---|---|---|---|
| photos | 300 | 234,827,986 | 234,845,986 | 18,000 | 0.007665% |
| pixel_art | 100 | 365,091 | 371,091 | 6,000 | **1.643426%** |
| scans | 100 | 67,871,353 | 67,877,353 | 6,000 | 0.008840% |
| textures | 150 | 133,872,854 | 133,881,854 | 9,000 | 0.006723% |
| ui | 100 | 16,830,176 | 16,836,176 | 6,000 | 0.035650% |
| **all** | **750** | **453,767,460** | **453,812,460** | **45,000** | **0.009917%** |

The **pixel-art row is the unfavourable one** and it is kept as it is:
**1.643426%** of a pixel-art file's bytes is the layer table, because a
pixel-art file is tiny — 3,650.91 bytes on average — and the table's cost is
fixed. A file of one kilobyte would spend **5.86%** of its bytes on the
table, and the corpus's **worst case** is a **111-byte** texture whose layer
table is **54.054054%** of the file. The table's cost is a **fixed 60
bytes**, not a proportional one, and a user who does not need layers does not
pay it: the default path writes no table at all.

The table does not push any file over the **10,000,000-byte ceiling**:
**748** of the 750 one-layer files are under it, exactly as **748** of the
750 plain files are. The two files that are over it are the two that were
over it before.

### 3.3 The two-layer file

The two-layer file adds the overlay's own payloads, the overlay's directory
entries (40 bytes per tile) and a second descriptor (40 bytes plus the name
`ink`):

| | bytes |
|---|---|
| Two-layer total, over the corpus | **568,637,618** |
| The one-layer total | 453,812,460 |
| The overlay's cost | **114,825,158** |
| The base layer's bytes in the two-layer file | **identical** to the one-layer file's, for all 750 images |

That last row is the structural claim of the layer model, and it is checked
for every image: the base layer's **compressed** bytes and its **uncompressed**
bytes are the same numbers in the one-layer file and in the two-layer file.
Adding an overlay did not change the base by one byte.

### 3.4 The per-layer ratios

Over all **1,500** layers (the base and the overlay of each image), the
per-layer ratio — uncompressed pixel bytes over compressed payload bytes, as
the file itself reports it:

| | min | median | p95 | mean | stdev | max |
|---|---|---|---|---|---|---|
| All layers | 1.0 | **2.548115** | 23.609023 | 5.996338 | 9.884095 | 102.4 |
| The overlays alone | 1.0 | **2.627658** | 23.251727 | 6.245591 | 11.270150 | 102.4 |

The **minimum is 1.0**: at least one layer did not get smaller at all. The
codecs are allowed to make a tile bigger, and the ratio is reported as it is
rather than clamped.

### 3.5 The big picture

The 16K photograph — **124,197,035** bytes as a plain file — is the clearest
case:

| | bytes | tiles | layer ratio |
|---|---|---|---|
| The plain v0.1 file | 124,197,035 | 2,040 | 3.207751 |
| The one-layer v0.4 file | **124,197,095** | 2,040 | 3.207751 |
| The two-layer v0.4 file | **155,472,590** | 2,550 | 3.184535 (the overlay) |

The one-layer file is the plain file plus **60 bytes**. Its digest is
`018cb6491f9dc944401ea63d5c938c9ad3fe0992dde167708ee0b018604cf6bb`; the
plain file's is the published
`d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf`. The
overlay — 7,680×4,320 pixels of the inverted picture — adds **31,275,495**
bytes, which is its own payload and nothing else.

### 3.6 The memory figures

On the 16K photograph (398,131,200 canonical bytes):

| Command | Peak working set |
|---|---|
| `vpng layers --json` | **159,817,728** bytes |
| `vpng check` | **159,768,576** bytes |
| `vpng decode-layer 0` (the base) | **558,731,264** bytes |
| `vpng decode-layer 1` (the overlay) | **260,386,816** bytes |
| `vpng composite` (one layer) | **527,650,816** bytes |
| `vpng composite` (two layers) | **658,427,904** bytes |

**The composite is a whole-canvas operation** and the figures say so. The
one-layer composite's 527,650,816 bytes are the 398,131,200-byte canvas plus
the 124,197,095-byte file plus the decoder's tile buffers; the two-layer
composite adds the 99,532,800-byte overlay.

This is **not** a breach of the 64 MB localized-editing target. That target
is about *editing* — rewriting one tile of a 16K image without holding the
picture — and the editing engine still meets it; the P6 memory matrix's
worst case is 13,770,752 bytes of resident-set growth. The composite is a
different operation with a different shape, and §2.7 says so.

### 3.7 The refusals and the bounds

The sixteen tests in `crates/vpng-testkit/tests/v4_formats.rs` pin the
format's rules, and the six in `crates/vpng-encoder/tests/layer_identity.rs`
pin the encoder's:

| Test | What it pins |
|---|---|
| `the_default_path_is_still_a_v0_1_file` | the default path's version did not move |
| `a_layered_file_is_a_v0_4_file` | the version states what the file contains |
| `a_v0_3_file_may_not_carry_a_layer_table` | the table is a v0.4 thing |
| `the_layer_index_travels_in_the_directory_flags` | the index is in bits 8..11 |
| `the_layer_table_accounts_for_every_byte` | the accounting is exact |
| `a_file_without_a_layer_table_has_one_layer` | a flat file still has a base layer |
| `a_layered_file_may_not_also_have_a_pyramid` | the two features are exclusive |
| `the_base_layer_must_cover_the_whole_canvas` | the base is the canvas |
| `a_layer_may_not_leave_the_canvas` | a layer stays on the canvas |
| `a_layer_must_hold_the_pixels_its_rectangle_says` | the rectangle and the pixels agree |
| `a_mask_is_a_mask_layer_and_needs_an_alpha_channel` | a mask needs an alpha format |
| `a_mask_must_cover_the_layer_it_masks` | the mask covers its layer |
| `a_mask_modulates_the_layer_it_masks` | the mask changes the result |
| `a_mask_is_never_drawn_on_its_own` | a mask is not a picture |
| `every_layer_decodes_exactly` | every layer is exact |
| `a_one_layer_file_composites_to_exactly_its_pixels` | the one-layer composite is the identity |
| `the_worker_count_does_not_change_the_bytes` | 1, 2, 4 and 8 workers agree |
| `the_streaming_writer_does_not_depend_on_the_worker_count_either` | the writer agrees too |
| `the_batch_encoder_and_the_streaming_writer_agree` | the two write paths agree |
| `a_layer_is_encoded_on_its_own` | a layer's bytes do not depend on its neighbours |
| `a_layered_file_may_not_ask_for_a_pyramid` | the encoder refuses the combination |
| `a_file_may_not_hold_more_than_sixteen_layers` | the limit is 16 |

Nine unit tests in `crates/vpng-decoder/src/composite.rs` pin the blend
itself — an opaque source replaces the destination, a transparent source
changes nothing, half opacity is exactly 127, a fully covered mask does not
change the blend, a mask that says nothing hides the source, the alpha
channel blends on its own, a layer hanging over the canvas's edge is clipped,
and 16-bit samples blend too.

Three tests in `crates/vpng-editing/tests/v4_layers.rs` pin the **refusal**:
the editing engine refuses a file that carries a layer table, in memory and
on disk, with a message that says "layer table". It does not silently drop
the layers.

Nine tests in `crates/vpng-cli/tests/v4_features.rs` pin the command line:
the version, the layer list, the base layer's pixels, the one-layer
composite, the mask's effect, and three refusals (a broken manifest, an
empty manifest, a mask that does not cover).

Fifteen tests in `wasm/test/vpng.test.mjs` pin the WebAssembly surface,
including two new ones: the module reports the three layers and their names,
and the overlay's pixels are the command line's pixels.

---

## 4. The twelve questions

**Q1 — Was the corpus actually run, or blocked?**
**Run, not blocked.** All **750** images were encoded three ways — plain,
one layer, two layers — and the 114 with an alpha format were encoded a
fourth time with a mask. Every file was checked, every composite was
compared byte for byte with the canonical pixels. The raw file is
`benchmarks/p12_layers_results.json`.

**Q2 — How many times was each case run, and what do the percentiles mean?**
Each case is a **single run per image**, and the corpus is the repetition:
750 images in each configuration. The percentile rules are the repository's:
the median is the middle sample, or the mean of the two middle samples when
the count is even; the p95 is the **nearest-rank** percentile; the standard
deviation is the sample one, with n − 1 in the denominator. The ratio
distribution in §3.4 is over the 1,500 layers.

**Q3 — What is the baseline, and was it measured in the same run?**
**The plain v0.1 file, and it was measured in the same run.** Every image was
encoded with the ordinary `vpng encode` in the same driver pass, and its
bytes and SHA-256 recorded. The plain total is **453,767,460** bytes — the
published stable baseline's, to the byte — and the 16K photograph's plain
digest is the published `d029a7eb…`.

**Q4 — Do the layered files change the pixels?**
**No.** The composite of a one-layer file is byte-for-byte the base's pixels
on **750 of 750** images. The base layer of a two-layer file is byte-for-byte
the original's pixels on **750 of 750** images. The reconstruction is
**exact**.

**Q5 — What does the layer model buy?**
**Independent, named, individually decodable grids — at a fixed 60 bytes per
file.** The base layer's compressed and uncompressed byte counts are the
same numbers in the one-layer and the two-layer file for all 750 images:
adding an overlay did not touch the base. A layer can be decoded on its own,
re-encoded on its own, and its own ratio is reported by the file itself.

**Q6 — How fast is it?**
**This milestone does not claim a speed.** §2.6 says why: the corpus pass
took 664.418 s for 750 images and three files each, and that is a driver
total, not a benchmark. The encoder's own byte-identity across worker counts
(1, 2, 4, 8) is measured and pinned, but no throughput figure is quoted.

**Q7 — What does it cost?**
**60 bytes per file for the table**, exactly, on every image — **45,000
bytes** over the corpus, **0.009917%** of the corpus's bytes. The **worst
case is the small file**: a pixel-art file spends **1.643426%** of its bytes
on the table, and the corpus's smallest file — 111 bytes — spends
**54.054054%**. The two-layer file costs the overlay's payloads, its
directory entries and one more descriptor: **114,825,158** bytes over the
corpus. Masks cost their own layer: the mask's payloads, its directory
entries and its descriptor.

**Q8 — Does the layer model move the default path's bytes or version?**
**No.** The default path writes no layer table. The corpus total is the
published baseline's **453,767,460** bytes, every plain file reports minor
version 1, and the 16K photograph's plain digest is the published one. The
one-layer files are minor version **4** and the two-layer files are minor
version **4**.

**Q9 — Is the format still binary-compatible?**
**Yes, and the milestone is an extension, not a redesign.** The 64-byte header
and the 40-byte directory entry keep their exact layouts. The two fields that
changed were **reserved** bytes — the header's last eight (required to be
zero in v0.1–v0.3) and four bits of the directory entry's `flags`. No old
file is affected, because an old file's reserved bytes are still zero. A
v0.4 file is rejected by an older reader for the ordinary reason: the minor
version is newer than the reader knows.

**Q10 — Are the research codecs still out of reach?**
**Yes.** The shared metadata validation refuses a directory entry that names
an unimplemented codec, and the three promoted codecs may only be named by a
file whose minor version says so. The layer work added no codec and changed
no codec rule.

**Q11 — What is in the memory figures?**
**Process peak working sets, and §2.7 says exactly what is in them**: the
operating system, the loader, the binary, the Rust heap, every buffer the
command allocated and the file it read — nothing is excluded. The figures are
for the 16K photograph and they are **not** the 64 MB localized-editing
target: a composite blends the whole canvas, so it holds the whole canvas.
The editing engine's own target is unchanged and still met.

**Q12 — What is not measured, and what remains untested?**
The limits are in §6. The four that matter most: the **editing engine
refuses a layered file** rather than editing it, so the layer model's edit
path does not exist yet; **cross-layer prediction is out of scope**, so an
overlay that resembles the base does not compress better than it would alone;
the **HTTP Range reader and the WebAssembly module read the base layer only**
and do not composite; and **no speed was measured** at all.

---

## 5. Reproducing the evidence

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# One: the format's own tests.
cargo test -p vpng-container -p vpng-decoder -p vpng-encoder
cargo test -p vpng-testkit --test v4_formats
cargo test -p vpng-encoder --test layer_identity
cargo test -p vpng-editing --test v4_layers
cargo test -p vpng-cli --test v4_features

# Two: the WebAssembly surface.
cargo build --release --target wasm32-unknown-unknown -p vpng-wasm
cargo build --release -p vpng-cli
node --test wasm/test/vpng.test.mjs
node wasm/check_exports.mjs

# Three: the corpus.
cargo build --release -p vpng-cli
python benchmarks\corpus\run_layers_corpus.py --jobs 1 --workers 4

# Four: the memory figures.
python benchmarks\layers\measure_layer_memory.py

# Five: the whole Rust suite, and the check that every quoted figure matches
# the raw data.
cargo test --workspace --all-targets
python benchmarks/verify_reports.py
```

---

## 6. What this does not claim

* **No claim that the editing engine can edit a layered file.** It cannot:
  it **refuses** such a file with a message that says "layer table". A
  refusal is not a capability.
* **No claim about cross-layer prediction.** Layers are encoded independently.
  An overlay that closely resembles the base does not compress better than
  it would on its own.
* **No claim that the HTTP or WebAssembly readers composite.** They read the
  base layer. `vpng http-decode-region` says so when the file has more than
  one layer.
* **No claim about speed.** §2.6: there are no timing figures in this
  report.
* **No claim that the layer table is free.** It is 60 bytes, and on a
  pixel-art file that is **1.643426%** — and on the corpus's smallest file,
  **54.054054%**.
* **No claim that a composite is cheap.** It holds the whole canvas:
  **527,650,816** bytes for the 16K photograph's one-layer composite.
* **No claim that VPNG now beats PNG.** The P5 report's unfavourable
  comparison with PNG, WebP and JPEG XL still stands unchanged.

---

## 7. The acceptance gates

| # | Gate | Result | Evidence |
|---|---|---|---|
| 1 | The corpus actually ran, at its full size | **PASS** | 750 of 750 images, three files each and a fourth for the 114 with an alpha format; `benchmarks/p12_layers_results.json` |
| 2 | A one-layer file composites to exactly its own pixels | **PASS** | 750 of 750 composites are byte-for-byte the canonical pixels |
| 3 | The base layer of a layered file is the original | **PASS** | 750 of 750 base layers are byte-for-byte the canonical pixels |
| 4 | The composite really composites | **PASS** | 750 of 750 two-layer composites differ from the base, and 114 of 114 masked composites differ from the unmasked ones |
| 5 | Every file passes the container's own check | **PASS** | 0 failures across the plain, one-layer, two-layer and masked files |
| 6 | The layer table's cost is measured, not estimated | **PASS** | exactly 60 bytes per file, 45,000 bytes over the corpus, 0.009917% |
| 7 | The accounting in the file is exact | **PASS** | the reader recomputes each layer's two byte counts from the directory and rejects a mismatch; 750 of 750 files agree |
| 8 | The layers are encoded independently | **PASS** | the base layer's compressed and uncompressed byte counts are identical in the one-layer and the two-layer file for all 750 images |
| 9 | The output is deterministic | **PASS** | 1, 2, 4 and 8 workers write the same bytes; the batch encoder and the streaming writer agree |
| 10 | The default path's bytes and version do not move | **PASS** | the corpus total is the published baseline's **453,767,460**; every plain file is minor version 1 |
| 11 | The binary layout is unchanged | **PASS** | the 64-byte header and the 40-byte entry keep their exact layouts; only reserved fields gained a job |
| 12 | Every quoted figure is bound to the raw data | **PASS** | `check_layers()` in `benchmarks/verify_reports.py` |

---

## 8. What is next

The layer model is in, measured and documented. What it leaves open:

* **Editing a layered file.** The engine refuses one today. Carrying the
  layer table and the per-layer accounting through a commit is the next
  piece of work in the editing engine.
* **Compositing over HTTP and in the browser.** Both readers read the base
  layer today. The pieces are there — `decode_layer_from` is public, and the
  layer table is fetched by the range reader — but the compositing path is
  not wired up.
* **Then** GPU-ready payloads and the graphical UI; and **last** the
  v0.2 → v0.23 release cadence and the v1.0 freeze.
