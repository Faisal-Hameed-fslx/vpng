# The VPNG WebAssembly package

The browser's copy of the VPNG engine.

This package is **not** a second implementation of the format. Every pixel it
produces comes from the same `vpng-decoder` and `vpng-codecs` code the
command line runs, through the same shared container validation and the same
shared tile decoding. The evidence is in the test suite: the module's
canonical digest for a region is the command line's canonical digest for the
same region, on every image of the 750-image corpus.

## What is in here

| File | What it is |
|---|---|
| `vpng.js` | The JavaScript wrapper. Plain ES modules, no dependency, no build step. |
| `run_wasm_bench.mjs` | The measurement half of the corpus benchmark. |
| `check_exports.mjs` | The check that every documented export is really there. |
| `test/vpng.test.mjs` | The module's own test suite, run by Node's test runner. |

The Rust side is the workspace crate `crates/vpng-wasm`.

## Building it

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown -p vpng-wasm
```

The module lands at `target/wasm32-unknown-unknown/release/vpng_wasm.wasm`.

## Using it

```js
import { Vpng } from './vpng.js';

const vpng = await Vpng.instantiate(await fetch('vpng_wasm.wasm'));
const image = vpng.open(containerBytes);

console.log(image.width, image.height, vpng.formatName(image));

// A rectangle of the picture, as canonical pixels.
const pixels = vpng.decodeRegion(image, 0, 0, 1024, 1024);
```

The pixels come back in the **canonical** order: rows tightly packed, a
16-bit sample little-endian. That is the order the format stores and the
order the native decoder returns, so the two can be compared byte for byte.

### Reading a container in an object store

The whole-file path needs the whole container in memory. The **range** path
does not: the module is opened with the header and the directory alone, it
says which tiles a rectangle touches, and the caller fetches exactly those
payloads.

```js
const view = new DataView(containerBytes.buffer);
const directoryOffset = Number(view.getBigUint64(32, true));
const tileCount = view.getUint32(28, true);

const image = vpng.openParts(
  containerBytes.subarray(0, 64),
  containerBytes.subarray(directoryOffset, directoryOffset + tileCount * 40 + 4),
  containerBytes.length,
);

const { pixels, bytes, requests } = await vpng.decodeRegionStreaming(
  image,
  0,
  0,
  1024,
  1024,
  async (offset, length) => {
    const response = await fetch(url, {
      headers: { Range: `bytes=${offset}-${offset + length - 1}` },
    });
    return new Uint8Array(await response.arrayBuffer());
  },
);
```

`bytes` is what the viewport cost and `requests` is how many ranges it took.
The pixels are the same ones the whole-file path returns.

## The ABI

The module exports a **raw `extern "C"` ABI**. There is no `wasm-bindgen`,
no generated glue and no build-time code generation.

| Export | Returns | What it does |
|---|---|---|
| `vpng_alloc(len)` | `u32` | Allocate `len` bytes; zero on failure. |
| `vpng_dealloc(ptr, len)` | — | Return a buffer. |
| `vpng_open(ptr, len)` | `i32` | Open a whole container; returns a handle. |
| `vpng_open_parts(hptr, hlen, dptr, dlen, size)` | `i32` | Open from the header and the directory. |
| `vpng_close(handle)` | `i32` | Close a document. |
| `vpng_width(handle)` | `i32` | The image's width. |
| `vpng_height(handle)` | `i32` | The image's height. |
| `vpng_format(handle)` | `i32` | The pixel format's code. |
| `vpng_bytes_per_pixel(handle)` | `i32` | The bytes one pixel takes. |
| `vpng_tile_count(handle)` | `i32` | The number of tiles. |
| `vpng_level_count(handle)` | `i32` | The number of levels. |
| `vpng_tile_offset(handle, index)` | `i64` | A tile's payload offset. |
| `vpng_tile_length(handle, index)` | `i64` | A tile's compressed length. |
| `vpng_tile_uncompressed_length(handle, index)` | `i64` | A tile's decoded length. |
| `vpng_tile_codec(handle, index)` | `i32` | A tile's codec id. |
| `vpng_tile_level(handle, index)` | `i32` | A tile's level. |
| `vpng_tile_rect(handle, index, out)` | `i32` | A tile's rectangle: four `u32`s. |
| `vpng_region_length(h, x, y, w, h)` | `i64` | The bytes a region takes. |
| `vpng_region_tiles(h, x, y, w, h, out, cap)` | `i32` | The tiles a region touches. |
| `vpng_decode_region(h, x, y, w, h, out)` | `i64` | Decode a region into a buffer. |
| `vpng_decode_payload(h, index, ptr, len, out)` | `i64` | Decode one fetched payload. |
| `vpng_check(handle)` | `i32` | Verify every payload's checksum. |
| `vpng_canonical_digest(ptr, len, w, h, fmt, out)` | `i32` | The specification's canonical digest. |
| `vpng_sha256(ptr, len, out)` | `i32` | A plain SHA-256. |
| `vpng_error_length()` | `i32` | The last error's message length. |
| `vpng_error_copy(ptr, len)` | `i32` | Copy the last error's message. |
| `vpng_format_version()` | `i32` | The format version: major in the high half. |
| `vpng_report_error(code, ptr, len)` | `i32` | Record an error the caller found. |

The conventions:

* a **handle** is `i32`; zero is always an error;
* a **return value** is `i32` or `i64`; zero is success and a negative value
  is the negative of the error's exit code — **the same codes the command
  line uses**, so `-4` is a corrupt file and `-3` is an unsupported feature;
* an **error message** is fetched with `vpng_error_length` and
  `vpng_error_copy`;
* every **pointer** is checked against the module's real memory size before
  it is used, so a bad pointer from JavaScript is an error and not undefined
  behaviour;
* the memory **grows** when the module allocates, which detaches any typed
  array the caller made earlier. The wrapper re-creates its views after every
  call that can allocate, and never hands a view into the module's memory
  back to the caller: every returned buffer is a copy.

## Testing it

```sh
cargo build --release --target wasm32-unknown-unknown -p vpng-wasm
cargo build --release -p vpng-cli
node --test wasm/test/vpng.test.mjs
node wasm/check_exports.mjs
```

The tests compare the module against the real `vpng` binary, not against a
pinned constant: they encode an image with the command line, decode it with
both, and compare the canonical digests. The export check proves that every
function this README documents is actually exported, that the module exports
a linear memory at all, and that closing handle zero fails rather than
succeeds.

## What this package does not do

* It does not **encode**. The module is a decoder. Encoding in a browser is
  a separate piece of work, and it is not needed to *read* a picture.
* It does not **edit**. The editing engine is in `vpng-editing` and is
  native; a browser editor over it is a later piece of work.
* It does not change the **format**. Not one byte of the header or the
  directory entry moved.
