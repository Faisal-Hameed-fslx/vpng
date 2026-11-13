# ADR-0002: Tile directory entry layout

**Status:** accepted
**Date:** 2026-11-12
**Question (roadmap §18.2):** Final directory entry width and field order.

## Decision

A fixed **40-byte** entry, little-endian, with this field order:

| Offset | Size | Field |
|-------:|-----:|-------|
| 0 | 4 | tile x (pixel origin) |
| 4 | 4 | tile y (pixel origin) |
| 8 | 2 | tile width |
| 10 | 2 | tile height |
| 12 | 1 | codec id |
| 13 | 1 | predictor id |
| 14 | 2 | flags |
| 16 | 8 | payload offset |
| 24 | 8 | compressed length |
| 32 | 4 | uncompressed length |
| 36 | 4 | payload checksum (CRC32C) |

The directory is `tile_count * 40` bytes followed by a 4-byte CRC32C trailer.

## Rationale

* The roadmap warned that an earlier draft combined sizes ambiguously. This
  table sums to exactly 40 and every field has one meaning.
* `tile x`/`tile y` are pixel origins rather than grid indices so that edge
  tiles and, later, variable-size partitions are described directly.
* 32-bit `uncompressed length` is enough for a single tile: at 16-bit RGBA a
  65,535 × 65,535 tile would be 34 GB, but `Limits` caps decompressed size far
  below that, and tile edges are capped at 65,535 by the 16-bit width/height
  fields. A tile that large is rejected, not silently truncated.
* `flags` stays 0 in v0.1, leaving room for per-tile markers later.

## Consequences

* No file has been released, so no compatibility weight is carried (as the
  roadmap directs).
* If 64-bit uncompressed length is ever needed, the entry grows and the minor
  version bumps.
