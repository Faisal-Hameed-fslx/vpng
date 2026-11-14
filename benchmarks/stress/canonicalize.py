#!/usr/bin/env python3
"""Convert one image to canonical raw pixels for the stress and corpus runs.

Uses Pillow only. Large images are streamed through a PPM intermediate so the
canonical buffer is never held in Python alongside the decoded image. The module
is importable: `convert_image(path, out_path)` returns the metadata dictionary.
"""

import argparse
import hashlib
import json
import os
import re
import sys

from PIL import Image

# A native 15360x8640 photo is a legitimate fixture, not a decompression bomb.
Image.MAX_IMAGE_PIXELS = None

FORMATS = {
    "L": ("gray8", 1),
    "LA": ("graya8", 2),
    "RGB": ("rgb8", 3),
    "RGBA": ("rgba8", 4),
}

# Below this many canonical bytes, writing the buffer directly is simplest.
DIRECT_LIMIT = 64 * 1024 * 1024


def _ppm_header_end(fh, magic: bytes) -> int:
    fh.seek(0)
    head = fh.read(256)
    if magic == b"P7":
        marker = b"ENDHDR\n"
        index = head.find(marker)
        if index < 0:
            raise ValueError("PAM header has no ENDHDR")
        return index + len(marker)
    match = re.match(rb"P[56]\s+\d+\s+\d+\s+\d+\s", head)
    if not match:
        raise ValueError("unexpected PPM/PGM header")
    return match.end()


def _stream(path: str, out_path: str) -> tuple[int, str]:
    with Image.open(path) as probe:
        fmt = probe.format
    magic = b"P7" if fmt == "PAM" else (b"P6" if fmt == "PPM" else b"P5")
    digest = hashlib.sha256()
    written = 0
    with open(path, "rb") as src:
        end = _ppm_header_end(src, magic)
        src.seek(end)
        with open(out_path, "wb") as dst:
            while True:
                chunk = src.read(1024 * 1024)
                if not chunk:
                    break
                digest.update(chunk)
                dst.write(chunk)
                written += len(chunk)
    return written, digest.hexdigest()


def _direct(im: Image.Image, out_path: str) -> tuple[int, str]:
    data = im.tobytes()
    with open(out_path, "wb") as fh:
        fh.write(data)
    return len(data), hashlib.sha256(data).hexdigest()


def convert_image(image_path: str, out_path: str) -> dict:
    """Write canonical raw pixels to `out_path` and return their metadata."""
    with Image.open(image_path) as im:
        im.load()
        info = FORMATS.get(im.mode)
        if info is None:
            raise ValueError(
                f"unsupported Pillow mode {im.mode!r}; expected one of {sorted(FORMATS)}"
            )
        pixel_format, bpp = info
        width, height = im.size
        expected = width * height * bpp

        if expected <= DIRECT_LIMIT or bpp in (2, 4):
            written, sha = _direct(im, out_path)
        else:
            tmp = out_path + ".ppm"
            im.save(tmp, format="PPM")
            try:
                written, sha = _stream(tmp, out_path)
            finally:
                os.remove(tmp)

    if written != expected:
        raise ValueError(f"canonical size mismatch: wrote {written}, expected {expected}")
    return {
        "source": image_path,
        "output": out_path,
        "width": width,
        "height": height,
        "pixel_format": pixel_format,
        "bytes": written,
        "sha256": sha,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("image")
    parser.add_argument("output")
    args = parser.parse_args()
    print(json.dumps(convert_image(args.image, args.output)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())