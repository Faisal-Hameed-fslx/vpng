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
| `vpng_open_parts_with_layers(hptr, hlen, dptr, dlen, tptr, tlen, size)` | `i32` | Open from the header, the directory and the layer table. |
| `vpng_open_parts_with_tables(hptr, hlen, dptr, dlen, lptr, llen, gptr, glen, size)` | `i32` | Open from the header, the directory, the layer table and the GPU table. |
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
| `vpng_layer_count(handle)` | `i32` | The number of layers. |
| `vpng_layer_name_length(handle, layer)` | `i32` | A layer's name length in bytes. |
| `vpng_layer_name_copy(handle, layer, ptr, len)` | `i32` | Copy a layer's name. |
| `vpng_layer_rect(handle, layer, out)` | `i32` | A layer's rectangle: four `u32`s. |
| `vpng_layer_is_mask(handle, layer)` | `i32` | Whether a layer is a mask. |
| `vpng_layer_length(handle, layer)` | `i64` | The bytes a layer's own pixels take. |
| `vpng_decode_layer(handle, layer, out)` | `i64` | Decode one layer's own pixels. |
| `vpng_gpu_has_table(handle)` | `i32` | Whether the container carries a GPU table. |
| `vpng_gpu_record_count(handle)` | `i32` | The number of GPU records. |
| `vpng_gpu_payload_bytes(handle)` | `i64` | The GPU payloads' total size. |
| `vpng_gpu_payload_offset(handle, index)` | `i64` | One GPU payload's offset in the file. |
| `vpng_gpu_payload_length(handle, index)` | `i64` | One GPU payload's length. |
| `vpng_gpu_colour_codec(handle, index)` | `i32` | One record's colour codec id: 7 is BC1. |
| `vpng_gpu_alpha_codec(handle, index)` | `i32` | One record's alpha codec id: 8 is BC4. |
| `vpng_gpu_region_length(h, x, y, w, h)` | `i64` | The bytes a GPU region takes. |
| `vpng_decode_gpu_region(h, x, y, w, h, out)` | `i64` | Decode a region of the **lossy** GPU payload. |
| `vpng_decode_gpu_payload(h, index, ptr, len, out)` | `i64` | Decode one fetched GPU payload. |

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

## Layers

A container written for format v0.4 may hold **layers**: named grids of
tiles, each with a rectangle on the canvas, an opacity and — when the format
has an 8-bit alpha channel — a mask. Layer 0 is the base layer, the picture
itself.

The module reports the layers and decodes one of them:

```js
console.log(vpng.layerCount(image));          // 3
console.log(vpng.layerName(image, 1));         // 'ink'
console.log(vpng.layerRect(image, 1));         // { x: 8, y: 8, width: 32, height: 16 }
console.log(vpng.layerIsMask(image, 2));        // true
const pixels = vpng.decodeLayer(image, 1);    // the overlay's own pixels
```

A layer's pixels are sized to the layer's **own rectangle**, not to the
canvas, and they are the layer's pixels exactly.

The **whole-file** path opens a layered container with no help. The **parts**
path needs the layer table as well, because the directory alone does not say
which tile belongs to which layer:

```js
const view = new DataView(containerBytes.buffer);
const directoryOffset = Number(view.getBigUint64(32, true));
const tileCount = view.getUint32(28, true);
const tableOffset = view.getUint32(56, true);
const tableLength = view.getUint32(60, true);

const image = vpng.openPartsWithLayers(
  containerBytes.subarray(0, 64),
  containerBytes.subarray(directoryOffset, directoryOffset + tileCount * 40 + 4),
  containerBytes.subarray(tableOffset, tableOffset + tableLength),
  containerBytes.length,
);
```

The module **does not composite**: it reports the layers and decodes them,
and the caller blends them if it wants a flattened picture.

## The GPU payload

A container written for format v0.5 may carry an **optional GPU payload**:
beside every tile's lossless payload, a second, explicitly named, **lossy**
payload in the block format a GPU samples as a texture — BC1 colour blocks
and, when the format has an alpha channel, BC4 alpha blocks.

The GPU payload is a **sibling** of the lossless payload, never a
replacement for it. Nothing exact reads it.

```js
console.log(vpng.gpuHasTable(image));            // true
console.log(vpng.gpuRecordCount(image));          // the tile count
console.log(vpng.gpuPayloadBytes(image));         // the payloads' total size
console.log(vpng.gpuColourCodec(image, 0));       // 7 — BC1
console.log(vpng.gpuAlphaCodec(image, 0));        // 8 — BC4, or 0 for none

// The GPU payload decodes to RGBA8, and it is LOSSY.
const pixels = vpng.decodeGpuRegion(image, 0, 0, 1024, 1024);
```

The **parts** path needs the GPU table as well, because the directory alone
does not say which block codec a tile's GPU payload uses:

```js
const view = new DataView(containerBytes.buffer);
const headerSize = view.getUint32(12, true);         // 72 for a v0.5 file
const directoryOffset = Number(view.getBigUint64(32, true));
const tileCount = view.getUint32(28, true);
const tableOffset = view.getUint32(64, true);
const tableLength = view.getUint32(68, true);

const image = vpng.openPartsWithTables(
  containerBytes.subarray(0, headerSize),
  containerBytes.subarray(directoryOffset, directoryOffset + tileCount * 40 + 4),
  null,
  containerBytes.subarray(tableOffset, tableOffset + tableLength),
  containerBytes.length,
);
```

A container with no GPU table says so: `gpuHasTable` returns `false` and
`decodeGpuRegion` **refuses** with the unsupported-feature code, rather than
returning an empty picture.

## What this package does not do

* It does not **encode**. The module is a decoder. Encoding in a browser is
  a separate piece of work, and it is not needed to *read* a picture.
* It does not **edit**. The editing engine is in `vpng-editing` and is
  native; a browser editor over it is a later piece of work.
* It does not **composite**. It reports the layers and decodes them; the
  caller blends them.
* It does not change the **format**. Not one byte of the header or the
  directory entry moved: the v0.4 layer table uses header bytes that were
  reserved, and the layer index uses directory flag bits that were reserved.
* It does not use the **GPU payload** for anything exact. The GPU payload is
  lossy by construction; the exact pixels come from the lossless payload.
