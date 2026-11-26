// The WebAssembly module's own test.
//
// The claim this package makes is narrow and testable: **the browser's
// pixels are the command line's pixels, byte for byte**. This test proves it
// against the real `vpng` binary, not against a pinned constant.
//
// For every case it:
//
// * builds a deterministic raw image;
// * encodes it with the command line;
// * asks the command line for a region's canonical digest;
// * asks the WebAssembly module for the same region and computes the digest
//   with the module's own canonical-digest function;
// * compares the two.
//
// It also pins the refusals: a corrupt payload, a payload of the wrong
// length, a pointer outside the module's memory, and a file that is not a
// container at all.
//
// Run from the repository root:
//
//     cargo build --release --target wasm32-unknown-unknown -p vpng-wasm
//     cargo build --release -p vpng-cli
//     node --test wasm/test/vpng.test.mjs
//
// The two binaries are found through `VPNG_WASM` and `VPNG_BIN` if they are
// somewhere else.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { Vpng, VpngError, FORMAT_NAMES } from '../vpng.js';

const HERE = fileURLToPath(new URL('.', import.meta.url));
const REPO = resolve(HERE, '..', '..');

const WASM_PATH =
  process.env.VPNG_WASM ??
  join(REPO, 'target', 'wasm32-unknown-unknown', 'release', 'vpng_wasm.wasm');
const BIN_PATH =
  process.env.VPNG_BIN ??
  join(REPO, 'target', 'release', process.platform === 'win32' ? 'vpng.exe' : 'vpng');

let work;
let vpng;

function run(args) {
  return execFileSync(BIN_PATH, args, { encoding: 'utf8' });
}

function runJson(args) {
  return JSON.parse(run([...args, '--json']));
}

/** A deterministic raw RGB8 image: 300 x 200. */
function rawPixels(width, height) {
  const bytes = Buffer.alloc(width * height * 3);
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const at = (y * width + x) * 3;
      bytes[at] = x % 251;
      bytes[at + 1] = y % 241;
      bytes[at + 2] = ((Math.floor(x / 16) + Math.floor(y / 16)) % 2) * 255;
    }
  }
  return bytes;
}

/** Encode the raw image and return the container's path and bytes. */
function encode(name, width, height, extra = []) {
  const raw = join(work, `${name}.raw`);
  const container = join(work, `${name}.vpng`);
  writeFileSync(raw, rawPixels(width, height));
  const report = runJson([
    'encode',
    raw,
    container,
    '--input-format',
    'raw',
    '--width',
    String(width),
    '--height',
    String(height),
    '--format',
    'rgb8',
    '--tile-size',
    '64',
    ...extra,
  ]);
  return { container, report, bytes: readFileSync(container) };
}

test.before(async () => {
  work = mkdtempSync(join(tmpdir(), 'vpng-wasm-'));
  vpng = await Vpng.instantiate(readFileSync(WASM_PATH));
});

test.after(() => {
  if (work) rmSync(work, { recursive: true, force: true });
});

test('the module reports the container format it understands', () => {
  const version = vpng.formatVersion();
  assert.equal(version >>> 16, 0, 'the major version is zero');
  assert.equal(version & 0xffff, 3, 'the minor version is three');
});

test('the pixels are the command line pixels, byte for byte', () => {
  const { container, bytes } = encode('exact', 300, 200);
  const handle = vpng.open(new Uint8Array(bytes));

  const native = runJson([
    'decode-region',
    container,
    join(work, 'exact.png'),
    '--x',
    '0',
    '--y',
    '0',
    '--width',
    '300',
    '--height',
    '200',
  ]);

  const pixels = vpng.decodeRegion(handle, 0, 0, 300, 200);
  const digest = vpng.canonicalDigest(pixels, 300, 200, vpng.format(handle));

  assert.equal(digest, native.sha256, 'the canonical digests differ');
  vpng.close(handle);
});

test('a region in the middle of the picture is the command line region', () => {
  const { container, bytes } = encode('middle', 300, 200);
  const handle = vpng.open(new Uint8Array(bytes));

  for (const rect of [
    [0, 0, 64, 64],
    [64, 32, 128, 96],
    [200, 150, 100, 50],
    [299, 199, 1, 1],
  ]) {
    const [x, y, width, height] = rect;
    const native = runJson([
      'decode-region',
      container,
      join(work, `middle-${x}-${y}.png`),
      '--x',
      String(x),
      '--y',
      String(y),
      '--width',
      String(width),
      '--height',
      String(height),
    ]);
    const pixels = vpng.decodeRegion(handle, x, y, width, height);
    assert.equal(pixels.length, width * height * 3, 'the region is the wrong size');
    const digest = vpng.canonicalDigest(pixels, width, height, vpng.format(handle));
    assert.equal(digest, native.sha256, `the region ${rect.join('x')} differs`);
  }
  vpng.close(handle);
});

test('the layout the module reports is the layout the command line reports', () => {
  const { container, bytes } = encode('layout', 300, 200);
  const handle = vpng.open(new Uint8Array(bytes));
  const native = runJson(['info', container]);

  assert.equal(vpng.width(handle), native.width);
  assert.equal(vpng.height(handle), native.height);
  assert.equal(vpng.tileCount(handle), native.tile_count);
  assert.equal(vpng.levelCount(handle), native.level_count);
  assert.equal(vpng.formatName(handle), 'rgb8');
  assert.equal(vpng.bytesPerPixel(handle), 3);
  assert.equal(FORMAT_NAMES[vpng.format(handle)], 'rgb8');
  vpng.close(handle);
});

test('a preview pyramid is visible to the module', () => {
  const { container, bytes } = encode('pyramid', 300, 200, ['--pyramid']);
  const handle = vpng.open(new Uint8Array(bytes));
  const native = runJson(['info', container]);
  assert.ok(native.level_count > 1, 'the file has previews');
  assert.equal(vpng.levelCount(handle), native.level_count);
  vpng.close(handle);
});

test('the parts path decodes the same pixels as the whole path', () => {
  const { container, bytes } = encode('parts', 300, 200);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);

  // The header's directory offset is at byte 32, and the tile count is at
  // byte 28. The directory is 40 bytes per tile plus a four-byte checksum.
  const directoryOffset = Number(view.getBigUint64(32, true));
  const tileCount = view.getUint32(28, true);
  const directorySize = tileCount * 40 + 4;

  const header = bytes.subarray(0, 64);
  const directory = bytes.subarray(directoryOffset, directoryOffset + directorySize);
  const handle = vpng.openParts(new Uint8Array(header), new Uint8Array(directory), bytes.length);

  assert.equal(vpng.width(handle), 300);
  assert.equal(vpng.tileCount(handle), tileCount);

  // Fetch one tile's payload the way a browser would — by its offset and
  // length — and decode it.
  const index = 3;
  const offset = vpng.tileOffset(handle, index);
  const length = vpng.tileLength(handle, index);
  const payload = bytes.subarray(offset, offset + length);
  const pixels = vpng.decodePayload(handle, index, new Uint8Array(payload));

  // The tile is at grid position (3, 0) of a 5-by-4 grid of 64-pixel tiles,
  // so it covers the rectangle 192x64+192+0 of the original.
  const whole = vpng.open(new Uint8Array(bytes));
  const region = vpng.decodeRegion(whole, 192, 0, 64, 64);
  vpng.close(whole);

  assert.equal(pixels.length, 64 * 64 * 3, 'the tile is the wrong size');
  assert.deepEqual(
    Array.from(pixels),
    Array.from(region),
    'the payload path and the whole path disagree',
  );
});

test('the range path paints the same region as the whole path', async () => {
  const { container, bytes } = encode('range', 300, 200);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const directoryOffset = Number(view.getBigUint64(32, true));
  const tileCount = view.getUint32(28, true);
  const directorySize = tileCount * 40 + 4;

  const handle = vpng.openParts(
    new Uint8Array(bytes.subarray(0, 64)),
    new Uint8Array(bytes.subarray(directoryOffset, directoryOffset + directorySize)),
    bytes.length,
  );

  // The "fetch" is a slice of the file, which is what a range request over
  // HTTP would hand back.
  const fetched = [];
  const fetchRange = async (offset, length) => {
    fetched.push({ offset, length });
    return new Uint8Array(bytes.subarray(offset, offset + length));
  };

  const rect = [32, 16, 128, 96];
  const streamed = await vpng.decodeRegionStreaming(handle, ...rect, fetchRange);
  vpng.close(handle);

  const whole = vpng.open(new Uint8Array(bytes));
  const direct = vpng.decodeRegion(whole, ...rect);
  vpng.close(whole);

  assert.deepEqual(
    Array.from(streamed.pixels),
    Array.from(direct),
    'the range path and the whole path disagree',
  );
  assert.equal(streamed.requests, fetched.length);
  assert.ok(streamed.bytes < bytes.length, 'the range path fetched the whole file');
});

test('a payload with a broken checksum is refused', () => {
  const { bytes } = encode('corrupt', 300, 200);
  const handle = vpng.open(new Uint8Array(bytes));
  const index = 0;
  const offset = vpng.tileOffset(handle, index);
  const length = vpng.tileLength(handle, index);
  const payload = new Uint8Array(bytes.subarray(offset, offset + length));
  payload[payload.length - 1] ^= 0xff;

  assert.throws(() => vpng.decodePayload(handle, index, payload), VpngError);
  vpng.close(handle);
});

test('a payload of the wrong length is refused', () => {
  const { bytes } = encode('short', 300, 200);
  const handle = vpng.open(new Uint8Array(bytes));
  assert.throws(
    () => vpng.decodePayload(handle, 0, new Uint8Array([1, 2, 3])),
    VpngError,
  );
  vpng.close(handle);
});

test('a file that is not a container is refused', () => {
  assert.throws(
    () => vpng.open(new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8, 9, 10])),
    VpngError,
  );
});

test('a pointer outside the module memory is refused, not obeyed', () => {
  const { bytes } = encode('bounds', 300, 200);
  const handle = vpng.open(new Uint8Array(bytes));
  const code = vpng.exports.vpng_decode_region(handle, 0, 0, 16, 16, 0xfffff000);
  assert.ok(code < 0, 'the bad pointer was accepted');
  vpng.close(handle);
});

test('every payload checksum verifies', () => {
  const { bytes } = encode('check', 300, 200);
  const handle = vpng.open(new Uint8Array(bytes));
  assert.equal(vpng.check(handle), true);
  vpng.close(handle);
});

test('the SHA-256 the module computes is the known SHA-256', () => {
  // The SHA-256 of the empty string, from FIPS 180-4.
  assert.equal(
    vpng.sha256(new Uint8Array(0)),
    'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
  );
});
