"""A minimal PNG writer and reader for the VPNG Python tests.

The tests must not depend on a third-party imaging library, and the CI job
must run with nothing but the standard library. So this module writes and
reads PNG by hand: `zlib` does the compression and `struct` does the
packing.

It supports the colour types and bit depths VPNG accepts:

* colour type 0 — grayscale
* colour type 2 — RGB
* colour type 4 — grayscale + alpha
* colour type 6 — RGBA
* bit depth 8 and 16

Two details matter and are handled explicitly:

* PNG stores 16-bit samples **big-endian**; VPNG's canonical pixels are
  **little-endian**. The reader and writer convert.
* PNG scanlines are filtered. The writer emits filter type 0 (None) for
  every row, but the reader implements all five filter types so it can read
  a file written by any other tool.
"""

from __future__ import annotations

import struct
import zlib
from pathlib import Path

SIGNATURE = b"\x89PNG\r\n\x1a\n"

#: The colour type each channel count means.
COLOR_TYPES = {1: 0, 2: 2, 3: 4, 4: 6}


def _chunk(kind: bytes, data: bytes) -> bytes:
    """One PNG chunk: length, type, payload, CRC."""
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    )


def _paeth(a: int, b: int, c: int) -> int:
    """The PNG Paeth predictor."""
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    if pb <= pc:
        return b
    return c


def write_png(
    path: str | Path,
    width: int,
    height: int,
    pixels: bytes,
    channels: int = 4,
    bit_depth: int = 8,
) -> None:
    """Write canonical pixels to a PNG.

    ``pixels`` are in VPNG's canonical order: rows tightly packed, 16-bit
    samples little-endian. The writer converts them to PNG's order.
    """
    if channels not in COLOR_TYPES:
        raise ValueError(f"unsupported channel count: {channels}")
    if bit_depth not in (8, 16):
        raise ValueError(f"unsupported bit depth: {bit_depth}")
    color_type = COLOR_TYPES[channels]
    sample_bytes = bit_depth // 8
    expected = width * height * channels * sample_bytes
    if len(pixels) != expected:
        raise ValueError(f"expected {expected} bytes, got {len(pixels)}")

    # Canonical (little-endian) to PNG (big-endian) for 16-bit samples.
    data = bytearray(pixels)
    if bit_depth == 16:
        for offset in range(0, len(data), 2):
            data[offset], data[offset + 1] = data[offset + 1], data[offset]

    row_bytes = width * channels * sample_bytes
    raw = bytearray()
    for row in range(height):
        raw.append(0)  # filter type 0: None
        start = row * row_bytes
        raw += data[start : start + row_bytes]

    header = struct.pack(
        ">IIBBBBB",
        width,
        height,
        bit_depth,
        color_type,
        0,  # compression: deflate
        0,  # filter method: adaptive
        0,  # interlace: none
    )
    body = (
        SIGNATURE
        + _chunk(b"IHDR", header)
        + _chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + _chunk(b"IEND", b"")
    )
    Path(path).write_bytes(body)


def read_png(path: str | Path) -> tuple[int, int, bytes, int, int]:
    """Read a PNG into canonical pixels.

    Returns ``(width, height, pixels, channels, bit_depth)`` where ``pixels``
    are in VPNG's canonical order.
    """
    body = Path(path).read_bytes()
    if not body.startswith(SIGNATURE):
        raise ValueError("not a PNG file")

    offset = len(SIGNATURE)
    width = height = 0
    bit_depth = color_type = 0
    idat = bytearray()
    while offset < len(body):
        (length,) = struct.unpack(">I", body[offset : offset + 4])
        kind = body[offset + 4 : offset + 8]
        payload = body[offset + 8 : offset + 8 + length]
        offset += 12 + length
        if kind == b"IHDR":
            width, height, bit_depth, color_type = struct.unpack(">IIBB", payload[:10])
        elif kind == b"IDAT":
            idat += payload
        elif kind == b"IEND":
            break

    if color_type not in (0, 2, 4, 6):
        raise ValueError(f"unsupported colour type: {color_type}")
    if bit_depth not in (8, 16):
        raise ValueError(f"unsupported bit depth: {bit_depth}")

    channels = {0: 1, 2: 3, 4: 2, 6: 4}[color_type]
    sample_bytes = bit_depth // 8
    row_bytes = width * channels * sample_bytes
    pixel_bytes = row_bytes * height

    raw = zlib.decompress(bytes(idat))
    out = bytearray(pixel_bytes)
    previous = bytearray(row_bytes)
    position = 0
    for row in range(height):
        filter_type = raw[position]
        position += 1
        line = bytearray(raw[position : position + row_bytes])
        position += row_bytes
        for index in range(row_bytes):
            left = line[index - channels] if index >= channels else 0
            up = previous[index]
            upper_left = previous[index - channels] if index >= channels else 0
            value = line[index]
            if filter_type == 0:
                pass
            elif filter_type == 1:
                value = (value + left) & 0xFF
            elif filter_type == 2:
                value = (value + up) & 0xFF
            elif filter_type == 3:
                value = (value + ((left + up) >> 1)) & 0xFF
            elif filter_type == 4:
                value = (value + _paeth(left, up, upper_left)) & 0xFF
            else:
                raise ValueError(f"unsupported filter type: {filter_type}")
            line[index] = value
        out[row * row_bytes : (row + 1) * row_bytes] = line
        previous = line

    data = bytes(out)
    if bit_depth == 16:
        swapped = bytearray(data)
        for offset in range(0, len(swapped), 2):
            swapped[offset], swapped[offset + 1] = swapped[offset + 1], swapped[offset]
        data = bytes(swapped)
    return width, height, data, channels, bit_depth
