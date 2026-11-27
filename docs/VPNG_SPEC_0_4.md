# VPNG container specification v0.4

**Status:** draft, implemented by `vpng-container` v0.4.0 (the workspace is
still versioned 0.1.0; the *format* minor version is what this document
defines)
**Extension:** `.vpng`
**Byte order:** little-endian throughout
**Predecessors:** `docs/VPNG_SPEC_0_3.md`, `docs/VPNG_SPEC_0_2.md`,
`docs/VPNG_SPEC_0_1.md`

> **What v0.4 changes, in one line.** The 64-byte header and the 40-byte
> directory entry keep their exact layouts; v0.4 gives the header's last
> eight reserved bytes a job (they now point at a **layer table**), gives
> four bits of the directory entry's `flags` field a job (they now name the
> **layer** a tile belongs to), and adds a table between the directory and
> the payloads that names the layers.

This document is normative for the v0.4 implementation. Where a design
question was open, the resolved answer is recorded in `docs/decisions/`. If
code and this document disagree, that is a bug in one of them; file it, do not
paper over it.

---

## 0. Relationship to v0.3, v0.2 and v0.1

v0.4 is an **extension**, not a redesign:

| | v0.1 | v0.2 | v0.3 | v0.4 |
|---|---|---|---|---|
| main header | 64 bytes, fixed layout | **unchanged** | **unchanged** | **unchanged** (bytes 56..64 now carry the layer table's location) |
| directory entry | 40 bytes, fixed layout | **unchanged** | **unchanged** | **unchanged** (`flags` gains four bits) |
| file layout | header, directory, payloads | **unchanged** | **unchanged** | header, directory, **layer table**, payloads |
| codecs | ids 0–3 | ids 0–3 | ids 0–6 | **unchanged** (ids 0–6) |
| minor version | 1 | 2 | 3 | 4 |
| feature flags | must be 0 | bit 0 = pyramid | **unchanged** | **bit 1 = layers** (bit 0 = pyramid) |
| entry `flags` | must be 0 | low byte = level index | **unchanged** | low byte = level index, next four bits = layer index |
| tiles in a level | one uniform grid | an exact partition; sizes may vary | **unchanged** | **unchanged** |
| levels | exactly one | one or more | **unchanged** | **unchanged** |
| layers | exactly one (the image) | exactly one | exactly one | **one or more, named** |
| masks | none | none | none | **a layer may name a mask layer** |

Everything v0.3 defines — the promoted codecs, the version rule, the codec
sets — remains in force in v0.4. Everything v0.2 defines — the preview
pyramid, the adaptive partition, the level rules — also remains in force,
with **one exception**: a file that carries a layer table may not also carry a
preview pyramid. §4 explains why.

### 0.1 Compatibility rules

| Situation | Required behaviour |
|---|---|
| A v0.4 reader reads a v0.1 file | Accept it, exactly as v0.1 defines; the file has one layer, the base layer |
| A v0.4 reader reads a v0.2 file | Accept it, exactly as v0.2 defines; one layer, with the pyramid in it |
| A v0.4 reader reads a v0.3 file | Accept it, exactly as v0.3 defines; one layer |
| A v0.1, v0.2 or v0.3 reader reads a v0.4 file | **It cannot know the file is a v0.4 file**: the minor version is 4, so it rejects it with `UnsupportedFeature` — the minor version is newer than the reader knows |
| A v0.4 file carries a layer table | Accept it, and derive the layers from the table and the directory |
| A file with minor version below 4 declares a layer table | Reject it with `CorruptFile`: that version never defined a layer table |
| A file with minor version 4 or newer has no layer table | Accept it: it is a flat file with one layer |
| A file names an id in `0x80..=0xFF` | Reject it with `UnsupportedFeature`: those ids are research-only and may never reach a file |

### 0.2 The minor version states what the file **contains**

The minor version is a record of what the file *is*, not of what the encoder
was permitted to do. The rule is extended, not replaced:

1. if the file carries a **layer table**, the file is a **v0.4** file and its
   minor version is 4;
2. otherwise, if any directory entry names codec id 4, 5 or 6, the file is a
   **v0.3** file and its minor version is 3;
3. otherwise, if the file uses a v0.2 feature (a preview pyramid or an
   adaptive partition), the file is a **v0.2** file and its minor version is 2;
4. otherwise the file is a **v0.1** file and its minor version is 1.

The default encoder path writes no layer table, permits only ids 0–3 and
requests neither a pyramid nor an adaptive partition, so it keeps writing the
exact v0.1 bytes the golden fixtures pin. **v0.4 moves no published byte.**

---

## 1. File layout

```text
+----------------------------+  offset 0
| main header (64 bytes)     |
+----------------------------+  offset 64
| tile directory             |  N * 40 bytes + 4-byte CRC32C
+----------------------------+  end of directory
| layer table                |  only when the header says so
+----------------------------+  end of layer table
| tile payloads, back to back|
+----------------------------+  end of file
```

A file with no layer table has exactly the v0.1 layout: the payloads start
where the directory ends.

---

## 2. Main header (64 bytes)

The layout is v0.1's, byte for byte. The only field whose **value** differs
from v0.1 is the minor version, which may now be 1, 2, 3 or 4, and the last
eight bytes, which were reserved.

| Offset | Size | Field             | Notes |
|-------:|-----:|-------------------|-------|
| 0      | 8    | magic             | `56 50 4E 47 0D 0A 1A 0A` |
| 8      | 2    | major version     | must be 0 |
| 10     | 2    | minor version     | 1, 2, 3 or 4 |
| 12     | 4    | header size       | must be 64 |
| 16     | 4    | width             | pixels, non-zero |
| 20     | 4    | height            | pixels, non-zero |
| 24     | 1    | channel count     | must equal color model's channel count |
| 25     | 1    | bits per channel  | 8 or 16 |
| 26     | 1    | color model id    | v0.1 §3 |
| 27     | 1    | default codec id  | informational (ADR-0003) |
| 28     | 4    | tile count        | number of directory entries, **all layers** |
| 32     | 8    | directory offset  | byte offset from file start |
| 40     | 8    | declared file size| exact total file length |
| 48     | 4    | feature flags     | §2.1 |
| 52     | 4    | header checksum   | CRC32C over `0..52` and `56..64` (ADR-0001) |
| 56     | 4    | layer table offset| byte offset from file start; zero when there is no table |
| 60     | 4    | layer table length| the table's length in bytes; zero when there is no table |

The header checksum rule is unchanged, so the minor version, the feature flags
and the layer table's location are all covered by it: a file cannot be edited
from v0.1 into v0.4 without rewriting the header.

### 2.1 Feature flags

| bit | mask | meaning |
|---:|------|---------|
| 0 | `0x0000_0001` | `FEATURE_PYRAMID` — the file has derived preview levels (v0.2) |
| 1 | `0x0000_0002` | `FEATURE_LAYERS` — the file has a layer table (v0.4) |
| 2..31 | | reserved; must be zero |

The two feature flags are **mutually exclusive**: a file may not have both a
preview pyramid and a layer table. §4 explains why, and a reader that finds
both rejects the file with `CorruptFile`.

`FEATURE_LAYERS` is set **iff** the layer table length is non-zero, and the
layer table offset is zero **iff** the length is zero. A reader checks both
halves; a file that claims a table and does not have one, or has one and does
not claim it, is corrupt.

A file whose minor version is below 4 must have both fields zero and must not
set `FEATURE_LAYERS`. Bytes 56..64 were reserved in v0.1–v0.3 and those
versions required them to be zero, so the reserved bytes of an old file are
still exactly zero and no old file is affected.

---

## 3. Tile directory

The entry layout is v0.1's, byte for byte. The `flags` field (offset 22,
`u16`) gains a second job.

| Offset | Size | Field |
|-------:|-----:|-------|
| 0      | 4    | x |
| 4      | 4    | y |
| 8      | 2    | width |
| 10     | 2    | height |
| 12     | 1    | codec id |
| 13     | 1    | predictor id |
| 14     | 2    | flags |
| 16     | 8    | payload offset |
| 24     | 8    | compressed length |
| 32     | 4    | uncompressed length |
| 36     | 4    | payload checksum (CRC32C) |

### 3.1 The `flags` field in v0.4

| bits | mask | meaning |
|------|------|---------|
| 0..7 | `0x00FF` | the tile's **level index** (v0.2) |
| 8..11 | `0x0F00` | the tile's **layer index** (v0.4) |
| 12..15 | `0xF000` | reserved; must be zero in a v0.4 file |

In a v0.2 or v0.3 file, bits 8..15 are reserved and must be zero, exactly as
they were in v0.1. A reader applies the rule that matches the file's minor
version, so a v0.2 file cannot accidentally carry a layer index and a v0.4
file cannot carry a stray reserved bit.

The layer index is four bits, so a file holds at most **16** layers. The
encoder refuses a seventeenth; a reader rejects a table that claims more.

### 3.2 Canonical order

The entries of a v0.4 file are in canonical **`(layer, level, y, x)`** order,
and each layer's entries are contiguous. For a file with no layer table the
layer index is 0 for every entry, so the order is exactly v0.2's
`(level, y, x)` order and **nothing moves**.

A reader rejects a file whose entries are out of canonical order with
`CorruptFile`: a reader must be able to find a layer's tiles without scanning
the whole directory.

---

## 4. The layer table

### 4.1 Where it sits and why

The table sits **between the directory and the payloads**. The payloads
therefore start at `layer_table_offset + layer_table_length`, and a reader
computes that from the header rather than by guessing.

A layered file has **no preview pyramid**. Two reasons:

* the accounting would otherwise be ambiguous — a layer's "uncompressed
  bytes" would have to say whether it includes the derived levels;
* the pyramid exists to make a *large flat picture* cheap to preview, and a
  layered file is already a set of independently decodable grids, each of
  which can be previewed by itself.

So `FEATURE_PYRAMID` and `FEATURE_LAYERS` are mutually exclusive, and a
layered file's every directory entry is at level 0.

### 4.2 The table header

```text
offset  size  field
     0     4  layer count
     4     4  descriptor array length
     8     4  CRC32C over the descriptor array
    12     4  reserved (must be zero)
    16   ...  descriptor array
```

The descriptor array length must be exactly the number of bytes between the
end of this header and the end of the table: no more, no fewer. The
checksum covers the descriptor array only.

A table must hold at least one layer, and at most 16.

### 4.3 One descriptor

```text
offset  size  field
     0     1  name length in bytes
     1     1  kind flags (bit 0: this layer is a mask)
     2     2  reserved (must be zero)
     4     4  x
     8     4  y
    12     4  width
    16     4  height
    20     1  opacity (0..255)
    21     1  reserved (must be zero)
    22     2  mask layer index (0xFFFF: no mask)
    24     8  uncompressed pixel bytes
    32     8  compressed payload bytes
    40   ...  name (UTF-8)
```

The name is UTF-8, at most 255 bytes, and may be empty. It is **not** required
to be unique: two layers may share a name, and the index is what identifies a
layer.

The kind flags byte has one bit defined:

| bit | meaning |
|---:|---|
| 0 | this layer is a **mask** |
| 1..7 | reserved; must be zero |

The rectangle is the layer's place on the canvas, in pixels. The opacity is
0 (invisible) to 255 (opaque).

The mask index names the layer that carries this layer's mask, or `0xFFFF`
for none. A layer names **at most one** mask.

The byte counts are the layer's **own**: its level-0 pixels, and every payload
it owns. Because a layered file has no pyramid, these two numbers are exact
and need no second rule.

---

## 5. The rules a reader enforces

A reader rejects a v0.4 file with `CorruptFile` when any of these is false:

1. the layer count is between 1 and 16;
2. the descriptor array length matches the bytes present, exactly;
3. the descriptor array's CRC32C matches the stored checksum;
4. the table's reserved word is zero;
5. every descriptor's reserved bytes are zero;
6. every name is valid UTF-8;
7. every layer's rectangle lies inside the canvas;
8. the base layer (index 0) starts at the canvas origin and **covers the whole
   canvas**;
9. the base layer is not a mask;
10. every layer has at least one tile, and the directory holds no tile that
    belongs to no layer;
11. the entries are in canonical `(layer, level, y, x)` order;
12. every entry of a layered file is at level 0;
13. each layer's tiles are an **exact partition** of the layer's rectangle:
    they cover it exactly once, with no gap and no overlap;
14. the bounding box of a layer's tiles is exactly the rectangle the table
    declares;
15. each layer's uncompressed and compressed byte counts match the directory
    exactly;
16. a layer that names a mask names an existing layer, that layer is a mask,
    it is not the layer itself, and its rectangle **covers** the layer's
    rectangle;
17. the file does not set both `FEATURE_PYRAMID` and `FEATURE_LAYERS`;
18. a file with minor version below 4 has no layer table.

### 5.1 The two coordinate systems

A layer's tiles carry **canvas** coordinates, so a reader can place them
without knowing where the layer sits. The layer's **pixels** live in the
layer's own rectangle, so the pixels of a layer whose rectangle starts at
`(8, 8)` and is 32×16 are a 32×16 image, not a 40×24 one.

The two meet in exactly one place: the encoder wraps the layer's pixel source
in a translating source whose origin is the layer rectangle's `(x, y)`. The
decoder's `decode_layer` returns a buffer sized to the layer's **own**
rectangle.

### 5.2 The byte accounting is exact

For every layer, the reader recomputes two numbers from the directory and
compares them with the table's:

* `uncompressed_bytes` = the sum of the layer's entries' uncompressed lengths
  — that is, the layer's level-0 pixel bytes;
* `compressed_bytes` = the sum of the layer's entries' compressed lengths —
  that is, every payload byte the layer owns.

A mismatch is `CorruptFile`. This is what makes the per-layer ratio in §7.3 a
measurement rather than an estimate.

---

## 6. Masks

A **mask** is a layer whose kind flag says it is one. Its **alpha channel** is
the coverage: 0 means the layer it masks is invisible there, 255 means fully
visible.

* A file that has a mask must use a format with an 8-bit alpha channel:
  `graya8` or `rgba8`. A format without an alpha channel cannot carry a mask,
  and the encoder refuses to write one.
* A mask layer is **never composited on its own**. It is coverage for another
  layer, not a picture.
* A layer references at most one mask, and a mask layer may be referenced by
  any number of layers.
* The mask's rectangle must **cover** the rectangle of the layer it masks.
  The mask may be larger; it may not be smaller.

The mask's colour channels carry no meaning and are not read.

---

## 7. Compositing

### 7.1 The model

The blend is **straight alpha** "source over". The source pixel is laid over
the destination in proportion to

```text
w = source_alpha * opacity * coverage / (max * 255 * max)
```

where `max` is 255 for an 8-bit format and 65535 for a 16-bit one. `coverage`
is the mask's alpha over the pixel, or `max` when the layer has no mask.

The **colour** channels blend as

```text
out = (source * numerator + destination * (denominator - numerator)) / denominator
```

and the **alpha** channel blends on its own, as the weight itself:

```text
out_alpha = (max * numerator + destination_alpha * (denominator - numerator)) / denominator
```

The weights are carried as a **fraction**, not as a floating-point number, so
the result does not depend on the host's floating-point behaviour. Every
operation is integer, so the same file composites to the same bytes on every
platform.

### 7.2 The base layer is exact

A file with **one** layer composites to **exactly** that layer's pixels. The
blend of a single layer is the identity, not an approximation, and no rounding
creeps in. This is pinned by the test suite and measured over the whole
750-image corpus.

The encoder also composites the base layer in, so a file's composite starts
from the base layer's pixels rather than from a blank canvas.

### 7.3 The per-layer ratio

Each layer reports its own ratio:

```text
ratio = uncompressed_bytes / compressed_bytes
```

A ratio above 1 means the layer got smaller; 1 means it did not; below 1
means it grew, which the codecs are allowed to do and the encoder never hides.

---

## 8. Encoding a layered file

### 8.1 The rules an encoder enforces

1. At least one layer, at most 16.
2. The base layer is layer 0: it is not a mask, it starts at the origin, and
   it covers the whole canvas.
3. Every layer uses the **same pixel format** as the file.
4. Every layer's pixel source is exactly the size of its rectangle.
5. Every layer's rectangle lies inside the canvas.
6. A layered file may not ask for a preview pyramid or an adaptive partition.
7. A mask layer requires a format with an 8-bit alpha channel.
8. A layer that names a mask names an existing mask layer, not itself, and
   the mask's rectangle covers the layer's rectangle.

### 8.2 Layers are encoded independently

Each layer's tiles are encoded on their own, with the same per-tile codec
selection as any other file. **No layer predicts from another layer**, and no
state crosses a layer boundary: cross-layer prediction is explicitly out of
scope for v0.4.

The consequence is a useful property: a layer can be re-encoded, or decoded,
without touching the others, and the base layer's bytes do not move when an
overlay is added.

### 8.3 Determinism

The same layers, the same pixels and the same options produce the same file,
byte for byte, whatever the worker count and whether the file was written by
the batch encoder or the streaming writer. The worker count changes the speed
and not the bytes.

---

## 9. Editing a v0.4 file

The editing engine **refuses** a file that carries a layer table. The reason
is stated plainly in the refusal: the engine's commit path rewrites the
directory and the payloads, and it does not yet know how to carry the layer
table and the per-layer accounting through that rewrite. Silently dropping the
layers would lose the user's work, so the engine refuses instead.

A file with no layer table is edited exactly as v0.3 defines: an edit
invalidates the derived levels, the committed file holds level 0 only, the
`FEATURE_PYRAMID` bit is cleared, the tiles that were not edited keep their
**payload bytes** exactly, and the report says how many preview levels were
dropped.

---

## 10. Migration policy

* **Readers** must accept a v0.1, v0.2, v0.3 or v0.4 file. A reader that does
  not implement the layer table rejects a v0.4 file with
  `UnsupportedFeature` — the ordinary "newer than I know" refusal — and must
  not attempt a best-effort decode.
* **Writers** default to the flat path, so the bytes a published encoder
  produces do not change. The layered path is opt-in.
* **A v0.4 file with one layer** is a valid v0.4 file, and a v0.3 reader
  rejects it for the ordinary reason: the minor version is newer than the
  reader knows. It is not a special case.
* **Research ids** (`0x80..=0xFF`) may never reach a file. They exist in the
  research harness only, and the production decoder rejects them with
  `UnsupportedFeature`.

---

## 11. Conformance checklist

A v0.4 implementation is conforming when it:

1. Writes a v0.1 file, byte for byte what v0.1 writes, when no layer table,
   no pyramid, no adaptive partition and no promoted codec is involved.
2. Writes a v0.4 file with `minor version == 4` when it writes a layer table.
3. Writes the layer table between the directory and the payloads, and points
   the header at it.
4. Sets `FEATURE_LAYERS` iff the layer table length is non-zero, and never
   sets it together with `FEATURE_PYRAMID`.
5. Puts the layer index in bits 8..11 of each entry's `flags`, and keeps bits
   12..15 zero.
6. Orders the entries `(layer, level, y, x)`.
7. Rejects a file that breaks any rule in §5, with `CorruptFile`.
8. Decodes every layer **exactly**.
9. Composites a one-layer file to exactly that layer's pixels.
10. Composites with integer, deterministic, straight-alpha arithmetic.
11. Never composites a mask layer on its own.
12. Refuses to write a mask in a format without an 8-bit alpha channel.
13. Produces the same file, byte for byte, whatever the worker count and
    whether the file was written by the batch encoder or the streaming writer.
14. Refuses to edit a file that carries a layer table, rather than dropping
    the layers.

The test suite pins these rules in
`crates/vpng-testkit/tests/v4_formats.rs`,
`crates/vpng-encoder/tests/layer_identity.rs`,
`crates/vpng-decoder/src/composite.rs`,
`crates/vpng-editing/tests/v4_layers.rs` and
`crates/vpng-cli/tests/v4_features.rs`.
