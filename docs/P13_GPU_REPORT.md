# P13 — GPU payloads (format revision v0.5)

**Date:** 2026-11-28
**Deliverables:** the GPU table in `crates/vpng-container/src/gpu.rs`; the
v0.5 header fields in `crates/vpng-container/src/header.rs`; the BC1 and BC4
block codecs in the new crate `crates/vpng-gpu`; the GPU payload's production
in `crates/vpng-encoder/src/parallel.rs`; the GPU table's assembly in
`crates/vpng-encoder/src/lib.rs` and `src/stream.rs`; the GPU decode in
`crates/vpng-decoder/src/gpu.rs`; the range reader's GPU path in
`crates/vpng-source/src/reader.rs`; the editing engine's refusal in
`crates/vpng-editing/src/source.rs`; the CLI's GPU commands in
`crates/vpng-cli/src/gpu.rs`; the Python surface in `crates/vpng-python/src/lib.rs`;
the WebAssembly exports in `crates/vpng-wasm/src/ffi.rs`; the measurement
drivers `benchmarks/corpus/run_gpu_corpus.py` and
`benchmarks/gpu/measure_gpu_memory.py`; the raw result files
`benchmarks/p13_gpu_results.json` and
`benchmarks/p13_gpu_memory_results.json`; the specification
`docs/VPNG_SPEC_0_5.md`; and the decision record
`docs/decisions/ADR-0019-gpu-payloads.md`.
**Status:** the case **ran** — it did not have to be reported BLOCKED. Every
figure below is read out of the raw JSON, and `check_gpu()` in
`benchmarks/verify_reports.py` binds the quoted figures to it.

---

## 1. What this is

Phase C step 4 of the agreed plan: **the format's optional GPU payload.**

A v0.5 container may carry, beside every tile's lossless payload, a second,
explicitly named, **lossy** payload: the same picture in the block format a
GPU samples as a texture — **BC1** colour blocks and, when the format has an
alpha channel, **BC4** alpha blocks.

### 1.1 The lossless payload stays normative

This is the rule everything else hangs off:

* the directory entry's `codec_id` **never** names a GPU block codec — a
  reader refuses such an entry with `UnsupportedFeature`;
* the decoder's exact paths — `decode_region`, `decode_layer`, `composite`,
  `verify`, `check` — **never** read the GPU payload;
* the GPU payload is read only by an explicitly named API that says it is
  **lossy** in its own name and in its output.

So a file that carries a GPU payload still reconstructs **exactly**, through
its lossless payload, as if the GPU payload were not there. That is measured
on **750 of 750** corpus images below: every GPU file's decoded pixels are
byte for byte the canonical pixels.

### 1.2 The header grows to 72 bytes, through the field that exists for it

The 64-byte header has no room left: bytes 56..64 hold the layer table's
location since v0.4, and the directory entry's 40 bytes are all in use. So the
header **grows**, and it grows through the `header_size` field that was put
there for exactly this. A v0.5 file's header is **72 bytes**:

| Offset | Size | Field |
|-------:|-----:|-------|
| 0 | 8 | magic |
| 8 | 2 | major version |
| 10 | 2 | minor version |
| 12 | 4 | header size (64, or 72 from v0.5) |
| 16 | 24 | width, height, channels, bits, colour model, default codec, tile count |
| 32 | 8 | directory offset |
| 40 | 8 | declared file size |
| 48 | 4 | feature flags |
| 52 | 4 | header checksum (CRC32C) |
| 56 | 4 | layer table offset |
| 60 | 4 | layer table length |
| 64 | 4 | **GPU table offset** (v0.5) |
| 68 | 4 | **GPU table length** (v0.5) |

The header checksum covers bytes `0..52` and `56..header_size`. For a 64-byte
header that is exactly the rule v0.1–v0.4 use, so **no existing file's
checksum changes** and no existing byte moves. A v0.1–v0.4 file's header is
still 64 bytes and still says so.

### 1.3 The GPU table sits with the other tables

The table sits after the directory, and after the layer table when there is
one. Its header is **24 bytes** — the record count, the descriptor array's
length, a CRC32C over that array, one reserved word, and the total number of
GPU payload bytes — and it is followed by **one 20-byte record per tile**, in
the same canonical `(layer, level, y, x)` order as the directory.

One record per tile, rather than a record per payload that exists, because
the record is then *indexed by tile*: a reader finds a tile's GPU payload
without scanning, and the table's size is a function of the tile count alone.
A tile with no GPU payload says so — both codec ids are zero, and so are the
offset, the length and the checksum.

Each record carries the GPU payload's **absolute offset**, its **length**, the
**codec id** of its colour blocks, the **codec id** of its alpha blocks, and a
**CRC32C** of the payload. The absolute offset matters: it is what lets an
HTTP Range reader fetch exactly one tile's GPU payload and nothing else.

### 1.4 The payloads are interleaved, and the default path writes the same bytes

Each tile's lossless payload is written, then its GPU payload, then the next
tile's lossless payload. One contiguous GPU region would have been tidier to
describe and would have needed the whole GPU region in memory before it could
be written — about 66 MB for the 16K photograph, more than the 64 MB budget
the project holds itself to. Interleaving costs a range reader nothing: it
fetches `[payload_offset, payload_offset + length)` and
`[gpu_offset, gpu_offset + gpu_length)`, and both are exact ranges.

The default encoder path writes no GPU table, so the header's table fields
are zero, the feature flags are zero and the minor version is still 1. The
**corpus total is 453,767,460 bytes** — the published stable baseline's, to
the byte — and the 16K photograph's file digest is
`d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf`, the same
digest the P5, P6, P8, P9, P10 and P11 reports quote. The v0.5 work moved no
published byte.

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
cache, so the bytes the GPU files are built from are the bytes the other
reports encoded.

The canonical formats are `rgb8` (595 images), `rgba8` (106), `gray8` (41)
and `graya8` (8).

### 2.3 The method

For each image, two files are built with the **real `vpng` binary** from the
cached canonical pixels:

1. **plain** — the ordinary `vpng encode`, which writes a v0.1 file. This is
   the baseline the GPU payload's cost is measured against.
2. **gpu** — the same pixels with `--gpu`, which writes a v0.5 file carrying a
   GPU payload beside every tile's lossless payload.

Every GPU file is then:

* checked with `vpng check`, which CRCs every payload, lossless and GPU;
* listed with `vpng info --json` and `vpng gpu-info --json`, which is where
  the minor version, the table's offset and length, the record count and the
  payload byte counts come from — the numbers in the tables are read out of
  the file, not out of the encoder;
* **decoded exactly** with `vpng decode`, and the result compared with the
  canonical pixels, byte for byte;
* **decoded lossily** with `vpng gpu-decode`, and the result compared with
  the canonical pixels **channel by channel**: the mean squared error, the
  peak signal-to-noise ratio, the mean absolute error and the worst single
  difference are all measured.

The comparison holds **one row** of each picture in memory at a time, which
is what lets the 16K photograph be measured on a host with a few gigabytes of
RAM. The statistics are over **every** channel, the alpha channel included;
for a format without one, the original's implicit alpha is 255 and the GPU
payload's alpha is 255 too.

### 2.4 The percentile rules

The same rules the other reports use: the median is the middle sample, or the
mean of the two middle samples when the count is even; the p95 is the
**nearest-rank** percentile; the standard deviation is the **sample** one,
with n − 1 in the denominator.

### 2.5 What is in the byte figures

This measurement reports **bytes in files**, not process memory.

* **Included**: the complete size of every file that was written, header,
  directory, GPU table and payloads alike.
* **Excluded**: the decoded pixels, the raw files the GPU payload was written
  to, the driver's own allocations, and every process working set.

The GPU payload's overhead is therefore the **whole** difference between the
plain file and the GPU file: the GPU table's bytes, the GPU payloads' bytes,
and the eight bytes the header grew by. Nothing is missing from either side.

### 2.6 What is in the timing figures

The driver records each encode's wall-clock time, and the totals are here —
but they are **not a benchmark** and they do not support a speed claim.
Each case is a **single run**, and in every pair the plain encode ran
**first** and the GPU encode **second**, so the second run had the file
system's cache already warm. The GPU encode came out at **0.8617×** the
plain encode's total in that order — the plain encode's median was **208**
ms and the GPU encode's **185** ms — which is an artefact of the order and
not a statement about the block codecs. The corpus pass took **1160.77 s**
wall-clock for 750 images and two files each, and that figure is a driver
total: it includes the Python driver's own work and the writes of the raw
files.

### 2.7 What is in the memory figures

The memory figures come from a **separate, single-image measurement**
(`benchmarks/gpu/measure_gpu_memory.py`) on the 16K photograph, and they are
**process** figures.

* **Measured**: the process's **peak working set**, taken from the operating
  system's own `GetProcessMemoryInfo` while the process was alive. It is the
  process's high-water mark.
* **Included**: the operating system, the loader, the binary's text and data,
  the Rust heap, every buffer the command allocated, and the file it read.
* **Excluded**: nothing. The figure is the whole process.
* **Not a target claim**: the **64 MB localized-editing target** applies to
  *editing* — rewriting one tile of a 16K image. These commands encode,
  check and decode **whole pictures**, so they hold whole pictures. These
  numbers are not the same kind of number and are not compared to each other.

**A defect was found and fixed in the driver before these figures were
taken.** The driver ran each command with its output on a **pipe** and never
read the pipe while the child ran, so a command that printed more than a
pipe's worth — `vpng gpu-info --json` over the 16K photograph's **2,040**
records is about 194 KB — blocked forever waiting for a reader that never
came. The first run of this measurement hung on exactly that command. The
driver now writes the child's output to a **temporary file**, which never
blocks, and the same fix was applied to the layer-memory driver and to the
shared `benchmarks/measure_peak.py`. The figures below are from the clean
re-run, in which every command exited **0**.

---

## 3. What was measured

### 3.1 The headline

| | |
|---|---|
| Images measured | **750** of 750 |
| GPU files whose **lossless** payload reconstructs the picture **exactly** | **750** of 750 |
| Files that failed `vpng check` | **0** |
| Files whose byte accounting does not add up | **0** |
| Files that report minor version **5** | **750** of 750 |
| GPU payloads that **differ** from the original | **746** of 750 |
| GPU payloads that happen to be **identical** to the original | **4** of 750 |
| Plain (v0.1) total | **453,767,460** bytes — the published stable baseline's |
| GPU (v0.5) total | **705,483,532** bytes |
| The GPU payloads' bytes, over 750 files | **251,513,192** bytes |
| The GPU tables' bytes, over 750 files | **196,880** bytes |
| The GPU payload's cost, over 750 files | **251,716,072** bytes — **55.472482%** |
| The GPU payloads against the canonical bytes | **17.341392%** |
| Canonical pixel bytes | **1,450,363,330** bytes |

The plain total is the published baseline's, to the byte. That is the
cross-check that matters: **the GPU work did not move the default path.**

### 3.2 What a GPU payload costs

The overhead is **251,716,072 bytes** over 750 files: the GPU payloads'
**251,513,192**, the GPU tables' **196,880**, and the header's eight extra
bytes, 6,000 in total.

That is **55.472482%** of the corpus's plain bytes: **the GPU payload more
than doubles the corpus.** It is not a marginal cost and it is not hidden
behind a flag — the file is bigger, and the report says by how much.

Per category, the overhead's share of the file varies, and the variation is
the interesting part:

| Category | Images | Plain (v0.1) | GPU (v0.5) | GPU payloads | GPU tables | Overhead | Overhead share | Payload / canonical |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| photos | 300 | 234,827,986 | 334,041,218 | 99,134,352 | 76,480 | 99,213,232 | 42.249322% | 16.726497% |
| pixel_art | 100 | 365,091 | 1,334,635 | 964,144 | 4,600 | 969,544 | **265.562284%** | 25.044009% |
| scans | 100 | 67,871,353 | 107,133,989 | 39,229,816 | 32,020 | 39,262,636 | 57.848613% | 19.710657% |
| textures | 150 | 133,872,854 | 199,974,034 | 66,054,960 | 45,020 | 66,101,180 | 49.376089% | 17.050101% |
| ui | 100 | 16,830,176 | 62,999,656 | 46,129,920 | 38,760 | 46,169,480 | **274.325592%** | 17.251941% |
| **all** | **750** | **453,767,460** | **705,483,532** | **251,513,192** | **196,880** | **251,716,072** | **55.472482%** | **17.341392%** |

The **unfavourable rows are kept as they are**. A **UI screenshot** spends
**274.325592%** of its bytes on the GPU payload — the file more than triples —
and a **pixel-art** image spends **265.562284%**. The reason is structural:
a GPU payload is a **fixed 8 bytes per 4×4 block** whatever the picture is,
while a UI screenshot compresses extraordinarily well in the lossless
payload, so the two move in opposite directions.

The corpus's **worst single case** is a **25,977-byte** UI screenshot,
`github-docs-images-02c96358febd`, whose GPU file is **403,109** bytes: the
GPU payload is **1451.79197%** of the plain file — more than fifteen times
the size. A user who does not want a GPU payload does not pay for one: the
default path writes no table and no payload at all.

The **10,000,000-byte ceiling** is where the trade bites hardest. The plain
files clear it on **748** of 750; the GPU files clear it on **745**. The GPU
payload pushed **three** files over the line that were under it before:

| Image | Plain bytes | GPU bytes |
|---|---:|---:|
| `threejs-textures-09a9aaad82bf` | 8,320,876 | 12,517,772 |
| `threejs-textures-2b0d9924da39` | 9,352,555 | 13,549,451 |
| `threejs-textures-755cc8af847e` | 9,065,712 | 13,262,608 |

### 3.3 The fidelity, and what it costs

The GPU payload is **lossy**, and the report says how lossy. Over all
**1,998,310,392** channel samples:

| | |
|---|---|
| Peak signal-to-noise ratio | **37.2237 dB** |
| Mean absolute error | **1.456997** levels of 255 |
| Worst single difference | **170** levels |

The per-image PSNR distribution, over the **746** images whose GPU payload is
not identical to the original:

| | min | median | p95 | mean | stdev | max |
|---|---|---|---|---|---|---|
| PSNR (dB) | **13.3972** | **36.4972** | **44.3597** | 35.808753 | 6.58653 | 57.4420 |

And the per-image worst-difference distribution, over all **750**:

| | min | median | p95 | mean | stdev | max |
|---|---|---|---|---|---|---|
| Worst difference (levels) | **0** | **56** | **150** | 68.612000 | 42.038543 | **170** |

Two things in that table are worth naming.

**The minimum is 0, and it happens four times.** Four pixel-art images —
`phaser-example-sprites-2fc788ac4a48`, `phaser-example-sprites-31865c7bbdea`,
`phaser-example-sprites-3879b3f2212f` and
`phaser-example-sprites-48e49afc0c3e` — come back from the GPU payload
**byte for byte identical** to the original. BC1 reproduces a flat colour and
a hard black-and-white block exactly, and those four pictures are made of
them. That is not a defect; it is what the codec is good at, and it is
measured rather than assumed.

**The minimum PSNR is 13.3972 dB, and it is unfavourable.** The worst image is
`synthetic-noise-256-rgba8` — pure noise, which is the hardest possible input
for a block codec — at a mean absolute error of 40.220997 levels and a worst
difference of 170. The second worst is `threejs-textures-48661d3e9daa` at
13.4611 dB, and the worst pixel-art images sit between 15 and 18 dB. **A
picture's GPU payload can be badly wrong**, and the corpus contains pictures
where it is.

Per category, the pooled PSNR:

| Category | Images | PSNR (dB) | Mean absolute error | Worst difference |
|---|---:|---:|---:|---:|
| photos | 300 | 37.3293 | 1.582653 | 167 |
| pixel_art | 100 | 28.9811 | 2.106573 | 170 |
| scans | 100 | 40.0348 | 1.266216 | 89 |
| textures | 150 | 35.2384 | 1.903711 | 170 |
| ui | 100 | 39.7140 | 0.693174 | 170 |
| **all** | **750** | **37.2237** | **1.456997** | **170** |

The **pixel-art row is the worst one**, at **28.9811 dB**, and it is kept as
it is: pixel art is made of flat areas with hard edges, and BC1's four
colours per block cannot hold an arbitrary flat colour exactly.

### 3.4 The 16K photograph

The 16K photograph — **124,197,035** bytes as a plain file — is the clearest
case:

| | bytes | tiles |
|---|---:|---:|
| The plain v0.1 file | 124,197,035 | 2,040 |
| The GPU v0.5 file | **190,593,067** | 2,040 |
| The GPU payloads | **66,355,200** | — |
| The GPU table | 40,824 | 2,040 records |

The GPU file is the plain file plus **66,396,032** bytes — **53.460239%** —
which is the GPU payloads, the GPU table (24 bytes of header and 20 bytes per
tile) and the header's eight extra bytes, and nothing else. The GPU file's
digest is
`ba7441184ff165d099a9e872f906cd73deba19f2fc96cc00bf83c25bd24d461c`; the
plain file's is the published
`d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf`.

The fidelity of the 16K photograph's GPU payload is the **best** in the
corpus's photographs: **45.0635 dB**, a mean absolute error of **0.916696**
levels and a worst difference of **46**. A real photograph is the friendliest
input a block codec has.

### 3.5 The memory figures

*(The §2.7 scope applies: these are process peak working sets, and **nothing
is excluded** from them.)*

The 16K photograph — **398,131,200** canonical bytes, **124,197,035** bytes as
a plain file and **190,593,067** bytes as a GPU file — was measured with the
release binary:

| Command | Peak working set | Wall clock |
|---|---:|---:|
| `vpng encode` (plain) | **410,099,712** | 26,372 ms |
| `vpng encode --gpu` | **411,205,632** | 27,290 ms |
| `vpng gpu-info --json` | **195,555,328** | 169 ms |
| `vpng check` | **194,916,352** | 122 ms |
| `vpng decode` (exact, raw) | **594,096,128** | 21,086 ms |
| `vpng gpu-decode` (lossy, raw) | **726,097,920** | 1,608 ms |

**The encoder's GPU payload costs 1,105,920 bytes of memory** — the GPU
encode's peak against the plain encode's. That is the **interleaving**
paying off: the encoder writes each tile's GPU payload straight after that
tile's lossless payload, so it never holds the whole 66,355,200-byte GPU
region in memory, which would have been more than the project's 64 MB
budget on its own. The figure is a **single run**, and the plain encode's
own peak moved by a few hundred kilobytes between runs, so it is read as
"the GPU payload adds about a megabyte", not as an exact constant.

The file's accounting, read out of the file itself rather than inferred:

| | bytes |
|---|---:|
| The plain v0.1 file | 124,197,035 |
| The GPU v0.5 file | 190,593,067 |
| The GPU payloads | **66,355,200** |
| The GPU table (24 bytes of header and 20 bytes per tile) | **40,824** |
| The header's eight extra bytes | 8 |
| **The GPU payload's whole cost** | **66,396,032** — **53.460239%** |

The **readers** cost more than the writer, and the lossy reader costs most:
`vpng decode` peaks at 594,096,128 bytes because it reconstructs the whole
398,131,200-byte picture, and `vpng gpu-decode` peaks at 726,097,920 bytes
because it writes the picture as **RGBA8** — 530,841,600 bytes — whatever
the file's own format is. Neither is the 64 MB localized-editing target, and
neither is compared to it: these are whole-picture operations.

### 3.6 The refusals and the bounds

The sixteen tests in `crates/vpng-testkit/tests/v5_formats.rs` pin the
format's rules, and the six in `crates/vpng-encoder/tests/gpu_identity.rs`
pin the encoder's:

| Test | What it pins |
|---|---|
| `the_default_path_is_still_a_v0_1_file` | the default path's version did not move |
| `a_gpu_file_is_a_v0_5_file_with_a_72_byte_header` | the version and the header size state what the file contains |
| `the_gpu_flag_is_orthogonal_to_the_others` | a GPU payload and promoted codecs may live together |
| `the_lossless_payload_still_reconstructs_the_picture_exactly` | the load-bearing rule |
| `the_gpu_payload_is_lossy` | the GPU payload is a *different* picture |
| `the_gpu_table_holds_one_record_per_tile` | the table is indexed by tile |
| `a_record_without_alpha_names_no_alpha_codec` | an RGB8 tile has no alpha blocks |
| `every_payload_length_matches_the_tile_geometry` | the length is what the geometry implies |
| `every_payload_checksum_matches` | the payload's CRC32C is verified |
| `the_bytes_add_up` | the file's accounting is exact |
| `a_directory_entry_may_never_name_a_gpu_block_codec` | the codec ids never name a directory entry |
| `a_gpu_payload_with_a_flipped_byte_is_refused` | a corrupt payload is refused |
| `a_gpu_table_with_the_wrong_record_count_is_refused` | the record count is checked |
| `an_older_file_may_not_claim_the_gpu_feature` | the version states what the file contains |
| `a_sixteen_bit_picture_has_no_gpu_payload` | 16-bit is refused, not quietly dropped |
| `a_file_without_a_gpu_table_has_none` | a caller never has to ask "is this a GPU file?" first |
| `the_gpu_payload_agrees_across_every_path` | 1, 2, 4 and 8 workers, batch and streaming, all agree |
| `the_gpu_payload_makes_the_file_v0_5` | the version follows the contents |
| `the_lossless_payload_is_untouched` | adding the sibling moved no lossless byte |
| `a_gpu_file_is_larger_than_the_same_file_without_one` | the cost is reported, not inferred |
| `a_mask_layers_gpu_payload_carries_only_its_coverage` | a mask's tile carries BC4 coverage and no colour blocks |

One more test, in the command line's own suite, proves that the rule is
reachable from the public surface and not only from the Rust API:
`vpng encode-layers --gpu` writes a layered v0.5 file whose **mask** layer's
tile carries BC4 coverage alone and no colour blocks, while every base-layer
tile carries BC1 colour and BC4 alpha.

Twenty-one unit tests in `crates/vpng-gpu` pin the block codecs themselves:
the BC1 reference arithmetic (the five-bit and six-bit expansions, the
interpolated colours, the three-colour mode's opaque fourth entry), the BC4
reference weights and its two modes, and the payload lengths the geometry
implies.

Three tests in `crates/vpng-editing/tests/v5_gpu.rs` pin the **refusal**:
the editing engine refuses a file that carries a GPU table, in memory and on
disk, with a message that says "GPU table". It cannot rebuild a lossy
payload from the rewritten pixels, and copying the old one would leave a
stale picture in the file.

Nine tests in `crates/vpng-cli/tests/v5_features.rs` pin the command line:
the version, the table's contents, the lossless round trip, the lossy decode,
the refusal on a file with no table, the integrity check, the worker count's
not moving the bytes, and a layered file whose **mask** carries its coverage
alone.

Eighteen tests in `wasm/test/vpng.test.mjs` pin the WebAssembly surface,
including three new ones: the GPU payload is visible and is **lossy**, the
parts path opens a container with its GPU table, and a container without one
says so.

---

## 4. The twelve questions

**Q1 — Was the corpus actually run, or blocked?**
**Run, not blocked.** All **750** images were encoded twice — plain and with
`--gpu` — and every GPU file was checked, decoded exactly, decoded lossily
and measured against the canonical pixels. The raw file is
`benchmarks/p13_gpu_results.json`.

**Q2 — How many times was each case run, and what do the percentiles mean?**
Each case is a **single run per image**, and the corpus is the repetition:
750 images in each configuration. The percentile rules are the repository's:
the median is the middle sample, or the mean of the two middle samples when
the count is even; the p95 is the **nearest-rank** percentile; the standard
deviation is the sample one, with n − 1 in the denominator. The PSNR
distribution in §3.3 is over the 746 images whose payload is not identical
to the original.

**Q3 — What is the baseline, and was it measured in the same run?**
**The plain v0.1 file, and it was measured in the same run.** Every image was
encoded with the ordinary `vpng encode` in the same driver pass, and its
bytes and SHA-256 recorded. The plain total is **453,767,460** bytes — the
published stable baseline's, to the byte — and the 16K photograph's plain
digest is the published `d029a7eb…`.

**Q4 — Do the GPU files change the pixels?**
**No.** The **lossless** payload of every one of the **750** GPU files
reconstructs the picture byte for byte. The GPU payload is a different
picture, and it is deliberately so.

**Q5 — What does the GPU payload buy?**
A picture in the block format a GPU samples as a texture, with no
decompression and no per-pixel work on the GPU. It costs **251,716,072**
bytes over the corpus — **55.472482%** of the plain total — and it buys a
picture that is **close** to the original: **37.2237 dB** pooled PSNR, a
mean absolute error of **1.456997** levels, and a worst single difference of
**170** levels. It is a **preview**, not a copy, and the file says so.

**Q6 — How fast is it?**
**This milestone does not claim a speed.** §2.6 says why: each case is a
single run, the plain encode always ran first, and the GPU encode's total
came out at 0.8617× the plain one in that order — an artefact of the order,
not a measurement of anything. The encoder's own byte-identity across worker
counts (1, 2, 4, 8) and across the batch and streaming paths **is** measured
and pinned.

**Q7 — What does it cost?**
**251,716,072 bytes over the corpus — 55.472482% of the plain total**, and
**17.341392%** of the canonical pixels. The **worst case is a small file**: a
25,977-byte UI screenshot spends **1451.79197%** of its bytes on the GPU
payload, and the **pixel-art** and **UI** categories spend **265.562284%**
and **274.325592%** as a share of their own files. The GPU payload also
pushes **three** files over the 10 MB ceiling that were under it before.

**Q8 — Does the GPU work move the default path's bytes or version?**
**No.** The default path writes no GPU table. The corpus total is the
published baseline's **453,767,460** bytes, every plain file reports minor
version 1, and the 16K photograph's plain digest is the published one. The
GPU files are minor version **5**.

**Q9 — Is the format still binary-compatible?**
**Yes, and the milestone is an extension, not a redesign.** The 40-byte
directory entry keeps its exact layout. The header grows from 64 to 72 bytes
**through the `header_size` field that exists for that purpose**, and the
header checksum rule becomes "bytes `0..52` and `56..header_size`", which for
a 64-byte header is exactly the rule v0.1–v0.4 use. So no existing file's
checksum changes, no existing byte moves, and an old file still reads
unchanged. A v0.5 file is rejected by an older reader for the ordinary
reason: the minor version is newer than the reader knows.

**Q10 — Are the research codecs still out of reach?**
**Yes.** The shared metadata validation refuses a directory entry that names
an unimplemented codec, and the three promoted codecs may only be named by a
file whose minor version says so. The GPU work added **two** codec ids — 7
(BC1) and 8 (BC4) — and they are **not** in `IMPLEMENTED_CODEC_IDS`: a
directory entry that names one is refused with `UnsupportedFeature`, and the
cost-based selector never picks one. The research ids `0x80..=0xFF` still
cannot reach a file.

**Q11 — What is in the memory figures?**
**Process peak working sets, and §2.7 says exactly what is in them**: the
operating system, the loader, the binary, the Rust heap, every buffer the
command allocated and the file it read — nothing is excluded. The figures are
for the 16K photograph and they are **not** the 64 MB localized-editing
target: these commands encode, check and decode **whole pictures**, so they
hold whole pictures. The editing engine's own target is unchanged and still
met, and the engine refuses a GPU file outright.

**Q12 — What is not measured, and what remains untested?**
The limits are in §6. The four that matter most: **no GPU was involved** —
the block codecs are CPU code, and nothing here measures a GPU's texture
sampling; **no speed was measured** at all; the **editing engine refuses a
GPU file** rather than editing it, so the GPU payload's edit path does not
exist yet; and **BC7 and ASTC are not implemented** — the ids are free for
them.

---

## 5. Reproducing the evidence

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# One: the format's own tests.
cargo test -p vpng-container -p vpng-gpu -p vpng-decoder -p vpng-encoder
cargo test -p vpng-testkit --test v5_formats
cargo test -p vpng-encoder --test gpu_identity
cargo test -p vpng-editing --test v5_gpu
cargo test -p vpng-cli --test v5_features

# Two: the WebAssembly surface.
cargo build --release --target wasm32-unknown-unknown -p vpng-wasm
cargo build --release -p vpng-cli
node --test wasm/test/vpng.test.mjs
node wasm/check_exports.mjs

# Three: the corpus.
cargo build --release -p vpng-cli
python benchmarks\corpus\run_gpu_corpus.py --jobs 1 --workers 4

# Four: the memory figures.
python benchmarks\gpu\measure_gpu_memory.py

# Five: the whole Rust suite, and the check that every quoted figure matches
# the raw data.
cargo test --workspace --all-targets
python benchmarks/verify_reports.py
```

---

## 6. What this does not claim

* **No claim that the GPU payload is exact.** It is **lossy** by
  construction. The worst image in the corpus is at **13.3972 dB**, and the
  worst single difference anywhere is **170** levels of 255. A picture's GPU
  payload can be badly wrong, and the corpus contains pictures where it is.
* **No claim that the GPU payload is cheap.** It is **55.472482%** of the
  corpus's bytes, it more than triples a UI screenshot, and on the corpus's
  smallest file it is **1451.79197%**.
* **No claim about a GPU.** There is **no GPU in this measurement**. The
  block codecs are CPU code; what is measured is the bytes and the fidelity,
  not a texture's sampling speed.
* **No claim about speed.** §2.6: the timing figures are driver totals from
  single runs in a fixed order, and they do not support a speed claim in
  either direction.
* **No claim that the editing engine can edit a GPU file.** It cannot: it
  **refuses** such a file with a message that says "GPU table". A refusal is
  not a capability.
* **No claim that BC7 or ASTC are implemented.** They are not. The block
  codecs here are **BC1** and **BC4**, which the roadmap's own text named as
  the honest first pair.
* **No claim that VPNG now beats PNG.** The P5 report's unfavourable
  comparison with PNG, WebP and JPEG XL still stands unchanged.

---

## 7. The acceptance gates

| # | Gate | Result | Evidence |
|---|---|---|---|
| 1 | The corpus actually ran, at its full size | **PASS** | 750 of 750 images, two files each; `benchmarks/p13_gpu_results.json` |
| 2 | The lossless payload is untouched | **PASS** | 750 of 750 GPU files reconstruct the picture byte for byte, and the plain total is the published baseline's **453,767,460** |
| 3 | The GPU payload is lossy, and says so | **PASS** | 746 of 750 payloads differ from the original; the pooled PSNR is **37.2237 dB** and the worst difference is **170** levels |
| 4 | Every file passes the container's own check | **PASS** | 0 failures across the 750 GPU files |
| 5 | The GPU table's cost is measured, not estimated | **PASS** | **251,716,072 bytes** over the corpus, **55.472482%**; the payloads' **251,513,192** and the tables' **196,880** are counted separately |
| 6 | The accounting in the file is exact | **PASS** | the file is the plain file plus the GPU table, the GPU payloads and eight header bytes; 750 of 750 files add up |
| 7 | A directory entry never names a GPU block codec | **PASS** | the reader refuses such an entry with `UnsupportedFeature`, and the writer never produces one |
| 8 | The output is deterministic | **PASS** | 1, 2, 4 and 8 workers write the same bytes; the batch encoder and the streaming writer agree |
| 9 | The default path's bytes and version do not move | **PASS** | the corpus total is the published baseline's **453,767,460**; every plain file is minor version 1 |
| 10 | The binary layout is unchanged for every older file | **PASS** | the header grows only through `header_size`, and the checksum rule is unchanged for a 64-byte header |
| 11 | The editing engine refuses a GPU file | **PASS** | `UnsupportedFeature`, with a message that says "GPU table", in memory and on disk |
| 12 | Every quoted figure is bound to the raw data | **PASS** | `check_gpu()` in `benchmarks/verify_reports.py` |

---

## 8. What is next

The GPU payload is in, measured and documented. What it leaves open:

* **Editing a GPU file.** The engine refuses one today. Carrying the GPU
  table and its per-tile records through a commit is the next piece of work
  in the editing engine.
* **BC7 and ASTC.** The ids are free for them. BC7's mode 6 needs a subset
  search over endpoint candidates, which is a research project of its own.
* **A GPU-side consumer.** The payload is in the format a GPU speaks, but
  nothing in this repository uploads it to a GPU.
* **Then** the graphical UI (P11), and **last** the v0.2 → v0.23 release
  cadence and the v1.0 freeze.
