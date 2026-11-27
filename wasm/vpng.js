// The VPNG decoder, in JavaScript.
//
// The WebAssembly module (`vpng_wasm.wasm`) exports a raw `extern "C"` ABI.
// This file is the wrapper around it: it turns pointer arithmetic and return
// codes into an ordinary JavaScript API, and it is the only place that knows
// how the module's memory works.
//
// There is no build step and no dependency. The module is instantiated with
// `WebAssembly.instantiate`, and everything here is plain ES modules, so the
// same file runs in a browser and in Node.
//
//     import { Vpng } from './vpng.js';
//
//     const vpng = await Vpng.instantiate(bytesOfTheWasmFile);
//     const image = vpng.open(containerBytes);
//     const pixels = vpng.decodeRegion(image, 0, 0, 1024, 1024);
//     // `pixels` is the canonical RGBA8 (or RGB8, ...) bytes of that region.
//
// # The conventions the wrapper adds
//
// * A failed call throws a `VpngError` carrying the module's error code and
//   its message. The codes are the command line's exit codes: 2 is an
//   invalid argument, 3 an unsupported feature, 4 a corrupt file.
// * Byte lengths and offsets come back as JavaScript numbers. The module
//   computes them as 64-bit values; a value beyond `Number.MAX_SAFE_INTEGER`
//   is refused rather than rounded, because a byte offset that has been
//   rounded is a byte offset that points at the wrong bytes.
// * The module's memory grows when it allocates, which detaches any typed
//   array the caller made earlier. The wrapper therefore re-creates its view
//   after every call that can allocate, and the caller never holds a view
//   into the module's memory: every returned buffer is a copy.

/** The pixel formats, in the order the numeric format codes use. */
export const FORMAT_NAMES = Object.freeze([
  'gray8',
  'gray16',
  'graya8',
  'graya16',
  'rgb8',
  'rgb16',
  'rgba8',
  'rgba16',
]);

/** The bytes one pixel takes, for each format code. */
const BYTES_PER_PIXEL = Object.freeze([1, 2, 2, 4, 3, 6, 4, 8]);

/**
 * A failure reported by the module.
 *
 * `code` is the module's exit code for the failure: 2 is an invalid
 * argument, 3 an unsupported feature, 4 a corrupt file, 5 a resource limit,
 * 6 a size limit, 7 a verification failure and 8 an I/O failure. They are
 * the same codes the command line uses.
 */
export class VpngError extends Error {
  /**
   * @param {number} code the module's error code
   * @param {string} message the module's message
   */
  constructor(code, message) {
    super(message);
    this.name = 'VpngError';
    this.code = code;
  }
}

/** The largest value a byte count or offset may take. */
const MAX_SAFE = Number.MAX_SAFE_INTEGER;

/** Convert a 64-bit value from the module into a JavaScript number. */
function toNumber(value, what) {
  const number = typeof value === 'bigint' ? Number(value) : value;
  if (!Number.isSafeInteger(number)) {
    throw new VpngError(5, `${what} (${value}) is too large to be exact`);
  }
  return number;
}

/** Lowercase hexadecimal, for a digest. */
function toHex(bytes) {
  let out = '';
  for (let i = 0; i < bytes.length; i += 1) {
    out += bytes[i].toString(16).padStart(2, '0');
  }
  return out;
}

/** Read a whole response body into a `Uint8Array`. */
async function readAll(source) {
  if (source instanceof Uint8Array) return source;
  if (source instanceof ArrayBuffer) return new Uint8Array(source);
  if (typeof Response !== 'undefined' && source instanceof Response) {
    return new Uint8Array(await source.arrayBuffer());
  }
  const response = await fetch(source);
  if (!response.ok) {
    throw new VpngError(8, `could not fetch the module: ${response.status}`);
  }
  return new Uint8Array(await response.arrayBuffer());
}

/**
 * The VPNG decoder.
 *
 * One instance owns one WebAssembly module and the documents opened through
 * it.
 */
export class Vpng {
  /**
   * @param {WebAssembly.Instance} instance the instantiated module
   */
  constructor(instance) {
    this.exports = instance.exports;
    this.memory = instance.exports.memory;
    this.closed = false;
  }

  /**
   * Instantiate the module.
   *
   * @param {Uint8Array|ArrayBuffer|Response|string|URL} source the `.wasm`
   *   bytes, or something that can be fetched to get them
   * @returns {Promise<Vpng>} the ready decoder
   */
  static async instantiate(source) {
    const bytes = await readAll(source);
    const { instance } = await WebAssembly.instantiate(bytes, {});
    return new Vpng(instance);
  }

  /** A fresh view of the module's memory. */
  get bytes() {
    return new Uint8Array(this.memory.buffer);
  }

  /** The module's last error message. */
  errorMessage() {
    const length = this.exports.vpng_error_length();
    if (length <= 0) return '';
    const pointer = this.exports.vpng_alloc(length);
    if (pointer === 0) return '';
    try {
      const written = this.exports.vpng_error_copy(pointer, length);
      if (written < 0) return '';
      // The view is made after the copy, because the copy may have grown the
      // memory and detached an earlier view.
      const view = new Uint8Array(this.memory.buffer, pointer, written);
      return new TextDecoder('utf-8').decode(view);
    } finally {
      this.exports.vpng_dealloc(pointer, length);
    }
  }

  /** Throw the module's last error, with its code. */
  fail(code) {
    const message = this.errorMessage();
    throw new VpngError(-code, message || `the module failed with the code ${-code}`);
  }

  /** Call an export that returns `i32` and check it. */
  i32(name, ...args) {
    const value = this.exports[name](...args);
    if (value < 0) this.fail(value);
    return value;
  }

  /** Call an export that returns `i64` and check it. */
  i64(name, ...args) {
    const value = this.exports[name](...args);
    if (value < 0) this.fail(Number(value));
    return toNumber(value, name);
  }

  /** Copy `bytes` into the module's memory and return the pointer and length. */
  put(bytes) {
    const length = bytes.length;
    if (length === 0) {
      // The module refuses a zero-sized allocation, and an empty range needs
      // no memory at all.
      return { pointer: 0, length: 0 };
    }
    const pointer = this.exports.vpng_alloc(length);
    if (pointer === 0) {
      throw new VpngError(5, `the module could not allocate ${length} bytes`);
    }
    // The view is made after the allocation, because the allocation may have
    // grown the memory and detached an earlier view.
    new Uint8Array(this.memory.buffer, pointer, length).set(bytes);
    return { pointer, length };
  }

  /** The format code of a document. */
  format(handle) {
    return this.i32('vpng_format', handle);
  }

  /** The name of a document's pixel format. */
  formatName(handle) {
    return FORMAT_NAMES[this.format(handle)];
  }

  /**
   * Open a whole container.
   *
   * @param {Uint8Array} bytes the container's bytes
   * @returns {number} the document handle
   */
  open(bytes) {
    const { pointer, length } = this.put(bytes);
    try {
      return this.i32('vpng_open', pointer, length);
    } finally {
      this.exports.vpng_dealloc(pointer, length);
    }
  }

  /**
   * Open a container from its header and its directory alone.
   *
   * This is the path a browser takes when it reads a container over HTTP
   * Range requests: two ranges tell the module the layout, and then the
   * caller fetches the payload of the tile it wants and hands it to
   * `decodePayload`.
   *
   * @param {Uint8Array} header the 64-byte header
   * @param {Uint8Array} directory the directory, including its checksum
   * @param {number} fileSize the size the container claims for itself
   * @returns {number} the document handle
   */
  openParts(header, directory, fileSize) {
    const head = this.put(header);
    const tail = this.put(directory);
    try {
      return this.i32(
        'vpng_open_parts',
        head.pointer,
        head.length,
        tail.pointer,
        tail.length,
        fileSize,
      );
    } finally {
      this.exports.vpng_dealloc(head.pointer, head.length);
      this.exports.vpng_dealloc(tail.pointer, tail.length);
    }
  }

  /**
   * Open a container from its header, its directory and its layer table.
   *
   * A v0.4 container carries a layer table between the directory and the
   * payloads. The table may be omitted for a container that has none; a
   * container that *declares* a table and is given none is refused.
   *
   * @param {Uint8Array} header the 64-byte header
   * @param {Uint8Array} directory the directory, including its checksum
   * @param {Uint8Array|null} layerTable the layer table, or null
   * @param {number} fileSize the size the container claims for itself
   * @returns {number} the document handle
   */
  openPartsWithLayers(header, directory, layerTable, fileSize) {
    const head = this.put(header);
    const tail = this.put(directory);
    const table = layerTable ? this.put(layerTable) : { pointer: 0, length: 0 };
    try {
      return this.i32(
        'vpng_open_parts_with_layers',
        head.pointer,
        head.length,
        tail.pointer,
        tail.length,
        table.pointer,
        table.length,
        fileSize,
      );
    } finally {
      this.exports.vpng_dealloc(head.pointer, head.length);
      this.exports.vpng_dealloc(tail.pointer, tail.length);
      if (layerTable) this.exports.vpng_dealloc(table.pointer, table.length);
    }
  }

  /** Close a document. */
  close(handle) {
    this.i32('vpng_close', handle);
  }

  /** The image's width in pixels. */
  width(handle) {
    return this.i32('vpng_width', handle);
  }

  /** The image's height in pixels. */
  height(handle) {
    return this.i32('vpng_height', handle);
  }

  /** The number of bytes one pixel takes. */
  bytesPerPixel(handle) {
    return this.i32('vpng_bytes_per_pixel', handle);
  }

  /** The number of tiles in the container. */
  tileCount(handle) {
    return this.i32('vpng_tile_count', handle);
  }

  /** The number of levels in the container. */
  levelCount(handle) {
    return this.i32('vpng_level_count', handle);
  }

  /** The number of layers in the container. */
  layerCount(handle) {
    return this.i32('vpng_layer_count', handle);
  }

  /**
   * A layer's name.
   *
   * @param {number} handle the document
   * @param {number} layer the layer's index
   * @returns {string} the name, which may be empty
   */
  layerName(handle, layer) {
    const length = this.i32('vpng_layer_name_length', handle, layer);
    if (length <= 0) return '';
    const pointer = this.exports.vpng_alloc(length);
    if (pointer === 0) {
      throw new VpngError(5, `the module could not allocate ${length} bytes`);
    }
    try {
      const written = this.i32(
        'vpng_layer_name_copy',
        handle,
        layer,
        pointer,
        length,
      );
      const view = new Uint8Array(this.memory.buffer, pointer, written);
      return new TextDecoder('utf-8').decode(view);
    } finally {
      this.exports.vpng_dealloc(pointer, length);
    }
  }

  /**
   * The rectangle a layer covers on the canvas.
   *
   * @param {number} handle the document
   * @param {number} layer the layer's index
   * @returns {{x: number, y: number, width: number, height: number}} the
   *   layer's rectangle
   */
  layerRect(handle, layer) {
    const pointer = this.exports.vpng_alloc(16);
    if (pointer === 0) {
      throw new VpngError(5, 'the module could not allocate 16 bytes');
    }
    try {
      this.i32('vpng_layer_rect', handle, layer, pointer);
      const view = new Uint8Array(this.memory.buffer, pointer, 16);
      const at = (offset) =>
        (view[offset] |
          (view[offset + 1] << 8) |
          (view[offset + 2] << 16) |
          (view[offset + 3] << 24)) >>>
        0;
      return { x: at(0), y: at(4), width: at(8), height: at(12) };
    } finally {
      this.exports.vpng_dealloc(pointer, 16);
    }
  }

  /**
   * Whether a layer is a mask rather than picture pixels.
   *
   * @param {number} handle the document
   * @param {number} layer the layer's index
   * @returns {boolean} true for a mask
   */
  layerIsMask(handle, layer) {
    return this.i32('vpng_layer_is_mask', handle, layer) === 1;
  }

  /** The number of canonical bytes one layer's pixels take. */
  layerLength(handle, layer) {
    return this.i64('vpng_layer_length', handle, layer);
  }

  /**
   * Decode one layer's own pixels.
   *
   * The result is sized to the **layer's own rectangle**, not to the canvas,
   * and it holds the layer's pixels exactly.
   *
   * @param {number} handle the document
   * @param {number} layer the layer's index
   * @returns {Uint8Array} the canonical pixels
   */
  decodeLayer(handle, layer) {
    const length = this.layerLength(handle, layer);
    const pointer = this.exports.vpng_alloc(length);
    if (pointer === 0) {
      throw new VpngError(5, `the module could not allocate ${length} bytes`);
    }
    try {
      const written = this.i64('vpng_decode_layer', handle, layer, pointer);
      const view = new Uint8Array(this.memory.buffer, pointer, written);
      return view.slice();
    } finally {
      this.exports.vpng_dealloc(pointer, length);
    }
  }

  /** The byte offset of a tile's payload. */
  tileOffset(handle, index) {
    return this.i64('vpng_tile_offset', handle, index);
  }

  /** The compressed length of a tile's payload. */
  tileLength(handle, index) {
    return this.i64('vpng_tile_length', handle, index);
  }

  /** The uncompressed length of a tile. */
  tileUncompressedLength(handle, index) {
    return this.i64('vpng_tile_uncompressed_length', handle, index);
  }

  /** The codec id a tile's payload uses. */
  tileCodec(handle, index) {
    return this.i32('vpng_tile_codec', handle, index);
  }

  /** The level a tile belongs to. */
  tileLevel(handle, index) {
    return this.i32('vpng_tile_level', handle, index);
  }

  /** The number of canonical bytes a region takes. */
  regionLength(handle, x, y, width, height) {
    return this.i64('vpng_region_length', handle, x, y, width, height);
  }

  /**
   * The tiles of the original level that a region touches.
   *
   * @param {number} handle the document
   * @param {number} x the rectangle's left edge
   * @param {number} y the rectangle's top edge
   * @param {number} width the rectangle's width
   * @param {number} height the rectangle's height
   * @returns {number[]} the tiles' directory indices, in ascending order
   */
  regionTiles(handle, x, y, width, height) {
    // The count is bounded by the number of tiles, which the directory
    // already knows.
    const capacity = this.tileCount(handle) + 1;
    const pointer = this.exports.vpng_alloc(capacity * 4);
    if (pointer === 0) {
      throw new VpngError(5, `the module could not allocate ${capacity * 4} bytes`);
    }
    try {
      const count = this.i32(
        'vpng_region_tiles',
        handle,
        x,
        y,
        width,
        height,
        pointer,
        capacity,
      );
      const view = new Uint8Array(this.memory.buffer, pointer, count * 4);
      const indices = [];
      for (let i = 0; i < count; i += 1) {
        indices.push(
          view[i * 4] | (view[i * 4 + 1] << 8) | (view[i * 4 + 2] << 16) |
            (view[i * 4 + 3] << 24),
        );
      }
      return indices;
    } finally {
      this.exports.vpng_dealloc(pointer, capacity * 4);
    }
  }

  /**
   * The rectangle a tile covers.
   *
   * @param {number} handle the document
   * @param {number} index the tile's index
   * @returns {{x: number, y: number, width: number, height: number}} the
   *   tile's rectangle in the original image
   */
  tileRect(handle, index) {
    const pointer = this.exports.vpng_alloc(16);
    if (pointer === 0) {
      throw new VpngError(5, 'the module could not allocate 16 bytes');
    }
    try {
      this.i32('vpng_tile_rect', handle, index, pointer);
      const view = new Uint8Array(this.memory.buffer, pointer, 16);
      const at = (offset) =>
        (view[offset] |
          (view[offset + 1] << 8) |
          (view[offset + 2] << 16) |
          (view[offset + 3] << 24)) >>>
        0;
      return { x: at(0), y: at(4), width: at(8), height: at(12) };
    } finally {
      this.exports.vpng_dealloc(pointer, 16);
    }
  }

  /**
   * Decode a rectangle using only the ranges the caller fetches.
   *
   * This is the path a browser takes with a container in an object store: the
   * module is opened with the header and the directory alone, it says which
   * tiles the rectangle touches, and the caller fetches exactly those
   * payloads. The pixels come back exactly as they do from the whole-file
   * path.
   *
   * @param {number} handle the document
   * @param {number} x the rectangle's left edge
   * @param {number} y the rectangle's top edge
   * @param {number} width the rectangle's width
   * @param {number} height the rectangle's height
   * @param {(offset: number, length: number) => Promise<Uint8Array>} fetchRange
   *   fetches a byte range of the container
   * @returns {Promise<{pixels: Uint8Array, bytes: number, requests: number}>}
   *   the region's canonical pixels, and what it cost to fetch them
   */
  async decodeRegionStreaming(handle, x, y, width, height, fetchRange) {
    const indices = this.regionTiles(handle, x, y, width, height);
    const bpp = this.bytesPerPixel(handle);
    const out = new Uint8Array(width * height * bpp);
    let bytes = 0;
    let requests = 0;

    for (const index of indices) {
      const offset = this.tileOffset(handle, index);
      const length = this.tileLength(handle, index);
      const payload = await fetchRange(offset, length);
      requests += 1;
      bytes += payload.length;
      const pixels = this.decodePayload(handle, index, payload);
      const tile = this.tileRect(handle, index);
      // Copy the tile into the region, clipping it on every side: a tile at
      // the region's edge is larger than the part of it that shows.
      const srcX = Math.max(0, x - tile.x);
      const dstX = Math.max(0, tile.x - x);
      const copyWidth = Math.min(tile.width - srcX, width - dstX);
      const srcY = Math.max(0, y - tile.y);
      const dstY = Math.max(0, tile.y - y);
      const copyHeight = Math.min(tile.height - srcY, height - dstY);
      if (copyWidth <= 0 || copyHeight <= 0) continue;
      for (let row = 0; row < copyHeight; row += 1) {
        const from = ((srcY + row) * tile.width + srcX) * bpp;
        const to = ((dstY + row) * width + dstX) * bpp;
        out.set(pixels.subarray(from, from + copyWidth * bpp), to);
      }
    }

    return { pixels: out, bytes, requests };
  }

  /**
   * Decode a rectangle of the image.
   *
   * @param {number} handle the document
   * @param {number} x the rectangle's left edge
   * @param {number} y the rectangle's top edge
   * @param {number} width the rectangle's width
   * @param {number} height the rectangle's height
   * @returns {Uint8Array} the canonical pixels: rows tightly packed, a
   *   16-bit sample little-endian
   */
  decodeRegion(handle, x, y, width, height) {
    const length = this.regionLength(handle, x, y, width, height);
    const pointer = this.exports.vpng_alloc(length);
    if (pointer === 0) {
      throw new VpngError(5, `the module could not allocate ${length} bytes`);
    }
    try {
      const written = this.i64('vpng_decode_region', handle, x, y, width, height, pointer);
      // The view is made after the decode, because the decode may have
      // grown the memory and detached an earlier view.
      const view = new Uint8Array(this.memory.buffer, pointer, written);
      return view.slice();
    } finally {
      this.exports.vpng_dealloc(pointer, length);
    }
  }

  /**
   * Decode a tile payload the caller has fetched.
   *
   * The payload's CRC32C is verified against the directory before the codec
   * sees it, exactly as in the native reader.
   *
   * @param {number} handle the document
   * @param {number} index the tile's index
   * @param {Uint8Array} payload the compressed payload
   * @returns {Uint8Array} the tile's canonical pixels
   */
  decodePayload(handle, index, payload) {
    const size = this.tileUncompressedLength(handle, index);
    const input = this.put(payload);
    const output = this.exports.vpng_alloc(size);
    if (output === 0) {
      this.exports.vpng_dealloc(input.pointer, input.length);
      throw new VpngError(5, `the module could not allocate ${size} bytes`);
    }
    try {
      const written = this.i64(
        'vpng_decode_payload',
        handle,
        index,
        input.pointer,
        input.length,
        output,
      );
      const view = new Uint8Array(this.memory.buffer, output, written);
      return view.slice();
    } finally {
      this.exports.vpng_dealloc(input.pointer, input.length);
      this.exports.vpng_dealloc(output, size);
    }
  }

  /**
   * Verify every tile payload's checksum.
   *
   * @param {number} handle the document
   * @returns {boolean} true when every payload matches the directory
   */
  check(handle) {
    const code = this.exports.vpng_check(handle);
    if (code < 0) this.fail(code);
    return true;
  }

  /**
   * The canonical digest of a decoded region.
   *
   * It is the digest the specification defines and the command line prints,
   * computed by the same Rust function, so the two can be compared directly.
   *
   * @param {Uint8Array} pixels the region's canonical pixels
   * @param {number} width the region's width
   * @param {number} height the region's height
   * @param {number} format the region's format code
   * @returns {string} the digest, in lowercase hexadecimal
   */
  canonicalDigest(pixels, width, height, format) {
    const input = this.put(pixels);
    const output = this.exports.vpng_alloc(32);
    if (output === 0) {
      this.exports.vpng_dealloc(input.pointer, input.length);
      throw new VpngError(5, 'the module could not allocate 32 bytes');
    }
    try {
      this.i32(
        'vpng_canonical_digest',
        input.pointer,
        input.length,
        width,
        height,
        format,
        output,
      );
      const view = new Uint8Array(this.memory.buffer, output, 32);
      return toHex(view);
    } finally {
      this.exports.vpng_dealloc(input.pointer, input.length);
      this.exports.vpng_dealloc(output, 32);
    }
  }

  /**
   * The SHA-256 of some bytes.
   *
   * @param {Uint8Array} bytes the bytes to hash
   * @returns {string} the digest, in lowercase hexadecimal
   */
  sha256(bytes) {
    const input = this.put(bytes);
    const output = this.exports.vpng_alloc(32);
    if (output === 0) {
      this.exports.vpng_dealloc(input.pointer, input.length);
      throw new VpngError(5, 'the module could not allocate 32 bytes');
    }
    try {
      this.i32('vpng_sha256', input.pointer, input.length, output);
      const view = new Uint8Array(this.memory.buffer, output, 32);
      return toHex(view);
    } finally {
      this.exports.vpng_dealloc(input.pointer, input.length);
      this.exports.vpng_dealloc(output, 32);
    }
  }

  /** The container format the module understands: major in the high half. */
  formatVersion() {
    return this.i32('vpng_format_version');
  }
}

export default Vpng;
