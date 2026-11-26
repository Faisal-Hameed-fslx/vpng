# ADR-0016: The HTTP Range source and the byte-source abstraction

**Status:** accepted
**Date:** 2026-11-26
**Related:** ADR-0005 (grid derivation), ADR-0014 (levels and partitions),
ADR-0015 (the v0.3 version rule)
**Question:** The roadmap asks for "HTTP Range-request cloud streaming": a
reader that fetches a container's header, directory and tiles from an object
store without downloading the whole file. The format already supports this,
so **no format change is needed**. The question is how the reader is
structured, what the client is allowed to accept, and what happens when a
server does not behave.

## Decision

### 1. The byte source is a trait, and the reader is generic over it

`vpng-source` gains a `ByteSource` trait — `length()`, `read_at(offset,
len)`, `fork()`, `describe()` — and `RangeReader<S>` is generic over it.
`FileSource` is the local implementation; `HttpSource` is the network one.
`TileReader` remains `RangeReader<FileSource>`, and `vpng-view`'s
`FileDecoder` / `FileDecoderFactory` remain type aliases of the generalised
`SourceDecoder<S>` / `SourceDecoderFactory<S>`, so **the existing public API
does not move**.

The consequence is the point of the decision: there is **one** tile-reading
path, not two. The HTTP reader runs the same `decode_tile` and
`decode_region` the local reader runs.

### 2. The container validation and the tile payload decoding are one function each

The rules that decide which containers are acceptable — the dimensions, the
declared size against the bytes present, the directory's length and flags,
the promoted codecs against the minor version, the payload ranges, the tile
sizes — were written twice: once in the in-memory decoder and once in the
streaming reader. They are now **`vpng_decoder::parse_container_parts`**, and
both readers call it. The tile payload's checksum, its decoding and its
length check are now **`vpng_decoder::decode_tile_payload`**, and the
in-memory decoder, the streaming reader and the range reader all call it.

A container one reader accepts is therefore one every reader accepts, by
construction rather than by agreement. The WebAssembly package will call the
same two functions.

### 3. A server that does not honour `Range` is refused, not accommodated

* A `200 OK` answer to a range request is **refused** with
  `UnsupportedFeature`. Accepting it would silently turn a random-access
  reader into a whole-file download while reporting itself as a range read.
* A `206` whose `Content-Range` disagrees with the request is **refused**
  with `CorruptFile`.
* A short body is **refused** with `CorruptFile`. The reader is never handed
  half a payload and left to guess.
* A range past the end of the object is **refused** with `InvalidArgument`.
* A range larger than `max_range_bytes` (8 MiB by default) is fetched in
  several requests, so no single response grows without bound.

### 4. `open()` proves the server honours `Range` before the caller spends a request

`HEAD` is tried first, because it costs no body bytes, but the answer is
never taken on trust: a **one-byte range request** follows, and it both
proves the server honours `Range` and carries the authoritative length in
its `Content-Range`. A server that refuses `HEAD` with 405 or 501 is not an
error — the probe answers the same question. A `HEAD` length that disagrees
with the probe is `CorruptFile`: the object changed while it was being
opened.

### 5. The client is blocking, and the TLS provider is `rustls`

`ureq` is pinned to **exactly 3.2.1** with `default-features = false` and
the `rustls` and `gzip` features.

* **Blocking.** The reader is a plain synchronous API, so no async runtime
  enters the workspace. The viewer's own worker pool already provides the
  concurrency that matters.
* **Pinned exactly.** 3.3 and later require rustc 1.85; this workspace's
  MSRV is 1.74.
* **rustls, not a system TLS library.** A reader that needs OpenSSL
  installed is a reader that does not run everywhere.

### 6. No format change

The format already allows this: every tile payload has its own offset and
length in the directory, so a reader that can ask for byte ranges can fetch
the header, the directory and then exactly the payloads it needs. The
64-byte header and the 40-byte directory entry are untouched, and the
reader writes nothing at all.

## Rationale

* **The trait is the design.** The alternative — a second reader for HTTP —
  would have been two implementations of "which payloads does this region
  need", and they would have drifted. The cost of the trait is one generic
  parameter; the benefit is that the HTTP path is the local path.
* **Refusing is better than accommodating.** A reader that accepts a
  `200 OK` for a range request will appear to work and will download the
  whole file. The failure is invisible in the output and enormous in the
  bill. A refusal is loud, early, and cheap.
* **The probe is not optional.** Without it, the first sign of a
  range-ignorant server is a whole-file download that has already happened.
* **One validation function.** Two copies of the acceptance rules are two
  chances to disagree about which files are valid — and the disagreement
  would be a file one reader accepts and the other rejects, which is a
  security difference, not a convenience one.
* **No format change is the strongest possible compatibility statement.**
  Every file written before this decision is readable by the new reader, and
  the new reader reads exactly the files the old one read.

## Consequences

* The workspace gains one dependency (`ureq` and its tree). The engine
  crates — `vpng-core`, `vpng-codecs`, `vpng-tiling` — gain none.
* `vpng-source`'s reader is generic, so a few of its methods move onto
  `impl<S: ByteSource>`. **The public names and the call sites do not move.**
* The HTTP reader is **sequential**: one range at a time, in order. There is
  no retry, no connection reuse and no concurrency. A viewport that wants
  twenty tiles at once should ask for them at once — that is the next
  improvement, not this one.
* The source counts its own requests and body bytes, and the test server
  counts its own. The two are independent, so they cross-check each other:
  over the whole 750-image corpus the client's byte count and the server's
  are **exactly equal**.
* A CDN that rewrites `Content-Range` will be refused. That is the intended
  behaviour: the reader's byte accounting would otherwise be wrong, and the
  accounting is the point.

## Evidence

`docs/P10_HTTP_REPORT.md` — the twelve questions, the acceptance gates, and
the measurement over the whole 750-image corpus: **750 of 750** HTTP reads
return exactly the local pixels, the header and the directory cost
**0.0901%** of the corpus, and a 1024×1024 viewport of the 16K photograph
costs **0.8369%** of that file. Nine tests in `crates/vpng-http` and four in
`crates/vpng-cli` pin the refusals against a hand-written HTTP/1.1 server.
