# VPNG container specification v0.2

**Status:** draft, implemented by `vpng-container` v0.2.0 (the workspace is
still versioned 0.1.0; the *format* minor version is what this document
defines)
**Extension:** `.vpng`
**Byte order:** little-endian throughout
**Predecessor:** `docs/VPNG_SPEC_0_1.md`

> **What v0.2 changes, in one line.** The 64-byte header and the 40-byte
> directory entry keep their exact layouts; v0.2 changes two *rules* — the
> directory entry's `flags` field carries the pyramid level, and a level's
> tiles may have different sizes instead of forming one uniform grid.

This document is normative for the v0.2 implementation. Where a design
question was open, the resolved answer is recorded in `docs/decisions/`. If
code and this document disagree, that is a bug in one of them; file it, do not
paper over it.

---

## 0. Relationship to v0.1

v0.2 is an **extension**, not a redesign:

| | v0.1 | v0.2 |
|---|---|---|
| main header | 64 bytes, fixed layout | **unchanged** |
| directory entry | 40 bytes, fixed layout | **unchanged** |
| file layout | header, directory, payloads | **unchanged** |
| codecs | ids 0–3 | **unchanged** |
| minor version | 1 | 2 |
| feature flags | must be 0 | bit 0 = pyramid |
| entry `flags` | must be 0 | low byte = level index |
| tiles in a level | one uniform grid | an exact partition; sizes may vary |
| levels | exactly one | one or more |

A v0.2 file that uses neither a pyramid nor an adaptive partition is a file
whose only difference from v0.1 is the version byte and the header checksum
that covers it. A v0.1 file remains valid, readable and byte-identical: the
default encoder path still writes v0.1 bytes, and the golden fixtures pin
them.

### 0.1 Compatibility rules

| Situation | Required behaviour |
|---|---|
| A v0.2 reader reads a v0.1 file | Accept it. It has one level: a uniform grid (ADR-0005), derived exactly as v0.1 defines |
| A v0.1 reader reads a v0.2 file | Reject it with `UnsupportedFeature`: the minor version is newer than the reader knows |
| A v0.1 file with any feature flag set | Reject it with `UnsupportedFeature`: v0.1 defines no feature flags |
| A v0.2 file with an unknown feature-flag bit set | Reject it with `UnsupportedFeature` |
| A v0.2 file whose directory holds a level 0 and nothing else | Accept it. It is a v0.2 file with one level and no pyramid flag |
| A v0.2 file with the pyramid flag but one level | Reject it as corrupt: the flag and the levels must agree |

---

## 1. File layout

A v0.2 file is exactly three regions, in this order — the same three regions
v0.1 has:

```text
+----------------------------+  offset 0
| main header (64 bytes)     |
+----------------------------+  offset 64
| tile directory             |  N * 40 bytes + 4-byte CRC32C
+----------------------------+  end of directory
| tile payloads, back to back|
+----------------------------+  end of file
```

Rules 1–7 of v0.1 §1 apply unchanged. In particular: no implicit padding, no
alignment requirement, `declared_file_size` equals the real file length, and
there is no second authority for the directory.

The **only** structural difference is that the directory's entries may belong
to more than one level, and the payloads of all levels are written back to
back in directory order.

---

## 2. Main header (64 bytes)

The layout is v0.1's, byte for byte:

| Offset | Size | Field             | Notes |
|-------:|-----:|-------------------|-------|
| 0      | 8    | magic             | `56 50 4E 47 0D 0A 1A 0A` |
| 8      | 2    | major version     | must be 0 |
| 10     | 2    | minor version     | 1 or 2 |
| 12     | 4    | header size       | must be 64 |
| 16     | 4    | width             | pixels, non-zero |
| 20     | 4    | height            | pixels, non-zero |
| 24     | 1    | channel count     | must equal color model's channel count |
| 25     | 1    | bits per channel  | 8 or 16 |
| 26     | 1    | color model id    | v0.1 §3 |
| 27     | 1    | default codec id  | informational (ADR-0003) |
| 28     | 4    | tile count        | number of directory entries, **all levels** |
| 32     | 8    | directory offset  | byte offset from file start |
| 40     | 8    | declared file size| exact total file length |
| 48     | 4    | feature flags     | see below |
| 52     | 4    | header checksum   | CRC32C over `0..52` and `56..64` (ADR-0001) |
| 56     | 8    | reserved          | must be zero |

The header checksum rule is unchanged, so the feature flags are covered by it:
a file cannot be edited into a pyramid without rewriting the header.

### 2.1 Feature flags

| Bit | Name            | Meaning |
|----:|-----------------|---------|
| 0   | `FEATURE_PYRAMID` | the file holds derived preview levels above the original |
| 1–31| reserved       | must be zero; a reader rejects unknown bits |

`FEATURE_PYRAMID` is set **exactly when** the directory holds more than one
level. A reader checks both directions: a flag without a pyramid, and a
pyramid without a flag, are corrupt.

The flags say nothing about the *tile sizes*. A v0.2 file with one level and
no pyramid flag may still hold tiles of different sizes: the partition rules
in §4.3 cover that case, and the flag is only about levels.

### 2.2 Validation order

v0.1 §2.2 applies, with one addition: after the feature-flag check, a v0.1
file (minor version 1) with **any** feature flag set is rejected. The check is
separate from the unknown-bit check because the two reject different
mistakes: the unknown-bit check rejects a bit nobody has defined, and this
one rejects a *known* bit in a version that did not define it.

---

## 3. Pixel formats

Unchanged: v0.1 §3 applies in full, including the canonical pixel buffer and
the rule that alpha is a plain channel.

---

## 4. Tile directory

The directory is a flat array of `tile_count` fixed 40-byte entries, followed
by a 4-byte CRC32C over the entry bytes (ADR-0002).

### 4.1 Entry layout (40 bytes)

| Offset | Size | Field               | Notes |
|-------:|-----:|---------------------|-------|
| 0      | 4    | tile x              | pixel origin, not a tile index |
| 4      | 4    | tile y              | pixel origin |
| 8      | 2    | tile width          | pixels, non-zero |
| 10     | 2    | tile height         | pixels, non-zero |
| 12     | 1    | codec id            | authoritative for this tile (ADR-0003) |
| 13     | 1    | predictor id        | 0 when the codec needs no predictor |
| 14     | 2    | flags               | see 4.2 |
| 16     | 8    | payload offset      | byte offset from file start |
| 24     | 8    | compressed length   | payload bytes on disk |
| 32     | 4    | uncompressed length | must equal `width * height * bytes_per_pixel` |
| 36     | 4    | payload checksum    | CRC32C over the payload bytes |

Every field except `flags` keeps v0.1's meaning exactly.

### 4.2 The `flags` field

```text
bits 0..7   level index       (0 = the original image, 1..255 = derived previews)
bits 8..15  reserved          must be zero
```

* The level index is the level the tile belongs to. Level 0 is the original
  image; every other level holds **derived preview pixels**.
* A derived level is never used to reconstruct the original. The original is
  always level 0, and it is always exact.
* The reserved bits must be zero. A reader rejects a file whose entry sets
  one of them, as corrupt. A v0.1 file (minor version 1) must have `flags ==
  0` for every entry.

### 4.3 Canonical order

The entries are sorted by **`(level, y, x)`**: level first, then `tile y`,
then `tile x`. Within a level this is v0.1's canonical order, and for a
v0.1 file — which has one level — it is exactly v0.1's order.

A reader checks the order and rejects a file whose entries are not sorted: a
reader must be able to find a level's tiles without scanning the whole
directory.

### 4.4 Levels

A **level** is one set of tiles. Level 0 is the original image; every other
level is a derived preview. The levels of a file are derived from its
directory, not stored separately — there is no per-level metadata region.

The derivation rules:

1. The entries are in canonical `(level, y, x)` order.
2. The entries of one level form an **exact partition** of that level's
   rectangle: every pixel is covered exactly once, no gap and no overlap.
3. A level's rectangle is the bounding box of its tiles, anchored at the
   origin `(0, 0)`.
4. Level 0's rectangle is the image: its width and height are the header's
   width and height.
5. A derived level's rectangle is **strictly smaller in both dimensions**
   than the level below it.
6. The level indices are consecutive from 0: the first level is 0 and each
   following level is the previous one plus one.

### 4.5 Partitions

A level's tiles are a **partition**: a set of axis-aligned rectangles that
covers the level's rectangle exactly once.

Unlike v0.1, the tiles of a partition **may have different sizes**. There is
no nominal tile size in the format; the nominal size is a property of the
*encoder's* choice, not of the file.

A conforming partition satisfies:

1. Every tile lies entirely within the level's rectangle.
2. No two tiles overlap.
3. Every pixel of the rectangle is covered by exactly one tile.
4. Every tile has a non-zero width and height.

A reader validates the partition with a sweep line over the tiles' vertical
bands: for each band of constant coverage, the horizontal intervals must tile
the band's width exactly, with no gap and no overlap. The check is `O(n log
n)` in the number of tiles.

A v0.1 file's single level is a **uniform** partition: all tiles have the
same size, except that tiles in the last column may be narrower and tiles in
the last row may be shorter. v0.1 §4.4 (ADR-0005) remains the rule for
deriving that grid, and a v0.1 file whose entries cannot be explained by one
uniform grid is corrupt.

---

## 5. Codec identifiers

Unchanged: v0.1 §5 applies in full, including the rule that a reserved or
unknown codec id makes the file `UnsupportedFeature`.

A tile's codec is chosen independently of its size, so a level whose tiles
have different sizes may use different codecs. Nothing in §5 depends on the
tile being square or on a fixed size.

---

## 6. The preview pyramid

### 6.1 What a level is allowed to be

The **format** says only what §4.4 says: level 0 is exact, a derived level
is smaller than the level below it, and every level is an exact partition.
The format does **not** say how a derived level's pixels are computed.

That is deliberate. A decoder never needs the rule: it decodes the pixels
that are in the file. The rule below is the **reference encoder's** choice,
so that two encoders produce the same file for the same picture.

### 6.2 The reference downsampling

The reference encoder builds a preview level from the level below it with a
2 × 2 box average:

* The level below has width `W` and height `H`; the preview has width
  `ceil(W / 2)` and height `ceil(H / 2)`.
* Preview pixel `(x, y)` is the average of the up-to-four source pixels
  `(2x, 2y)`, `(2x+1, 2y)`, `(2x, 2y+1)` and `(2x+1, 2y+1)` that exist. An
  odd edge contributes one pixel instead of two.
* The average is computed on the **sample** values, not on the bytes: a
  16-bit channel is averaged as a 16-bit value.
* The division is **round-half-up** on a non-negative integer: the sum of the
  samples plus half the count, divided by the count.
* Every channel is averaged the same way, **including alpha**. There is no
  premultiplication.

The computation uses **integer arithmetic only**. No floating point is used
anywhere in the encoder, so the result is identical on every machine.

### 6.3 The chain

The reference encoder builds up to `pyramid_levels` derived levels, each from
the one below it. The chain stops early when the next level would not be
strictly smaller in both dimensions — for example, a 3 × 2 image has a 2 × 1
preview, and the next one would be 1 × 1, which is not smaller in height.

The preview levels use the **same tile size** as the original, and their
tiles form uniform grids over the preview's own rectangle.

### 6.4 What a preview is not

* A preview is **not** a lossless copy of the original and is not used to
  reconstruct it. The original is level 0, and level 0 is exact.
* The preview's pixels are *derived*: they are what the box average produces,
  and nothing else. A decoder that reads only a preview has a smaller
  picture, not a damaged one.

---

## 7. Adaptive tiling

### 7.1 What the format requires

Only §4.5: an exact partition. The format does **not** require the tiles to
have different sizes, and it does not require any particular planner. A file
whose tiles all happen to have the same size is a valid v0.2 file.

### 7.2 The reference planner

The reference encoder's planner is a policy, not a format rule. It is
described here so that a second implementation can reproduce the same file.

* The **nominal tile size** is the requested tile size (256 × 256 by
  default). The **maximum edge** is twice the nominal edge, and the
  **minimum edge** is half the nominal edge.
* The picture is first cut into a **coarse** partition: the whole picture is
  split along its longer axis until every tile is at most the maximum edge on
  a side. Each split's first part is a multiple of the minimum edge, so the
  tiles line up with a regular lattice.
* Each rectangle is scored by the **mean absolute difference between
  neighbouring pixels** — horizontal and vertical — over the colour channels,
  sampled on a lattice of at most 64 × 64 points and rounded down. The alpha
  channel is excluded from the score: transparency is a mask, not colour.
* A rectangle whose score is above the threshold (8 by default, on a 0–255
  scale) is **busy** and is split into two, along its longer axis; the
  splitting repeats until the parts reach the minimum edge or are calm
  enough.
* The busiest rectangle is split first, and among equally busy rectangles the
  top-left one is. The splitting stops when the budget is reached: **twice**
  the number of tiles a uniform grid of the picture would use.
* The resulting tiles are sorted by `(y, x)`.

The planner is deterministic: the same picture and the same options produce
the same partition, and therefore the same file.

### 7.3 The cost of the planner

The planner needs the picture's content, so an encoder that uses it holds the
whole image in memory while it plans. The **default** path — a uniform grid,
no pyramid — never does, and keeps its bounded-memory streaming property.

---

## 8. Editing a v0.2 file

An edit changes pixels, so it invalidates the derived levels: they were
computed from pixels the edit has changed, and the editor cannot rebuild them
without re-reading the whole original.

The editing engine therefore **drops the previews** on commit:

* the committed file holds level 0 only;
* the header's `FEATURE_PYRAMID` bit is cleared;
* the committed file's minor version is 1, because a v0.1 file is exactly a
  file with one uniform level — and the base level of a v0.2 file is uniform
  unless the file was also adaptive;
* the tiles that were not edited keep their **payload bytes** exactly;
* the report says how many preview levels were dropped.

A file that was encoded with an adaptive partition keeps its adaptive base
level, because the partition is a property of the level, not of the previews.

---

## 9. Conformance checklist

A v0.2 implementation is conforming when it:

1. Writes a v0.1 file when neither a pyramid nor an adaptive partition is
   requested, byte for byte what v0.1 writes.
2. Writes a v0.2 file with `minor version == 2` when either is requested.
3. Sets `FEATURE_PYRAMID` exactly when the directory holds more than one
   level.
4. Sorts the directory by `(level, y, x)`.
5. Puts the level index in the low byte of each entry's `flags` and zero in
   the reserved bits.
6. Keeps level 0 exact, and never derives the original from a preview.
7. Produces an exact partition for every level, and rejects a file whose
   partition has a gap or an overlap.
8. Rejects a derived level that is not strictly smaller than the level below
   it.
9. Rejects a v0.1 file with a feature flag, and a v0.2 file with an unknown
   feature-flag bit.
10. Reads a v0.1 file exactly as v0.1 defines.

The test suite pins these rules in
`crates/vpng-testkit/tests/v2_formats.rs` and
`crates/vpng-cli/tests/v2_features.rs`.
