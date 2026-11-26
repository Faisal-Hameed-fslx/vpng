# P10 — HTTP Range streaming

**Date:** 2026-11-26
**Deliverables:** the `ByteSource` trait and the `FileSource` in
`crates/vpng-source/src/source.rs`; the generic `RangeReader<S>` in
`crates/vpng-source/src/reader.rs`; the new crate `crates/vpng-http`
(`HttpSource`, `HttpSourceOptions`, `HttpSourceInfo`, `HttpReader`) with its
nine tests and its test-only HTTP/1.1 server; the generalised
`SourceDecoder<S>` / `SourceDecoderFactory<S>` in `crates/vpng-view`; the two
new command line commands `vpng http-info` and `vpng http-decode-region` with
their four CLI tests; the measurement driver
`benchmarks/http/run_http_bench.py`; the raw result file
`benchmarks/p10_http_results.json`; and the decision record
`docs/decisions/ADR-0016-http-range-source.md`.
**Status:** the case **ran** — it did not have to be reported BLOCKED. Every
figure below is read out of the raw JSON, and `check_http()` in
`benchmarks/verify_reports.py` binds the quoted figures to it.

---

## 1. What this is

Phase C step 2 of the agreed plan: **read a container over the network
without downloading it.**

The format already supported this and needed no change. Every tile payload
has its own offset and length in the directory, so a reader that can ask for
byte ranges can fetch the 64-byte header, the directory, and then exactly the
payloads it needs. **The binary layout did not move. The header and the
directory entry are byte-for-byte the layouts of v0.1.**

What was missing was the *plumbing*: the reader was hard-wired to a file on
disk. This milestone makes the byte source an abstraction and puts a second
implementation of it behind HTTP.

### 1.1 One reader, two sources

`vpng-source` now has a `ByteSource` trait — `length()`, `read_at(offset,
len)`, `fork()`, `describe()` — and `RangeReader<S>` is generic over it.
`FileSource` is the local implementation; `HttpSource` is the network one.
`TileReader` remains `RangeReader<FileSource>`, so **every existing caller
keeps its exact type**.

The important consequence is that there is now **one** tile-reading path, not
two. `RangeReader::decode_tile` and `RangeReader::decode_region` are the
functions the HTTP reader runs; the in-memory decoder and the streaming
reader now also share one `vpng_decoder::decode_tile_payload` helper, so all
three refuse the same corrupt payloads and produce the same pixels.

The **validation** is shared the same way. The rules that decide which
containers are acceptable — the dimensions, the declared size against the
bytes present, the directory's length and flags, the promoted codecs against
the minor version, the payload ranges, the tile sizes — used to be written
twice, once in the in-memory decoder and once in the reader. They are now
**one** function, `vpng_decoder::parse_container_parts`, and both readers
call it. A container the range reader accepts is one the in-memory decoder
accepts, by construction rather than by agreement. The WebAssembly package
will call the same function, so a browser and the command line cannot
disagree about which files are valid.

`vpng-view` was generalised the same way: `SourceDecoder<S>` and
`SourceDecoderFactory<S>` are generic, and `FileDecoder` / `FileDecoderFactory`
remain as type aliases. **The existing public API is unchanged.**

### 1.2 The rules the source enforces

These are the rules that make the "it fetched only what it needed" claim
meaningful rather than a matter of trust:

| Rule | What happens |
|---|---|
| A `200 OK` answer to a range request | **refused** with `UnsupportedFeature`: "the server does not honour HTTP Range requests" |
| A `206` whose `Content-Range` disagrees with the request | **refused** with `CorruptFile` |
| A short body | **refused** with `CorruptFile` — never handed back as a short buffer |
| A range past the end of the object | **refused** with `InvalidArgument` |
| A range larger than `max_range_bytes` (8 MiB by default) | fetched in several requests, so no response grows without bound |
| `HEAD` refused with 405 or 501 | **not an error**: the one-byte range probe that follows answers the same question |
| `HEAD` and the range probe disagreeing about the length | **refused** with `CorruptFile`: the object changed while it was being opened |

`open()` **always** sends a one-byte range request, even when `HEAD`
succeeded, because that request is what proves the server honours `Range`.
The source refuses to continue before the caller has spent a request on a
whole-object download.

### 1.3 The accounting

The source counts its own requests and body bytes, and the test server counts
its own. The two are independent, so they cross-check each other: over the
whole corpus, the server's byte count and the source's byte count are
**exactly equal** — 234,830,932 bytes each. A reader that quietly fetched
more than it reported could not pass that comparison.

---

## 2. How it was measured

### 2.1 The host

`stable-x86_64-pc-windows-gnu`, rustc 1.100.0, an Intel Core i5-1235U with
**10 physical cores and 12 logical processors**, 7.7 GB of physical memory.
CPython 3.13.0. The CLI binary is the **release** profile. Tile size 256,
**4 encoder workers**, one image at a time.

### 2.2 The corpus

The same **750-image corpus** the P5, P6, P7, P8 and P9 reports used:
**300** photographs, **100** pixel-art images, **100** scans, **150**
textures and **100** UI screenshots. The canonical pixels come from the same
cache, so the bytes the HTTP reader is given are the same bytes the other
reports encoded.

### 2.3 The method

For each image:

1. The image is encoded locally with the real `vpng` binary, from the cached
   canonical pixels, and the file's size and SHA-256 are recorded.
2. The file is served from a **local HTTP/1.1 server that honours `Range`**,
   written in the driver itself, which counts its own requests and bytes.
3. `vpng http-info <url>` fetches the header and the directory and **nothing
   else**. The bytes and the requests are recorded.
4. `vpng http-decode-region <url> <out>` decodes a **1024×1024** viewport at
   the origin over HTTP. The bytes, the requests and the wall clock are
   recorded.
5. The **same** region is decoded from the **local** file and the two results
   are compared — by the canonical pixel digest *and* by the SHA-256 of the
   two PNG files, so a reader that fetched the wrong bytes fails the run
   instead of being reported as a win.

### 2.4 The percentile rules

The same rules the other reports use: the median is the middle sample, or the
mean of the two middle samples when the count is even; the p95 is the
**nearest-rank** percentile; the standard deviation is the **sample** one,
with n − 1 in the denominator.

### 2.5 What is in the memory figures

**This measurement reports bytes over the wire, not process memory.** There
is no memory target in this milestone, and no memory figure is claimed.

* **Included** in the byte counts: every HTTP response **body** byte the
  source received, including the one-byte length probe, the header, the
  directory and every tile payload. The HTTP response *headers* are not
  counted, because they are not file bytes.
* **Excluded**: the request and response headers, the socket buffers, the
  driver's own working set, the CLI process's working set, the decoded
  pixels, and the file on disk.

The raw JSON carries the same statement under `settings.memory_scope`.

---

## 3. What was measured

### 3.1 The headline

| | |
|---|---|
| Images measured | **750** of 750 |
| Regions identical to the local read | **750** of 750 |
| PNG files identical to the local read | **750** of 750 |
| Total object size | **453,767,460** bytes |
| Bytes fetched to read the header and the directory | **408,760** (**0.0901%** of the corpus) |
| Bytes fetched to decode a 1024×1024 viewport | **234,420,672** (**51.661%** of the corpus) |
| Server bytes sent | 234,830,932 |
| Source bytes received | 234,830,932 — **exactly equal** |
| HTTP requests | 10,518 (4 per command: `HEAD`, the length probe, the header, the directory) |
| Range requests | 9,018 |

The total object size, **453,767,460** bytes, is the published stable
baseline's, which is a useful cross-check: the HTTP measurement encoded the
corpus with the same bytes the P5 and P9 reports did.

### 3.2 The big picture

The 16K photograph — **124,197,035** bytes — is the clearest case:

| | bytes | requests | share of the object |
|---|---|---|---|
| Header and directory | 81,668 | 4 | **0.0658%** |
| A 1024×1024 viewport | **1,039,409** | 20 | **0.8369%** |
| The whole object | 124,197,035 | — | 100% |

So a viewer that wants a 1024×1024 window of a 16K photograph downloads
**0.84%** of the file. That is the claim the range reader makes, and it is
measured.

The photograph was also **faster over HTTP than from the disk** in this run
(162.014 ms against 222.366 ms), because only 16 tiles were decoded instead
of the whole picture's 2,040. That is a statement about *this* comparison on
*this* host with a loopback server, not about the network — see §6.

### 3.3 The corpus as a whole — the unfavourable part

The **51.661%** headline is **not** a good number, and it is kept as it is:
**592 of the 750 images are smaller than the 1024×1024 viewport**, so for
them the whole file is the viewport and the reader fetches everything. Those
592 images account for 151,851,134 of the 234,420,672 bytes.

The subset where the range reader can actually save something is the **158
images larger than the viewport**:

| | bytes | share |
|---|---|---|
| Their objects | 301,916,326 | 100% |
| Their viewport fetches | **82,569,538** | **27.3485%** |

Per category:

| Category | Images | Object bytes | Header + directory | Viewport fetch | Viewport share |
|---|---|---|---|---|---|
| photos | 300 | 234,827,986 | 158,960 | 104,628,889 | 44.5555% |
| pixel_art | 100 | 365,091 | 11,200 | 365,091 | 100.0000% |
| scans | 100 | 67,871,353 | 66,040 | 56,808,900 | 83.7009% |
| textures | 150 | 133,872,854 | 93,040 | 60,736,581 | 45.3689% |
| ui | 100 | 16,830,176 | 79,520 | 11,881,211 | 70.5947% |

The pixel-art row is the honest worst case: every pixel-art image in the
corpus is smaller than the viewport, so the reader fetches 100% of it. The
**header-and-directory** share is the part that is always cheap, and it is
**0.0901%** of the corpus overall.

### 3.4 The timings

| | median | p95 | mean | stdev | min | max |
|---|---|---|---|---|---|---|
| Over HTTP | 51.8445 ms | 266.544 ms | 87.251991 ms | 94.212396 ms | 12.189 ms | 957.197 ms |
| From the local file | 41.7015 ms | 203.297 ms | 65.734116 ms | 68.892275 ms | 8.737 ms | 610.457 ms |

**Over HTTP was slower than the local read**, by about 24% at the median and
by about 31% at the p95. That is the loopback server's cost — four HTTP
round trips per command plus one per tile — and it is reported rather than
hidden. On a real network the round trips would cost far more; this report
does not measure that, because this host cannot measure it honestly.

### 3.5 The refusals

The nine tests in `crates/vpng-http/src/tests.rs` pin the refusals against a
hand-written server with five behaviours: `Ranges`, `IgnoreRanges`,
`WrongRange`, `ShortBody` and `NoHead`.

| Test | What it pins |
|---|---|
| `the_reader_fetches_only_the_bytes_it_needs` | the byte count is the bytes the tiles need |
| `the_pixels_over_http_are_the_pixels_on_disk` | the pixels are exact |
| `a_server_that_ignores_range_is_refused` | a `200 OK` is refused, not accepted |
| `a_server_that_answers_the_wrong_range_is_refused` | a `Content-Range` mismatch is refused |
| `a_truncated_body_is_refused` | a short body is an error, not a short buffer |
| `a_server_that_refuses_head_still_works` | a 405 on `HEAD` is not fatal |
| `a_range_beyond_the_object_is_refused` | an out-of-range read is refused |
| `a_large_range_is_fetched_in_several_requests` | the 8 MiB ceiling is honoured |
| `the_reader_reports_what_it_read` | the accounting is what it claims |

Four more tests in `crates/vpng-cli/tests/http_commands.rs` drive the two
commands themselves against the same server: `http-info` reports the
container **without** fetching the payloads, `http-decode-region` returns
the same region — and writes the same PNG — as the local `decode-region`, and
a server that ignores `Range` or answers the wrong range is refused with the
right exit code.

---

## 4. The twelve questions

**Q1 — Was the corpus actually run, or blocked?**
**Run, not blocked.** All **750** images were encoded locally, served over a
local HTTP server, read back over `Range` requests, and compared with the
local read. The raw file is `benchmarks/p10_http_results.json`.

**Q2 — How many times was each case run, and what do the percentiles mean?**
**Once per image per command, over 750 images.** The corpus is the
repetition. The median is the middle sample, or the mean of the two middle
samples when the count is even. The p95 is the **nearest-rank** percentile.
The standard deviation is the sample one, with n − 1 in the denominator.

**Q3 — What is the baseline, and was it measured in the same run?**
**The local file, and it was measured in the same run.** Every image was
decoded twice inside the same driver pass — once over HTTP and once from the
local file — and the two canonical digests and the two PNG digests were
compared there and then. The P5 corpus baseline was checked too: the total
object size is **453,767,460** bytes, the published stable baseline's.

**Q4 — Do the HTTP-read files change the pixels?**
**No.** The canonical pixel digest of the region read over HTTP equals the
digest of the same region read from the local file on **750 of 750** images,
and the two PNG files are byte-identical on **750 of 750**. The
reconstruction is **exact**.

**Q5 — What does the range reader buy?**
**It buys the ability to open a container without downloading it.** The
header and the directory cost **0.0901%** of the corpus; a 1024×1024
viewport of the 16K photograph costs **0.8369%** of that file. For the 158
images larger than the viewport it cuts the transfer from 301,916,326 bytes
to 82,569,538.

**Q6 — How fast is it?**
**Slower than the local read on this host.** The HTTP median is 51.8445 ms
against the local 41.7015 ms. The 16K photograph is the one case where HTTP
won, and it won because it decoded 16 tiles instead of 2,040, not because
HTTP is fast. The loopback server's four round trips per command are the
cost.

**Q7 — What does it cost?**
**One request per tile, plus four per command.** Over the corpus: 10,518
requests, of which 9,018 carried a range, for 4,518 payload ranges. The
request count is the price of the byte saving, and on a high-latency link
that price is real.

**Q8 — Does the range reader move the default path's bytes or version?**
**No.** The files it reads are the published stable ones — the corpus total
is **453,767,460** bytes and every file reports minor version **1**. This
milestone added **no format revision**: the header and the directory entry
keep their exact layouts, and the reader writes nothing at all.

**Q9 — Is the format still binary-compatible?**
**Yes, and the milestone did not touch it.** No byte of the 64-byte header
or the 40-byte directory entry moved. The range reader parses the same
container the local reader parses, through the same
`vpng_decoder::parse_container_parts` validation.

**Q10 — Are the research codecs still out of reach?**
**Yes.** The shared metadata validation refuses a directory entry that names
an unimplemented codec, and the three promoted codecs may only be named by a
file whose minor version says so. The HTTP path uses that same validation,
so it accepts exactly the files the in-memory decoder accepts.

**Q11 — What is in the memory figures?**
**There are none in this report.** §2.5 says exactly what the byte counts
include and exclude: response **body** bytes only; headers, sockets,
decoded pixels and process working sets are excluded. This milestone did not
measure process memory and makes no memory claim.

**Q12 — What is not measured, and what remains untested?**
The limits are in §6. The four that matter most: the server was a **loopback
HTTP/1.1 server written for this measurement**, so the figures say nothing
about a real network, a CDN, TLS, a proxy or a congested link; only **one
viewport size** (1024×1024) and **one position** (the origin) were
measured; **no compression or caching** was negotiated, so the byte counts
are the *uncompressed* file bytes, not the wire bytes a real deployment
would see; and the reader has **no retry, no connection reuse and no
concurrency** — it fetches one range at a time, in order.

---

## 5. Reproducing the evidence

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# One: the crate's own tests, against the hand-written server.
cargo test -p vpng-http

# Two: the corpus comparison, against the release CLI.
cargo build --release -p vpng-cli
python benchmarks\http\run_http_bench.py --sample 750

# Three: the whole Rust suite, and the check that every quoted figure matches
# the raw data.
cargo test --workspace --all-targets
python benchmarks/verify_reports.py
```

---

## 6. What this does not claim

* **No claim that HTTP is fast.** It was **slower** than the local read at
  the median and at the p95. The loopback server is a measurement
  instrument, not a network.
* **No claim that the whole corpus saves bytes.** The 51.661% figure is
  dominated by the 592 images that are smaller than the viewport. The
  honest statement is the subset: the **158** images larger than the
  viewport fetched **27.3485%** of their bytes.
* **No claim about the wire bytes of a real deployment.** The counts are
  uncompressed file bytes. A server that gzips the response would send less;
  one that does not, more.
* **No claim about concurrency, retries or resilience.** The reader fetches
  one range at a time. A flaky link fails the command; it does not retry.
* **No claim that the format changed.** It did not. This milestone is a
  reader, not a revision.
* **No claim that VPNG now beats PNG.** The P5 report's unfavourable
  comparison with PNG, WebP and JPEG XL still stands unchanged.

---

## 7. The acceptance gates

| # | Gate | Result | Evidence |
|---|---|---|---|
| 1 | The corpus actually ran, at its full size | **PASS** | 750 of 750 images, two reads each; `benchmarks/p10_http_results.json` |
| 2 | The HTTP read returns the same pixels as the local read | **PASS** | 750 of 750 canonical digests match |
| 3 | The HTTP read writes the same picture as the local read | **PASS** | 750 of 750 PNG digests match |
| 4 | The reader fetches only the bytes it needs | **PASS** | the 16K photograph's viewport costs **1,039,409** bytes of **124,197,035** — **0.8369%** |
| 5 | The client and the server agree on the byte count | **PASS** | 234,830,932 bytes on both sides, exactly |
| 6 | A server that ignores `Range` is refused | **PASS** | `a_server_that_ignores_range_is_refused`: `UnsupportedFeature`, not a silent whole-file download |
| 7 | A wrong, short or out-of-range answer is refused | **PASS** | three more tests: `WrongRange`, `ShortBody` and a range past the end |
| 8 | There is one reader, not two | **PASS** | `RangeReader<S>` is generic over `ByteSource`; `TileReader` is still `RangeReader<FileSource>`; the container validation is one shared `vpng_decoder::parse_container_parts` and the tile payload path is one shared `vpng_decoder::decode_tile_payload` |
| 9 | The format is unchanged | **PASS** | no format revision; the 64-byte header and the 40-byte entry are untouched; every file read is minor version 1 |
| 10 | The default path's bytes do not move | **PASS** | the corpus total is the published baseline's **453,767,460** |
| 11 | Every quoted figure is bound to the raw data | **PASS** | `check_http()` in `benchmarks/verify_reports.py` |
| 12 | The binary layout is unchanged | **PASS** | the milestone adds a source and a reader; it writes no container at all |

---

## 8. What is next

The HTTP path is in, measured and documented. What it leaves open:

* **The WebAssembly package**, which is the other half of "the browser can
  read this". The shared `decode_tile_payload` and the public
  `ContainerMeta::parse` are already in place for it.
* **Concurrency, retries and connection reuse** in the HTTP source. The
  reader is deliberately sequential today; a view that wants a viewport's
  twenty tiles at once should ask for them at once.
* **Conditional requests.** The source re-reads the header and the directory
  on every open. A validator (`ETag` / `If-Range`) would let a repeat open
  cost nothing.
* **Then** layers and masks (which is a format revision), GPU-ready payloads
  and the graphical UI; and **last** the v0.2 → v0.23 release cadence and the
  v1.0 freeze.
