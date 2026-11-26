# ADR-0017: The WebAssembly package's shape and its ABI

**Status:** accepted
**Date:** 2026-11-26
**Related:** ADR-0005 (grid derivation), ADR-0014 (levels and partitions),
ADR-0015 (the v0.3 version rule), ADR-0016 (the HTTP Range source)
**Question:** The roadmap asks for "a `wasm32` build of the decoder, the
viewer and the editor". The decoder is the part everything else needs, so
that is what this decision covers. The questions are: what goes into the
module, what the boundary with JavaScript looks like, and what must be true
before the package is called finished.

## Decision

### 1. The module is a decoder, and it is the *same* decoder

`crates/vpng-wasm` contains no codec, no predictor, no entropy coder and no
container parser of its own. It calls `vpng-decoder`, `vpng-codecs` and
`vpng-source` — the same crates the command line calls — and it validates a
container through `vpng_decoder::parse_container_parts`, the one shared
implementation of the acceptance rules.

The consequence is the whole point: **the browser and the command line
cannot disagree about what a file means**, because there is only one
implementation to disagree with. The evidence is measured, not asserted:
the module's canonical digest for a region is the command line's canonical
digest for the same region, on **750 of 750** corpus images, on both the
whole-file path and the range path.

### 2. The ABI is a raw `extern "C"`, not `wasm-bindgen`

The module exports plain functions with `i32`, `i64` and `u32` arguments and
returns. There is no `wasm-bindgen`, no generated glue and no build-time code
generation.

* **No new dependency.** The workspace gains none for this package.
* **No toolchain.** `cargo build --target wasm32-unknown-unknown` is enough.
  `wasm-pack` is not installed, is not needed, and would not be needed by a
  user either.
* **Any runtime can call it.** A browser, Node, Deno, a wasm runtime, or
  another language's WebAssembly bindings. The wrapper in `wasm/vpng.js` is
  optional sugar, not a requirement.

`wasm-bindgen` was reachable and was considered. It is the conventional
choice, and it would have produced a nicer JavaScript API. It was not
chosen because it adds a dependency, a code-generation step and a version
constraint to a package whose value is that it is the *same* engine — and
the conventional API is not what the measurement needs to be about.

### 3. Two ways to open a container, and both are needed

* `vpng_open` takes the **whole container** in the module's memory. The
  ordinary case: a browser that has the file, or one that has downloaded it.
* `vpng_open_parts` takes the **header and the directory** alone. The caller
  then asks for the payload of the tile it wants with `vpng_tile_offset` and
  `vpng_tile_length`, fetches that range, and hands the bytes back with
  `vpng_decode_payload`.

The second is the path a container in an object store takes, and it is the
same shape as the HTTP Range reader in ADR-0016: the container's layout is
known after two ranges, and after that the reader fetches exactly the
payloads it needs. The module's `vpng_region_tiles` says which tiles a
rectangle touches, so the caller never has to work out the tiling itself.

The payload's CRC32C is verified against the directory before the codec sees
it, in both paths, by the same `vpng_decoder::decode_tile_payload` the
native reader uses.

### 4. Every pointer is checked, and every error is typed

A pointer from JavaScript is an offset into the module's linear memory, and
a pointer that JavaScript made up would be undefined behaviour. Every pointer
is therefore checked against the module's **real** memory size — the size the
`memory.size` instruction reports, converted to bytes — before it is used. A
bad pointer is an error, not a crash.

The return convention is the command line's: zero is success and a negative
value is the negative of the error's exit code, so `-4` is a corrupt file and
`-3` is an unsupported feature. The message is fetched separately with
`vpng_error_length` and `vpng_error_copy`. A JavaScript caller therefore
sees the same error taxonomy the command line does, and a browser's error
handling can be written against the documented codes.

Byte lengths and offsets are `i64`, because a 16K RGBA16 region is 2 GiB and
a file may be larger than that. An `i32` would have been wrong.

### 5. The wrapper owns the memory, and the caller never holds a view into it

The module's memory **grows** when it allocates, and a grown WebAssembly
memory detaches every typed array the caller made over it. The wrapper
therefore re-creates its view after every call that can allocate, and copies
every result out before returning it. A caller that kept a view would find
it detached, and the wrapper's contract — "every returned buffer is a copy" —
removes the question.

### 6. No format change

Not one byte of the 64-byte header or the 40-byte directory entry moved. The
module reads what is there. The preview pyramid, the adaptive partition and
the promoted codecs are all visible to the module **because they are already
in the format**, not because the format was changed for the browser.

## Rationale

* **One implementation is worth more than a good API.** The risk in a
  browser decoder is not that it is awkward; it is that it quietly accepts a
  file the native decoder rejects, or produces pixels that differ in the
  last bit. Sharing the code removes the risk by construction, and the
  measurement confirms it rather than assuming it.
* **A raw ABI is the smaller commitment.** `wasm-bindgen` is a good tool,
  but it makes the package's users depend on it. A raw ABI is callable from
  anything, and the wrapper is a hundred lines of ordinary JavaScript.
* **The range path is not a second implementation.** It is the same
  validation, the same tile decoding and the same checksum, reached through
  a different door. That is why `openParts` and `open` cannot disagree about
  a file.
* **Checking every pointer is cheap.** The cost is one comparison against
  the memory size. The cost of *not* checking it is an arbitrary read in the
  caller's address space.

## Consequences

* The workspace gains **no dependency** for this package.
* The crate is a `cdylib` with no `rlib`: nothing in the workspace links
  against it. Its logic lives in `registry.rs`, which is ordinary Rust and
  is unit-tested on the host, so the host test suite covers the parts that
  the WebAssembly build cannot.
* The module is **single-threaded**. WebAssembly threads are not enabled;
  the registry and the error message are behind a mutex anyway, so the
  state is well-defined even if a future runtime instantiates it on more
  than one thread.
* The module **does not encode** and **does not edit**. Those are separate
  pieces of work, and a decoder is what a viewer needs first. The package's
  README says so plainly.
* The package's **speed is not claimed**. The measurement compares the
  module's decode time with the native reader's over the same corpus and
  reports both; it does not assert that one is faster than the other.

## Evidence

`docs/P11_WASM_REPORT.md` — the twelve questions, the acceptance gates, and
the measurement over the whole 750-image corpus: **750 of 750** regions are
**byte-identical** to the native decoder's, on the whole-file path and on
the range path; the range path fetches the tiles the viewport touches and
nothing else; and the module's canonical digest is computed by the **same**
Rust function the command line uses. Thirteen tests in `wasm/test/` compare
the module against the real `vpng` binary, and eight Rust unit tests cover
the registry on the host.
