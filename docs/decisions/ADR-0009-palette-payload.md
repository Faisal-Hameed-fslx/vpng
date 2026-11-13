# ADR-0009: Palette payload layout and canonical index packing

**Status:** accepted
**Date:** 2026-11-13
**Question (roadmap §6.2):** How is a Palette tile (codec id 1) laid out on
disk, and what makes one palette payload the canonical encoding of a tile?

## Decision

A Palette payload is:

```text
offset 0   u8   index-width code: 0=>1, 1=>2, 2=>4, 3=>8 bits; bits 2..7 reserved, zero
offset 1   u8   entry_count - 1   (so entry_count is 1..=256)
offset 2   ..   entry_count * bytes_per_pixel  palette entries, canonical pixel bytes
...        ..   ceil(pixel_count * index_bits / 8)  packed indices, MSB-first
```

* **Palette entries are the exact canonical pixel bytes** for the tile's format
  (for example four bytes per entry for RGBA8, eight for RGBA16). Alpha and
  every channel value survive verbatim; nothing is quantized.
* **Indices are packed MSB-first**, most significant bit of the first pixel in
  the most significant used bit of the first byte. Unused low bits of the final
  byte are zero.
* **Entry order is first-appearance order** in the tile's scan order. Combined
  with the fixed packing, this makes the payload deterministic: an encoder never
  depends on hash-map iteration order.

The payload is **canonical**, and a decoder rejects anything that is not:

1. The reserved header bits (bits 2..7 of byte 0) are zero, and the width code
   is one of the four defined values.
2. The declared index width is the **smallest** width that can address
   `entry_count` entries: 1 bit for 1–2 entries, 2 bits for 3–4, 4 bits for
   5–16, 8 bits for 17–256. A wider-than-necessary width is corrupt.
3. `entry_count` is 1..=256, so byte 1 is `entry_count - 1`.
4. Every index is `< entry_count`.
5. The payload length is exactly `2 + entry_count * bytes_per_pixel +
   ceil(pixel_count * index_bits / 8)`; there are no trailing bytes.
6. The padding bits of the final byte are zero.

Palette is **tile-local** (ADR-0004): the palette is built from the tile's own
pixels and no codec reads bytes outside its tile.

When a tile has **more than 256 distinct pixel values**, an empty pixel buffer,
or a payload that would exceed the configured per-tile payload limit, the codec
reports the tile as **not representable** (`UnsupportedFeature`). A cost-based
selector treats that as "skip this candidate" and another codec (ultimately Raw)
is used; it is not an input error.

## Rationale

* Encoding palette entries as whole canonical pixels means the codec needs no
  knowledge of channel layout beyond `bytes_per_pixel`, and cannot lose a bit of
  alpha or a low-order channel byte.
* Restricting the index width to four values and requiring the minimal width
  removes an entire class of ambiguity: given a tile, there is exactly one
  palette payload, so golden files and digests are stable.
* First-appearance entry order is the simplest deterministic rule; it also makes
  the byte sequence easy to reproduce by hand in tests.
* Falling back on an unrepresentable tile keeps the promise that a lossless
  encoding always exists: Raw is always available (roadmap §6.2, A06).

## Consequences

* The 1-byte `entry_count - 1` field caps a tile palette at 256 entries, which is
  the intent: larger tiles fall back instead of growing the palette.
* A decoder can compute the exact payload length from the header fields, the
  tile geometry, and the pixel format, so a truncated or over-long payload is
  detected without trusting any in-stream length.
* An encoder that wants to be canonical must assign entries in first-appearance
  order and choose the minimal width; both are implementation requirements, and
  both are covered by golden vectors in `vpng-codecs/src/palette.rs`.
* A future codec may define a different palette representation under a different
  codec id; id 1 is pinned to this layout.
