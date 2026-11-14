#!/usr/bin/env python3
"""Generate deterministic large canonical images for the stress harness.

Everything is streamed in row blocks, so peak memory stays in the low megabytes
even for a 16K image. Prints one JSON object with dimensions, format, size, and
canonical sha256.
"""

import argparse
import hashlib
import json

import numpy as np


def _write_blocks(path: str, width: int, height: int, block_rows: int, build) -> tuple[int, str]:
    digest = hashlib.sha256()
    written = 0
    x = np.arange(width, dtype=np.uint32)
    with open(path, "wb") as fh:
        for y0 in range(0, height, block_rows):
            y1 = min(y0 + block_rows, height)
            rows = np.arange(y0, y1, dtype=np.uint32)[:, None]
            block = build(x[None, :], rows, y1 - y0)
            data = np.asarray(block, dtype=np.uint8).tobytes()
            digest.update(data)
            fh.write(data)
            written += len(data)
    return written, digest.hexdigest()


def gray8_gradient(x, rows, _n):
    return (rows + x) & 0xFF


def rgba8_blocks(x, rows, n):
    red = (x * 3) & 0xFF
    green = (rows * 5) & 0xFF
    blue = (((x // 16) + (rows // 16)) * 17) & 0xFF
    alpha = 0x80 | ((x >> 9) & 0x7F)
    return np.stack(
        [
            np.broadcast_to(red, (n, x.shape[1])),
            np.broadcast_to(green, (n, x.shape[1])),
            np.broadcast_to(blue, (n, x.shape[1])),
            np.broadcast_to(alpha, (n, x.shape[1])),
        ],
        axis=-1,
    )


KINDS = {
    "gray8-gradient": ("gray8", 1, gray8_gradient),
    "rgba8-blocks": ("rgba8", 4, rgba8_blocks),
}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("kind", choices=sorted(KINDS))
    parser.add_argument("width", type=int)
    parser.add_argument("height", type=int)
    parser.add_argument("output")
    parser.add_argument("--block-rows", type=int, default=256)
    args = parser.parse_args()

    pixel_format, bpp, build = KINDS[args.kind]
    written, sha = _write_blocks(
        args.output, args.width, args.height, args.block_rows, build
    )
    expected = args.width * args.height * bpp
    if written != expected:
        raise SystemExit(f"size mismatch: wrote {written}, expected {expected}")
    print(
        json.dumps(
            {
                "kind": args.kind,
                "output": args.output,
                "width": args.width,
                "height": args.height,
                "pixel_format": pixel_format,
                "bytes": written,
                "sha256": sha,
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())