// The WebAssembly module's corpus measurement.
//
// This is the half of the measurement that has to run inside the module's
// runtime. The Python driver (`benchmarks/wasm/run_wasm_bench.py`) encodes
// the corpus and measures the native path; this script measures the
// WebAssembly path over the same files and writes the same shape of
// record, so the two can be compared line by line.
//
// For every file it measures two ways:
//
// * the **whole-file** path: the container is copied into the module's
//   memory and the region is decoded from there;
// * the **range** path: the module is opened with the header and the
//   directory alone, it says which tiles the region touches, and the tiles
//   are fetched by their byte ranges — the way a browser reads a container
//   in an object store.
//
// Both paths must produce the same canonical digest as the native decoder.
//
//     node wasm/run_wasm_bench.mjs --wasm <path> --dir <dir> --jobs <jobs.json> \
//         --output <results.json>

import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { performance } from 'node:perf_hooks';

import { Vpng } from './vpng.js';

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 1) {
    const key = argv[i];
    if (!key.startsWith('--')) continue;
    const name = key.slice(2);
    const value = argv[i + 1];
    i += 1;
    args[name] = value;
  }
  return args;
}

const args = parseArgs(process.argv.slice(2));
if (!args.wasm || !args.dir || !args.jobs || !args.output) {
  console.error(
    'usage: node wasm/run_wasm_bench.mjs --wasm FILE --dir DIR --jobs FILE --output FILE',
  );
  process.exit(2);
}

const vpng = await Vpng.instantiate(readFileSync(args.wasm));
const jobs = JSON.parse(readFileSync(args.jobs, 'utf8'));
const results = [];

for (const job of jobs) {
  const bytes = readFileSync(join(args.dir, job.file));
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const directoryOffset = Number(view.getBigUint64(32, true));
  const tileCount = view.getUint32(28, true);
  const directorySize = tileCount * 40 + 4;
  const header = new Uint8Array(bytes.subarray(0, 64));
  const directory = new Uint8Array(
    bytes.subarray(directoryOffset, directoryOffset + directorySize),
  );

  // The whole-file path.
  const wholeStarted = performance.now();
  const wholeHandle = vpng.open(new Uint8Array(bytes));
  const wholePixels = vpng.decodeRegion(wholeHandle, job.x, job.y, job.width, job.height);
  const wholeMs = performance.now() - wholeStarted;
  const wholeDigest = vpng.canonicalDigest(
    wholePixels,
    job.width,
    job.height,
    vpng.format(wholeHandle),
  );
  const format = vpng.format(wholeHandle);
  const tiles = vpng.tileCount(wholeHandle);
  const levels = vpng.levelCount(wholeHandle);
  vpng.close(wholeHandle);

  // The range path: the module is opened with the header and the directory
  // alone, and only the tiles the region touches are fetched.
  const rangeStarted = performance.now();
  const rangeHandle = vpng.openParts(header, directory, bytes.length);
  let rangeBytes = 0;
  let rangeRequests = 0;
  const streamed = await vpng.decodeRegionStreaming(
    rangeHandle,
    job.x,
    job.y,
    job.width,
    job.height,
    async (offset, length) => {
      rangeBytes += length;
      rangeRequests += 1;
      return new Uint8Array(bytes.subarray(offset, offset + length));
    },
  );
  const rangeMs = performance.now() - rangeStarted;
  const rangeDigest = vpng.canonicalDigest(streamed.pixels, job.width, job.height, format);
  vpng.close(rangeHandle);

  results.push({
    id: job.id,
    file: job.file,
    file_bytes: bytes.length,
    header_bytes: 64,
    directory_bytes: directorySize,
    range_bytes: rangeBytes + 64 + directorySize,
    range_payload_bytes: rangeBytes,
    range_requests: rangeRequests,
    viewport_width: job.width,
    viewport_height: job.height,
    format,
    tiles,
    levels,
    whole_ms: wholeMs,
    range_ms: rangeMs,
    whole_sha256: wholeDigest,
    range_sha256: rangeDigest,
  });

  if (results.length % 25 === 0 || results.length === jobs.length) {
    console.log(`  ${results.length}/${jobs.length} measured`);
  }
}

writeFileSync(args.output, `${JSON.stringify(results, null, 2)}\n`);
console.log(`wrote ${args.output}`);
