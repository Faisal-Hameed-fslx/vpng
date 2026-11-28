# VPNG container specification v0.5

**Status:** draft, implemented by `vpng-container` v0.5.0 (the workspace is
still versioned 0.1.0; the *format* minor version is what this document
defines)
**Extension:** `.vpng`
**Byte order:** little-endian throughout
**Predecessors:** `docs/VPNG_SPEC_0_4.md`, `docs/VPNG_SPEC_0_3.md`,
`docs/VPNG_SPEC_0_2.md`, `docs/VPNG_SPEC_0_1.md`

> **What v0.5 changes, in one line.** The header **grows to 72 bytes** (through
> the `header_size` field that exists for that purpose), the last eight of the
> new bytes point at a **GPU payload table**, and the table gives every tile
> an optional **lossy GPU sibling** of its lossless payload — a separate
> payload kind that never replaces the lossless one and is never read by an
> exact operation.

This document is normative for the v0.5 implementation. Where a design
question was open, the resolved answer is recorded in `docs/decisions/`. If
code and this document disagree, that is a bug in one of them; file it, do not
paper over it.

---

## 0. Relationship to v0.4, v0.3, v0.2 and v0.1

v0.5 is an **extension**, not a redesign:

| | v0.1 | v0.2 | v0.3 | v0.4 | v0.5 |
|---|---|---|---|---|---|
| main header | 64 bytes | **unchanged** | **unchanged** | **unchanged** | **72 bytes** (bytes 64..72 hold the GPU table's location) |
| directory entry | 40 bytes | **unchanged** | **unchanged** | **unchanged** | **unchanged** |
| file layout | header, directory, payloads | **unchanged** | **unchanged** | header, directory, layer table, payloads | header, directory, **layer table**, **GPU table**, payloads |
| codecs | ids 0–3 | ids 0–3 | ids 0–6 | **unchanged** | **unchanged** (ids 0–6) **+ GPU block codecs 7 and 8** |
| minor version | 1 | 2 | 3 | 4 | 5 |
| feature flags | must be 0 | bit 0 = pyramid | **unchanged** | bit 1 = layers | **bit 2 = GPU payloads** |
| entry `flags` | must be 0 | low byte = level | **unchanged** | low byte = level, next four bits = layer | **unchanged** |
| payloads per tile | one (lossless) | one | one | one | **one lossless, plus an optional lossy GPU payload** |

Everything v0.4 defines remains in force in v0.5. Everything v0.3 defines
remains in force in v0.4 and v0.5. Everything v0.2 defines remains in force
in v0.3, v0.4 and v0.5.

### 0.1 Compatibility rules

| Situation | Required behaviour |
|---|---|
| A v0.5 reader reads a v0.1, v0.2, v0.3 or v0.4 file | Accept it, exactly as that version defines |
| A v0.5 reader reads a v0.5 file | Accept it; the lossless payloads reconstruct exactly, and the GPU payloads are available through the explicitly named lossy API |
| A v0.1–v0.4 reader reads a v0.5 file | **Refuse it with `UnsupportedFeature`**: the minor version is newer than the reader knows |
| A v0.4 reader reads a v0.5 file's header | **Refuse it with `UnsupportedFeature`**, on the minor version, before the header size is examined |
| A file with minor version below 5 declares a GPU table | Reject it with `CorruptFile`: that version never defined a GPU table |
| A file with minor version 5 has no GPU table | Reject it with `CorruptFile`: a v0.5 file is a file with a GPU table |
| A directory entry names codec id 7 or 8 | Reject it with `UnsupportedFeature`: a GPU block codec is not a tile codec |
| A file names an id in `0x80..=0xFF` | Reject it with `UnsupportedFeature`: those ids are research-only and may never reach a file |

### 0.2 The minor version states what the file **contains**

The minor version is a record of what the file *is*, not of what the encoder
was permitted to do. The rule is extended, not replaced:

1. if the file carries a **GPU payload table**, the file is a **v0.5** file and
   its minor version is 5;
2. otherwise, if the file carries a **layer table**, the file is a **v0.4**
   file and its minor version is 4;
3. otherwise, if any directory entry names codec id 4, 5 or 6, the file is a
   **v0.3** file and its minor version is 3;
4. otherwise, if the file uses a v0.2 feature (a preview pyramid or an
   adaptive partition), the file is a **v0.2** file and its minor version is 2;
5. otherwise the file is a **v0.1** file and its minor version is 1.

The default encoder path writes no GPU table, permits only ids 0–3 and
requests neither a pyramid nor an adaptive partition, so it keeps writing the
exact v0.1 bytes the golden fixtures pin. **v0.5 moves no published byte.**

---

## 1. File layout

```text
+----------------------------+  offset 0
| main header (72 bytes)     |  only for a v0.5 file; 64 for every other
+----------------------------+  offset 64 or 72
| tile directory             |  N * 40 bytes + 4-byte CRC32C
+----------------------------+  end of directory
| layer table                |  only when the header says so
+----------------------------+  end of layer table
| GPU table                  |  only when the header says so
+----------------------------+  end of GPU table
| tile payloads and GPU      |
| payloads, interleaved      |
+----------------------------+  end of file
```

A file with no GPU table has exactly the v0.1 layout: the payloads start
where the directory ends.

---

## 2. Main header

### 2.1 The 64-byte header (v0.1 through v0.4)

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
| 48     | 4    | feature flags     | §2.3 |
| 52     | 4    | header checksum   | CRC32C over `0..52` and `56..64` (ADR-0001) |
| 56     | 4    | layer table offset| byte offset from file start; zero when there is no table |
| 60     | 4    | layer table length| the table's length in bytes; zero when there is no table |

### 2.2 The 72-byte header (v0.5)

A v0.5 file's header is **72 bytes**. Every field above keeps its offset and
its meaning, and the `header size` field says 72.

| Offset | Size | Field             | Notes |
|-------:|-----:|-------------------|-------|
| 0..64  | 64   | as §2.1           | unchanged, including the layer table's location |
| 64     | 4    | **GPU table offset** | byte offset from file start; zero when there is no table |
| 68     | 4    | **GPU table length** | the table's length in bytes; zero when there is no table |

The header checksum rule becomes: **CRC32C over bytes `0..52` and
`56..header_size`**. For a 64-byte header that is exactly the rule v0.1–v0.4
use, so no existing file's checksum changes and no existing byte moves. For a
72-byte header it covers the layer table's location and the GPU table's
location as well, so neither can be edited without rewriting the header.

A reader applies these rules:

1. the `header size` field must be 64 or 72;
2. a header size of 64 means the minor version is at most 4;
3. a header size of 72 means the minor version is at least 5;
4. a minor version below 5 must have the GPU table's offset and length zero,
   and must not set `FEATURE_GPU`;
5. a minor version of 5 must have a non-zero GPU table length, must set
   `FEATURE_GPU`, and must have the GPU table's offset at or beyond the end
   of the header;
6. bytes 56..`header_size` must be zero in a file whose minor version is
   below 4 — the rule v0.1–v0.3 already state, applied to the bytes that
   exist.

### 2.3 Feature flags

| bit | mask | meaning |
|---:|------|---------|
| 0 | `0x0000_0001` | `FEATURE_PYRAMID` — the file has derived preview levels (v0.2) |
| 1 | `0x0000_0002` | `FEATURE_LAYERS` — the file has a layer table (v0.4) |
| 2 | `0x0000_0004` | `FEATURE_GPU` — the file has a GPU payload table (v0.5) |
| 3..31 | | reserved; must be zero |

`FEATURE_PYRAMID` and `FEATURE_LAYERS` remain **mutually exclusive**, for the
reason v0.4 gives. `FEATURE_GPU` is **orthogonal** to both: a file may have a
GPU payload table, a layer table, a preview pyramid and promoted codecs at the
same time, because the GPU payload is a sibling of the lossless payload and
not a replacement for it.

`FEATURE_GPU` is set **iff** the GPU table length is non-zero, and the GPU
table offset is zero **iff** the length is zero. A reader checks both halves.

---

## 3. Tile directory

The entry layout is v0.1's, byte for byte, and the `flags` field keeps v0.4's
meaning exactly.

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

### 3.1 The `flags` field in v0.5

| bits | mask | meaning |
|------|------|---------|
| 0..7 | `0x00FF` | the tile's **level index** (v0.2) |
| 8..11 | `0x0F00` | the tile's **layer index** (v0.4) |
| 12..15 | `0xF000` | reserved; must be zero |

**Unchanged from v0.4.** The GPU payload needs no flag here, because the GPU
table has one record per tile and a record says for itself whether a payload
is there.

### 3.2 Canonical order

The entries of a v0.5 file are in canonical **`(layer, level, y, x)`** order,
exactly as in v0.4, and the GPU table's records are in the **same** order, one
per entry. Record *i* describes entry *i*.

---

## 4. The GPU payload table

### 4.1 Where it sits and why

The table sits with the other tables: after the directory, and after the layer
table when there is one. The lossless payloads therefore start after it, and a
reader computes that from the header rather than by guessing.

The table is **not** inside the directory, because the directory entry's 40
bytes are all in use and the entry layout is the one thing this format has
never moved.

### 4.2 The table header

```text
offset  size  field
     0     4  record count (must equal the file's tile count)
     4     4  descriptor array length
     8     4  CRC32C over the descriptor array
    12     4  reserved (must be zero)
    16     8  total GPU payload bytes (the sum of every record's length)
    24   ...  descriptor array
```

The table's header is **24 bytes**, and the descriptor array starts
immediately after it. The descriptor array length must be exactly the number
of bytes between the end of this header and the end of the table: no more, no
fewer. The checksum covers the descriptor array only.

A table must hold at least one record, and exactly as many as the file has
tiles.

### 4.3 One record

```text
offset  size  field
     0     8  GPU payload offset (absolute file offset)
     8     4  GPU payload length
    12     1  colour codec id (0: none, 7: BC1)
    13     1  alpha codec id (0: none, 8: BC4)
    14     2  reserved (must be zero)
    16     4  CRC32C of the GPU payload
```

A record with both codec ids zero means **this tile has no GPU payload**; its
offset, length and checksum are then zero as well. A record with either codec
id non-zero points at a payload.

The payload's bytes are laid out as:

```text
colour blocks   (when the colour codec id is 7: BC1)
alpha blocks    (when the alpha codec id is 8: BC4)
```

in that order, with no padding between them.

### 4.4 The GPU block codecs

| id | name | channels | bytes per 4×4 block | what it holds |
|---:|------|---------:|--------------------:|---------------|
| 7 | `bc1` | 3 | 8 | the colour channels, as R, G, B |
| 8 | `bc4` | 1 | 8 | one channel: alpha, or a mask's coverage |

A tile's payload length is therefore

```text
blocks = ceil(width / 4) * ceil(height / 4)
length = blocks * 8 * (colour_id != 0) + blocks * 8 * (alpha_id != 0)
```

and a reader checks the record's length against that number.

**Neither id may name a directory entry's `codec_id`.** A directory entry that
names id 7 or 8 is refused with `UnsupportedFeature`. The GPU block codecs are
not lossless, and a tile's lossless payload is always lossless.

---

## 5. What a GPU payload means

### 5.1 It is lossy, and it says so

A GPU payload is a **BC1** or **BC4** block payload. Decoding it gives pixels
that are *close to* the original and **not equal** to it. That is the purpose:
the blocks are what a GPU samples as a texture without decompressing anything.

So:

* the GPU payload is **never** used to reconstruct the image;
* `verify`, `check`, `decode`, `decode-region`, `decode-layer` and
  `composite` do not read it;
* only the explicitly named GPU API reads it, and what it returns is named
  after what it is.

A file with a GPU payload still reconstructs **exactly**, through its lossless
payload, as if the GPU payload were not there.

### 5.2 The channel mapping

| file format | colour blocks | alpha blocks |
|---|---|---|
| `rgb8` | BC1 | none |
| `rgba8` | BC1 | BC4 |
| `gray8` | BC1 (the grey value in all three channels) | none |
| `graya8` | BC1 (the grey value in all three channels) | BC4 |
| a **mask layer's** tile | none | BC4 (the coverage) |

A grey picture is replicated into RGB, because a texture is RGB and the
replication costs at most four levels on a 5-bit channel. The cost is
measured, not assumed: `docs/P13_GPU_REPORT.md` reports the error.

A mask layer's colour channels carry no meaning and are not encoded; only the
coverage is.

### 5.3 16-bit formats have no GPU payload

BC1 and BC4 are 8-bit block formats. A 16-bit file has no GPU payload, and
the encoder **refuses** to write one rather than quietly dropping the
precision or quietly writing a lossy 8-bit copy of a 16-bit picture.

### 5.4 The GPU payload's pixels

The explicitly named GPU decode returns **RGBA8** pixels:

* the colour blocks give R, G and B;
* the alpha blocks give A, and a tile with no alpha blocks is fully opaque;
* a mask tile is black with the coverage in its alpha.

The result is **not** the file's own format: a GPU payload is always 8-bit,
because that is what the block formats hold. A consumer that needs the exact
pixels uses the lossless payload.

---

## 6. The rules a reader enforces

A reader rejects a v0.5 file with `CorruptFile` when any of these is false:

1. the `header size` field is 64 or 72, and agrees with the minor version
   (§2.2);
2. the GPU table's record count equals the file's tile count;
3. the descriptor array length matches the bytes present, exactly;
4. the descriptor array's CRC32C matches the stored checksum;
5. the table's reserved words are zero;
6. every record's reserved bytes are zero;
7. every record's payload offset and length lie inside the file;
8. every record's payload offset is at or beyond the end of the container's
   tables;
9. every record's payload length equals the length its codec ids imply for
   that tile's geometry (§4.4);
10. every record's payload checksum matches the payload's bytes;
11. the table's total GPU payload bytes equal the sum of the records' lengths;
12. a record's codec ids are either 0 or a GPU block codec this build
    implements;
13. a record with both codec ids zero has a zero offset, a zero length and a
    zero checksum;
14. the file does not set both `FEATURE_PYRAMID` and `FEATURE_LAYERS`;
15. a file with minor version below 5 has no GPU table.

A reader rejects a v0.5 file with `UnsupportedFeature` when a directory entry
names a GPU block codec, or a GPU block codec it does not implement.

---

## 7. Encoding a file with GPU payloads

### 7.1 The rules an encoder enforces

1. The file's format is **8-bit** (`rgb8`, `rgba8`, `gray8` or `graya8`).
2. Every tile gets a GPU payload, except a **mask layer's** tile, which gets
   a BC4 coverage payload.
3. The GPU payload is built from the tile's **original** pixels, before any
   prediction, so the GPU payload does not depend on the lossless codec that
   was chosen.
4. The worker count changes the speed and not the bytes.

### 7.2 The payloads are interleaved

Each tile's lossless payload is written, and then its GPU payload, and then
the next tile's lossless payload. The GPU payload's offset is therefore
recorded per tile, and a consumer fetches exactly the bytes it wants.

### 7.3 Determinism

The same pixels and the same options produce the same file, byte for byte,
whatever the worker count and whether the file was written by the batch
encoder or the streaming writer.

---

## 8. Editing a v0.5 file

The editing engine **refuses** a file that carries a GPU payload table, for
the same reason it refuses a layered file: the commit path rewrites the
directory and the payloads, and it does not yet know how to carry the GPU
table and its per-tile records through that rewrite. The refusal is an
`UnsupportedFeature` whose message says "GPU table".

A file with no GPU table is edited exactly as v0.4 defines.

---

## 9. Migration policy

* **Readers** must accept a v0.1, v0.2, v0.3, v0.4 or v0.5 file. A reader
  that does not implement the GPU table rejects a v0.5 file with
  `UnsupportedFeature` — the ordinary "newer than I know" refusal — and must
  not attempt a best-effort decode.
* **Writers** default to the flat path, so the bytes a published encoder
  produces do not change. The GPU path is opt-in.
* **A v0.5 file without a GPU payload table is not a v0.5 file**: the minor
  version states what the file contains, so a file with no GPU table is
  written as the version its other contents call for.
* **Research ids** (`0x80..=0xFF`) may never reach a file.

---

## 10. Conformance checklist

A v0.5 implementation is conforming when it:

1. Writes a v0.1 file, byte for byte what v0.1 writes, when no GPU table, no
   layer table, no pyramid, no adaptive partition and no promoted codec is
   involved.
2. Writes a v0.5 file with `minor version == 5` and a **72-byte** header when
   it writes a GPU payload table.
3. Writes the GPU table after the directory and the layer table, and points
   the header at it through bytes 64..72.
4. Sets `FEATURE_GPU` iff the GPU table length is non-zero.
5. Puts one 20-byte record per tile in the GPU table, in the directory's
   canonical order.
6. Rejects a file that breaks any rule in §6.
7. Never lets a directory entry name a GPU block codec.
8. Decodes every tile **exactly**, through the lossless payload, whether or
   not the file has GPU payloads.
9. Reads a GPU payload only through the explicitly named lossy API.
10. Reports the size of the lossless payload and of the GPU payload
    separately.
11. Refuses to write a GPU payload for a 16-bit file.
12. Produces the same file, byte for byte, whatever the worker count and
    whether the file was written by the batch encoder or the streaming writer.
13. Refuses to edit a file that carries a GPU payload table, rather than
    dropping the payloads.
14. Gives a **mask layer's** tile a BC4 coverage payload and **no** colour
    payload, whatever the mask layer's colour channels hold.

The test suite pins these rules in
`crates/vpng-testkit/tests/v5_formats.rs`,
`crates/vpng-encoder/tests/gpu_identity.rs`, `crates/vpng-gpu`,
`crates/vpng-editing/tests/v5_gpu.rs` and
`crates/vpng-cli/tests/v5_features.rs`.
