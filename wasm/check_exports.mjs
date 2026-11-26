// The module's export surface, checked.
//
// The JavaScript wrapper is optional sugar: the package's contract is the
// **raw `extern "C"` ABI**. This script proves that every documented export
// is actually there, so a rename or a dropped function cannot pass silently.
//
// Run from the repository root:
//
//     cargo build --release --target wasm32-unknown-unknown -p vpng-wasm
//     node wasm/check_exports.mjs
//
// The module's path can be overridden with `VPNG_WASM`.

import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = fileURLToPath(new URL('.', import.meta.url));
const REPO = resolve(HERE, '..');
const WASM_PATH =
  process.env.VPNG_WASM ??
  join(REPO, 'target', 'wasm32-unknown-unknown', 'release', 'vpng_wasm.wasm');

/** Every export the package documents, with what it must return. */
const REQUIRED = [
  // Memory.
  'vpng_alloc',
  'vpng_dealloc',
  // Opening.
  'vpng_open',
  'vpng_open_parts',
  'vpng_close',
  // Layout.
  'vpng_width',
  'vpng_height',
  'vpng_format',
  'vpng_bytes_per_pixel',
  'vpng_tile_count',
  'vpng_level_count',
  'vpng_tile_offset',
  'vpng_tile_length',
  'vpng_tile_uncompressed_length',
  'vpng_tile_codec',
  'vpng_tile_level',
  'vpng_tile_rect',
  // Decoding.
  'vpng_region_length',
  'vpng_region_tiles',
  'vpng_decode_region',
  'vpng_decode_payload',
  'vpng_check',
  // Digests.
  'vpng_canonical_digest',
  'vpng_sha256',
  // Errors and the format.
  'vpng_error_length',
  'vpng_error_copy',
  'vpng_report_error',
  'vpng_format_version',
];

const bytes = readFileSync(WASM_PATH);
const { instance } = await WebAssembly.instantiate(bytes, {});
const exports = instance.exports;

const missing = REQUIRED.filter((name) => !(name in exports));
if (missing.length > 0) {
  console.error(`missing exports: ${missing.join(', ')}`);
  process.exit(1);
}

// The module must export a linear memory, or none of the pointer arguments
// mean anything.
if (!(exports.memory instanceof WebAssembly.Memory)) {
  console.error('the module does not export a linear memory');
  process.exit(1);
}

// A handle is `i32` and zero is always an error: closing handle zero must
// fail rather than succeed.
const closed = exports.vpng_close(0);
if (closed >= 0) {
  console.error(`closing handle zero returned ${closed}, expected a negative code`);
  process.exit(1);
}

// The format version is major in the high half and minor in the low half.
const version = exports.vpng_format_version();
const major = (version >> 16) & 0xffff;
const minor = version & 0xffff;
if (major !== 0) {
  console.error(`the format major version is ${major}, expected 0`);
  process.exit(1);
}

console.log(`exports: ${REQUIRED.length} checked, memory: ${exports.memory.buffer.byteLength} bytes`);
console.log(`format version: ${major}.${minor}`);
