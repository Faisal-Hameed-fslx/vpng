# VPNG container specification v0.3

**Status:** draft, implemented by `vpng-container` v0.3.0 (the workspace is
still versioned 0.1.0; the *format* minor version is what this document
defines)
**Extension:** `.vpng`
**Byte order:** little-endian throughout
**Predecessors:** `docs/VPNG_SPEC_0_2.md`, `docs/VPNG_SPEC_0_1.md`

> **What v0.3 changes, in one line.** The 64-byte header and the 40-byte
> directory entry keep their exact layouts; v0.3 changes one *rule* — a
> directory entry may name codec id 4 (LZ77 dictionary), 5 (predictor over a
> dictionary) or 6 (context-adaptive entropy), and a file that names one of
> them must say so in its minor version.

This document is normative for the v0.3 implementation. Where a design
question was open, the resolved answer is recorded in `docs/decisions/`. If
code and this document disagree, that is a bug in one of them; file it, do not
paper over it.

---

## 0. Relationship to v0.2 and v0.1

v0.3 is an **extension**, not a redesign:

| | v0.1 | v0.2 | v0.3 |
|---|---|---|---|
| main header | 64 bytes, fixed layout | **unchanged** | **unchanged** |
| directory entry | 40 bytes, fixed layout | **unchanged** | **unchanged** |
| file layout | header, directory, payloads | **unchanged** | **unchanged** |
| codecs | ids 0–3 | ids 0–3 | ids **0–6** |
| minor version | 1 | 2 | 3 |
| feature flags | must be 0 | bit 0 = pyramid | **unchanged** |
| entry `flags` | must be 0 | low byte = level index | **unchanged** |
| tiles in a level | one uniform grid | an exact partition; sizes may vary | **unchanged** |
| levels | exactly one | one or more | **unchanged** |

Everything v0.2 defines — the preview pyramid, the adaptive partition, the
level rules, the `flags` field — remains in force in v0.3. A v0.3 file may
carry a pyramid **and** name a promoted codec.

### 0.1 Compatibility rules

| Situation | Required behaviour |
|---|---|
| A v0.3 reader reads a v0.1 file | Accept it, exactly as v0.1 defines |
| A v0.3 reader reads a v0.2 file | Accept it, exactly as v0.2 defines |
| A v0.2 reader reads a v0.3 file that names only ids 0–3 | **It cannot know the file is a v0.3 file**: the minor version is 3, so it rejects it with `UnsupportedFeature` — the minor version is newer than the reader knows |
| A v0.1 or v0.2 reader reads a v0.3 file that names id 4, 5 or 6 | Reject it with `UnsupportedFeature` |
| A v0.1 or v0.2 file names id 4, 5 or 6 | Reject it with `UnsupportedFeature`: those versions never defined those codecs |
| A v0.3 file names id 4, 5 or 6 | Accept it, and decode with that codec |
| A v0.3 file names only ids 0–3 | Accept it; the version byte is the only difference from a v0.2 file |
| A file names an id in `0x80..=0xFF` | Reject it with `UnsupportedFeature`: those ids are research-only and may never reach a file |

### 0.2 The minor version states what the file **contains**

The minor version is not a record of what the encoder was *allowed* to do. It
is a record of what the file *is*:

1. if any directory entry names codec id 4, 5 or 6, the file is a **v0.3**
   file and its minor version is 3;
2. otherwise, if the file uses a v0.2 feature (a preview pyramid or an
   adaptive partition), the file is a **v0.2** file and its minor version is 2;
3. otherwise the file is a **v0.1** file and its minor version is 1.

This rule has two consequences that matter:

* A writer that *permits* the extended codec set but ends up choosing only the
  original four codecs writes a **v0.1** file. The permission is not a claim
  about the bytes.
* A writer that names a promoted codec **must** write minor version 3. A file
  that names id 6 and says minor version 1 is malformed, and every reader
  rejects it.

The default encoder path permits only ids 0–3, so it keeps writing the exact
v0.1 bytes the golden fixtures pin. The promotion does not move a single
published byte.

---

## 1. File layout

Unchanged from v0.2, and therefore from v0.1:

```text
+----------------------------+  offset 0
| main header (64 bytes)     |
+----------------------------+  offset 64
| tile directory             |  N * 40 bytes + 4-byte CRC32C
+----------------------------+  end of directory
| tile payloads, back to back|
+----------------------------+  end of file
```

---

## 2. Main header (64 bytes)

The layout is v0.1's, byte for byte. The only field whose **value** differs
from v0.1 is the minor version, which may now be 1, 2 or 3.

| Offset | Size | Field             | Notes |
|-------:|-----:|-------------------|-------|
| 0      | 8    | magic             | `56 50 4E 47 0D 0A 1A 0A` |
| 8      | 2    | major version     | must be 0 |
| 10     | 2    | minor version     | 1, 2 or 3 |
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
| 48     | 4    | feature flags     | v0.2 §2.1 |
| 52     | 4    | header checksum   | CRC32C over `0..52` and `56..64` (ADR-0001) |
| 56     | 8    | reserved          | must be zero |

The header checksum rule is unchanged, so the minor version and the feature
flags are covered by it: a file cannot be edited from v0.1 into v0.3 without
rewriting the header.

---

## 3. Tile directory

Unchanged from v0.2: the same 40-byte entries, the same `flags` semantics, the
same `(level, y, x)` canonical order, the same level and partition rules. Only
the **codec id** field (offset 12) has a wider range.

---

## 4. Codec identifiers

v0.1 §5 defines ids 0–3 and their payload layouts; those definitions are
unchanged and are not repeated here. v0.3 adds three codecs to the stable set:

| id | codec                    | v0.1 | v0.2 | v0.3 |
|---:|--------------------------|------|------|------|
| 0  | Raw                      | implemented | implemented | implemented |
| 1  | Palette                  | implemented | implemented | implemented |
| 2  | Run-length               | implemented | implemented | implemented |
| 3  | Spatial predictor + entropy | implemented | implemented | implemented |
| 4  | LZ77 dictionary          | reserved | reserved | **implemented** |
| 5  | Predictor + dictionary   | reserved | reserved | **implemented** |
| 6  | Context-adaptive entropy | reserved | reserved | **implemented** |
| `0x80..=0xFF` | research codecs | reserved | reserved | **reserved** |

The research ids remain research-only. A file may not carry one: a decoder
rejects it with `UnsupportedFeature`, and the production encoder cannot
produce one.

The three promoted codecs are the ones ADR-0004 reserved under exactly these
ids, so the reservation and the implementation agree.

### 4.1 The rule that binds a codec id to a version

A directory entry may name codec id 4, 5 or 6 **only if** the file's minor
version is 3 or newer. This is checked by the reader, per entry, after the
header has been parsed and before any payload is decoded:

* a v0.1 or v0.2 file that names id 4, 5 or 6 is rejected with
  `UnsupportedFeature` — the file is claiming a codec that version never
  defined, and a best-effort decode is not permitted;
* a v0.3 file that names id 4, 5 or 6 is accepted, and the entry's codec
  decides how the payload is decoded.

The check is per entry, so a file that mixes ids 0–3 and ids 4–6 is judged on
the entries that name a promoted codec. There is no "mixed mode": one file,
one version.

### 4.2 The selection rule

The per-tile selection is unchanged in shape: the encoder offers every tile to
the codecs its **codec set** permits, and the smallest payload wins. A tie
goes to the **lowest codec id**. Raw is always a candidate, so a valid
lossless encoding always exists.

Two codec sets are defined:

| set | ids | meaning |
|---|---|---|
| `stable` | 0, 1, 2, 3 | the four original codecs; the default |
| `extended` | 0, 1, 2, 3, 4, 5, 6 | the four original codecs plus the three promoted in v0.3 |

The default set is `stable`, so the default path's bytes do not move. The
`extended` set is opt-in. The selection is per tile and deterministic: the
same pixels, the same tile geometry and the same set always produce the same
file, byte for byte, whatever order the tiles were encoded in.

---

## 5. Codec id 4 — LZ77 dictionary

The payload is the tile's own bytes, compressed with a **tile-local** LZ77
dictionary. A match may only refer back to a position that is already inside
the current tile, so a tile decodes without its neighbours and no state
crosses a tile boundary.

### 5.1 Payload header

```text
offset 0   u8   version (1)
offset 1   u8   strategy
offset 2   u8   min_match
offset 3   u16  max_match              (little-endian)
offset 5   u8   window_bits
offset 6   u8   hash_bits
offset 7   u8   chain_length
offset 8   u8   reserved (must be 0)
offset 9   ..   output length, unsigned LEB128
offset ..  ..   token stream
```

Every parameter is serialized, so a decoder never has to assume a default.

| field | meaning | range |
|---|---|---|
| `strategy` | which match search to run | 0..=3 |
| `min_match` | shortest match that is worth a match token | 3..=255 |
| `max_match` | longest match the encoder will emit | 3..=799 |
| `window_bits` | `log2` of the window size in bytes | 10..=18 |
| `hash_bits` | `log2` of the hash table size | 8..=16 |
| `chain_length` | how many chain candidates to examine | 1..=64 |

A parameter outside its range makes the payload corrupt.

### 5.2 The token stream

A sequence of *groups*. Each group starts with a flag byte whose bit `i` (LSB
first) says whether item `i` of the group is a match (`1`) or a literal (`0`);
a group carries up to eight items.

* a literal item is one byte: the literal value;
* a match item is a length code byte, then a distance code byte, then zero, one
  or two extra bytes for the length and the distance.

Length codes:

| code | length | extra bytes |
|---|---|---|
| 0..=28 | `code + 3` (3..=31) | 0 |
| 29 | `32 + b0` (32..=287) | 1 |
| 30 | `288 + b0 + 256*b1` (288..=799) | 2 |
| 31 | reserved | — |

Distance codes use the DEFLATE-style base/extra-bit table, giving a maximum
representable distance of 24576 bytes.

### 5.3 End of stream, and invalid references

There is no terminator token: the decoder stops as soon as it has produced
exactly the declared output length. Unused slots in the final flag byte are
never read, and the encoder never writes bytes for them.

A match whose distance is zero, larger than the bytes already produced, or
larger than the window is corrupt. A match whose length is outside
`[min_match, max_match]`, or that would run past the declared output length,
is corrupt. Every one of those is a typed `CorruptFile` error, not a panic and
not a best-effort decode.

### 5.4 Matching is byte-level

Matches are found over the raw byte stream of the tile. A match may start in
the middle of a pixel, and a 16-bit sample may be split across a match
boundary. The decoder copies bytes verbatim, so the split is harmless — the
reconstruction is exact by construction, not by alignment.

---

## 6. Codec id 5 — predictor over a dictionary

The pipeline is **predictor → LZ77**: the tile's residual bytes are folded to
unsigned bytes and then compressed with the id 4 dictionary.

### 6.1 The residual fold

The predictor stage is the stable predictor's (id 3) residual coder, so its
residual layout and its plan are v0.1 §5.4's. The hybrid folds each signed
residual into a fixed-width unsigned byte:

```text
byte = residual + 2^(8B-1)
```

where `B` is the sample width. The fold is exact and reversible: the residual
range `[-2^(8B-1), 2^(8B-1) - 1]` maps onto `[0, 2^(8B) - 1]`. A 16-bit
sample stays a 16-bit value; the hybrid never splits a sample across two
residuals.

### 6.2 Payload layout

```text
offset 0   u8   hybrid version (1)
offset 1   u8   kind code (0 = predictor then LZ77)
offset 2   u8   predictor version (1)
offset 3   ..   tile width in **pixels**, unsigned LEB128
offset ..  ..   one predictor-mode byte per tile row
offset ..  ..   the LZ77 stage's own header (v0.3 §5.1)
offset ..  ..   the LZ77 token stream, verbatim
```

The LZ77 stage's output length is the length of the folded residual bytes, and
the LZ77 stage's declared output length is therefore that length — not the
tile's pixel length. The decoder folds the residual bytes back to signed
residuals and reconstructs the tile.

Every stage is tile-local: the predictor reads only the tile, the dictionary
only holds the tile's own bytes, and no state crosses a tile boundary.

### 6.3 The other two hybrids

The predictor-then-LZ77-then-entropy and LZ77-then-entropy pipelines remain
**research-only**. They carry research ids (`0x92`, `0x93`), are not in the
implemented set, and may never appear in a file.

---

## 7. Codec id 6 — context-adaptive entropy

The stable predictor's residual coder, with the **probability contexts**
conditioned on the picture's local gradient instead of being shared by every
residual in the tile.

### 7.1 The model

The stable predictor codes a residual with a sign bit and one decision per
magnitude plane, priced against a model that knows nothing about the image.
Codec 6 prices the same decisions against a **gradient class** of the
reconstructed neighbourhood: a small integer class derived from the
differences between the residual and its already-reconstructed left, upper and
upper-left neighbours.

The probability table is split into a fixed number of **blocks**. Block `k`
holds the sign context and one context per magnitude plane, so a block is
exactly `context_count(sample_bits)` probabilities. The model picks a block per
residual; the block index is an FNV-1a hash of the feature values, truncated
to the table size.

The table has 256 blocks, so the model state is 17 KiB for 8-bit samples and
34 KiB for 16-bit samples — bounded, and independent of the tile size. The
table is built fresh for every tile; no state crosses a tile boundary.

### 7.2 Payload layout

```text
offset 0   u8   version (1)
offset 1   u8   model id (2 = the gradient-conditioned model)
offset 2   u8   context table shift (log2 of the number of blocks)
offset 3   ..   tile width in **pixels**, unsigned LEB128
offset ..  ..   one predictor-mode byte per tile row
offset ..  ..   residual count, unsigned LEB128
offset ..  ..   entropy-coded residual data
```

The residual bit layout, the predictor modes and the arithmetic coder are
exactly the stable predictor codec's (v0.1 §5.4). Only the choice of the
probability block differs.

### 7.3 Determinism

Every decision is integer-only: the feature values are small integer classes,
the block index is an integer hash, the probability updates are the stable
coder's own integer updates, and the gradient tests compare integer
differences against integer thresholds that scale with the sample width. There
is no floating point anywhere in the bitstream path.

The reconstructed neighbours the gradient class depends on are the values the
decoder has already reconstructed, so the encoder and the decoder see exactly
the same numbers.

---

## 8. Editing a v0.3 file

The editing rules are v0.2 §8's: an edit invalidates the derived levels, so
the committed file holds level 0 only, the `FEATURE_PYRAMID` bit is cleared,
the tiles that were not edited keep their **payload bytes** exactly, and the
report says how many preview levels were dropped.

One addition: if the editor rewrites a tile and the rewritten tile uses a
codec promoted in v0.3, the committed file's minor version is at least 3. The
committed file may therefore be a v0.3 file even when the file it was edited
from was a v0.1 file — and it says so, so a reader never has to guess.

---

## 9. Migration policy

* **Readers** must accept a v0.1, v0.2 or v0.3 file. A reader that does not
  implement the promoted codecs rejects a file that names them with
  `UnsupportedFeature`; it must not attempt a best-effort decode.
* **Writers** default to the stable set, so the bytes a published encoder
  produces do not change. The extended set is opt-in.
* **A v0.3 file that names only ids 0–3** is a valid v0.3 file, and a v0.2
  reader rejects it for the ordinary reason: the minor version is newer than
  the reader knows. It is not a special case.
* **Research ids** (`0x80..=0xFF`) may never reach a file. They exist in the
  research harness only, and the production decoder rejects them with
  `UnsupportedFeature`.

---

## 10. Conformance checklist

A v0.3 implementation is conforming when it:

1. Writes a v0.1 file when neither a pyramid nor an adaptive partition is
   requested and the selection chose only ids 0–3, byte for byte what v0.1
   writes.
2. Writes a v0.2 file with `minor version == 2` when either is requested and
   no promoted codec was chosen.
3. Writes a v0.3 file with `minor version == 3` when any tile names id 4, 5
   or 6.
4. Never writes a file whose minor version is below the highest codec id it
   names.
5. Rejects a v0.1 or v0.2 file that names id 4, 5 or 6, with
   `UnsupportedFeature`.
6. Decodes a v0.3 file that names id 4, 5 or 6, exactly.
7. Rejects any file that names an id in `0x80..=0xFF`, with
   `UnsupportedFeature`.
8. Produces the same file, byte for byte, whatever the worker count and
   whether the file was written by the batch encoder or the streaming writer.
9. Keeps the selection rule: the smallest payload wins, and a tie goes to the
   lowest codec id.
10. Keeps every rule of v0.2 about levels, partitions, `flags` and the
    pyramid.

The test suite pins these rules in
`crates/vpng-testkit/tests/v3_formats.rs` and
`crates/vpng-encoder/tests/parallel_identity.rs`.
