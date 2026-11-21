# VPNG container specification v0.1

**Status:** draft, implemented by `vpng-container` v0.1.0
**Extension:** `.vpng`
**Byte order:** little-endian throughout
**Magic:** `56 50 4E 47 0D 0A 1A 0A` (`VPNG\r\n\x1a\n`)

> **Clarification (2026-11-18).** Header byte 27 (`default codec id`) is
> resolved as **informational** — see section 2.3 and
> `docs/decisions/ADR-0003-directory-authority.md`. The byte, its offset, its
> size and the format version are unchanged; only the description was wrong.
> A conforming test for the clarified behaviour is
> `crates/vpng-testkit/tests/default_codec.rs`.

This document is normative for the v0.1 implementation. Where a design question
was open, the resolved answer is recorded in `docs/decisions/`. If code and this
document disagree, that is a bug in one of them; file it, do not paper over it.

> **v0.2 (2026-11-21).** A later minor version, `docs/VPNG_SPEC_0_2.md`, adds a
> preview pyramid and adaptive (variable-size) tiles. It keeps this document's
> 64-byte header and 40-byte directory entry exactly as they are, and changes
> two rules: the entry's `flags` field carries the level index (it must be
> zero here), and a level's tiles may have different sizes (they must form one
> uniform grid here). A v0.1 file remains valid and readable; a v0.1 reader
> rejects a v0.2 file because its minor version is newer.

---

## 1. File layout

A v0.1 file is exactly three regions, in this order:

```text
+----------------------------+  offset 0
| main header (64 bytes)     |
+----------------------------+  offset 64
| tile directory             |  N * 40 bytes + 4-byte CRC32C
+----------------------------+  end of directory
| tile payloads, back to back|
+----------------------------+  end of file
```

Rules:

1. The header begins at offset 0 and is exactly 64 bytes.
2. `directory_offset` must be `>= 64`. v0.1 writers always use exactly 64.
3. The directory occupies `tile_count * 40 + 4` bytes starting at
   `directory_offset`.
4. Every tile payload starts at or after the end of the directory. Payloads may
   appear in any order and need not be contiguous, but no two payload ranges may
   overlap.
5. There is no implicit padding anywhere. No alignment is required.
6. `declared_file_size` must equal the actual byte length of the file. A decoder
   rejects a mismatch. Trailing bytes are therefore also rejected.
7. The file must be a single directory generation. There is no second authority.

### 1.1 Reserved for later versions

v0.1 defines no chunks. Chunk-oriented metadata (`ICCP`, `EXIF`, `XMP`, `THMB`,
`HASH`, `PYRM`, `EDIT`) is reserved for a later minor version, at which point it
will be a region following the directory and before the payloads, described by
the header's feature flags. Readers of v0.1 files never encounter them.

---

## 2. Main header (64 bytes)

| Offset | Size | Field             | Notes |
|-------:|-----:|-------------------|-------|
| 0      | 8    | magic             | exact bytes above |
| 8      | 2    | major version     | must be 0 |
| 10     | 2    | minor version     | must be `<= 1` |
| 12     | 4    | header size       | must be 64 |
| 16     | 4    | width             | pixels, non-zero |
| 20     | 4    | height            | pixels, non-zero |
| 24     | 1    | channel count     | must equal color model's channel count |
| 25     | 1    | bits per channel  | 8 or 16 |
| 26     | 1    | color model id    | see section 3 |
| 27     | 1    | default codec id  | informational hint; see 2.3 |
| 28     | 4    | tile count        | number of directory entries |
| 32     | 8    | directory offset  | byte offset from file start |
| 40     | 8    | declared file size| exact total file length |
| 48     | 4    | feature flags     | 0 in v0.1; unknown bits are rejected |
| 52     | 4    | header checksum   | CRC32C, see below |
| 56     | 8    | reserved          | must be zero in v0.1 |

### 2.1 Header checksum (ADR-0001)

`header checksum` is CRC32C (Castagnoli) over the concatenation of:

* bytes `0..52` (magic through feature flags), then
* bytes `56..64` (the reserved field).

The 4 checksum bytes at `52..56` are excluded. This makes the checksum
independent of its own value and covers every other byte of the header.

### 2.2 Validation order

A parser must check, in order: total length, magic, major version, minor
version, header size, color model and bits, channel-count agreement, header
checksum, reserved-is-zero, and feature flags. The first failure is reported.

### 2.3 The default codec id is informational — ADR-0003

Header byte 27 (`default codec id`) is **informational**. It is a hint about
the file as a whole. It is **not** the authority for how any tile is decoded.

* A reader **must not** use it to decode a tile, and **must not** reject a
  file because of its value. The byte is not validated: it may name a codec
  the file does not use, a reserved id, or an id that does not exist at all.
* The **`codec id` in the tile's own directory entry** is the authority for
  that tile (section 4.1). It is the only field that decides how a payload is
  decoded.
* A writer records the codec it used for the whole file when one codec
  applies to every tile, and records **Raw (0)** when the tiles were selected
  individually, because there is then no single file codec to name. Such a
  file's header byte therefore does not describe most of its own tiles. That
  is expected, and it is not an error.

This closes the question the first draft of this section left open: the
earlier wording, "codec used for all tiles in v0.1", was wrong, because v0.1
writers select a codec **per tile** (section 5). The correction changes only
the description. The byte keeps its offset, its size and its meaning as a
hint; the format version stays 0.1; and no conforming file changes meaning,
because the directory has always been the authority (ADR-0003, accepted
2026-11-12).

A conforming implementation behaves as follows:

| Situation | Required behaviour |
|---|---|
| The header names a codec the file does not use | Decode from the directory; the pixels are unaffected |
| The header names a reserved id (4, 5, 6) | Accept the file; the byte is ignored |
| The header names an unknown id (for example 0xFE) | Accept the file; the byte is ignored |
| A **directory entry** names a reserved or unknown id | Reject the file with `UnsupportedFeature` (section 5) |

---

## 3. Pixel formats

| Color model | id | Channels | Channel order |
|-------------|----|----------|---------------|
| Gray        | 0  | 1        | Y |
| GrayAlpha   | 1  | 2        | Y, A |
| RGB         | 2  | 3        | R, G, B |
| RGBA        | 3  | 4        | R, G, B, A |
| Indexed     | 4  | 1        | reserved, not implemented in v0.1 |

Bits per channel is 8 or 16.

### 3.1 Canonical pixel buffer

The canonical representation, over which the whole-image digest is defined and
which every decoder must reproduce, obeys:

* Row-major order: rows top to bottom, pixels left to right within a row.
* Channels interleaved in the order of the table above.
* Samples of a 16-bit channel are **little-endian**.
* Rows are tightly packed: `row_bytes = width * channels * bytes_per_channel`.
  Row padding is forbidden.
* Alpha is a plain channel. RGB values beneath a fully transparent pixel are
  preserved verbatim and are part of the digest.
* There is no premultiplication and no color management in the lossless path.

Indexed images are rejected in v0.1 with `UnsupportedFeature`.

---

## 4. Tile directory

The directory is a flat array of `tile_count` fixed 40-byte entries, in
canonical order, followed by a 4-byte CRC32C over the entry bytes.

### 4.1 Entry layout (40 bytes) — ADR-0002

| Offset | Size | Field               | Notes |
|-------:|-----:|---------------------|-------|
| 0      | 4    | tile x              | pixel origin, not a tile index |
| 4      | 4    | tile y              | pixel origin |
| 8      | 2    | tile width          | pixels, non-zero |
| 10     | 2    | tile height         | pixels, non-zero |
| 12     | 1    | codec id            | section 5; **authoritative** for this tile |
| 13     | 1    | predictor id        | 0 when the codec needs no predictor |
| 14     | 2    | flags               | 0 in v0.1 |
| 16     | 8    | payload offset      | byte offset from file start |
| 24     | 8    | compressed length   | payload bytes on disk |
| 32     | 4    | uncompressed length | must equal `width * height * bytes_per_pixel` |
| 36     | 4    | payload checksum    | CRC32C over the payload bytes |

### 4.2 Directory checksum

The trailer is CRC32C over the `tile_count * 40` entry bytes.

### 4.3 Directory invariants

A conforming directory satisfies all of:

1. Entries are in canonical order: ascending `tile y`, then ascending `tile x`.
2. Each entry's rectangle lies entirely within the image.
3. The rectangles partition the image exactly once: no overlap, no gap.
4. All tiles share one nominal tile size, except that tiles in the last column
   may be narrower and tiles in the last row may be shorter.
5. Every payload range `[payload_offset, payload_offset + compressed_length)`
   lies inside the file and does not overlap any other payload range.
6. Every payload begins at or after the end of the directory.
7. `uncompressed_length` equals the geometry-derived size.

### 4.4 Deriving the tile grid — ADR-0005

The header does not store a nominal tile size, and the reserved field must stay
zero, so the grid is derived from the directory:

* Collect the distinct `tile x` origins in ascending order. If there is more
  than one, the nominal tile width is the difference between the first two. If
  there is exactly one column, the nominal tile width is the image width.
* The same rule gives the nominal tile height from distinct `tile y` origins.
* The origin set must begin at 0.
* The resulting grid's tile count must equal `tile_count`, and the directory
  entries must match the grid's geometry exactly, in order.

A file whose entries cannot be explained by one uniform grid is corrupt.

---

## 5. Codec identifiers — ADR-0004

| id | codec                     | v0.1 status |
|---:|---------------------------|-------------|
| 0  | Raw                       | implemented |
| 1  | Palette                   | implemented |
| 2  | Run-length                | implemented |
| 3  | Spatial predictor + entropy | implemented |
| 4  | LZ77 dictionary           | reserved |
| 5  | Predictor + dictionary    | reserved |
| 6  | Context-adaptive entropy  | reserved |

A decoder that meets a reserved or unknown codec id must fail with
`UnsupportedFeature`. It must not attempt a best-effort decode.

A tile is decoded with the codec named in **its own directory entry**
(section 4.1). The header's `default codec id` is informational and plays no
part in decoding a tile (section 2.3).

### 5.1 Raw codec (id 0)

The payload is the tile's canonical pixel bytes verbatim.
`compressed_length == uncompressed_length`. Encoding never expands or alters a
byte.

### 5.2 Palette codec (id 1) — ADR-0009

A per-tile palette. The palette is built from the tile's own pixels only. The
payload is:

```text
offset 0   u8   index-width code: 0=>1, 1=>2, 2=>4, 3=>8 bits; bits 2..7 zero
offset 1   u8   entry_count - 1    (entry_count is 1..=256)
offset 2   ..   entry_count * bytes_per_pixel  palette entries
...        ..   ceil(pixel_count * index_bits / 8)  packed indices, MSB-first
```

* Each palette entry is the tile's exact canonical pixel bytes, so alpha and
  every channel value are preserved.
* Entries are assigned in first-appearance order and indices are packed
  MSB-first; the unused low bits of the final byte are zero.
* The payload is canonical: byte 0's reserved bits are zero, the index width is
  the smallest that addresses `entry_count`, every index is `< entry_count`, the
  payload length is exactly the formula above, and the padding bits are zero. A
  decoder rejects any other form.
* A tile with more than 256 distinct pixel values is **not representable** by
  this codec; the encoder falls back to another codec (ultimately Raw). It is not
  an input error.

### 5.3 Run-length codec (id 2) — ADR-0010

Runs are counted in whole pixels (symbols), never bytes. The payload is a sequence
of self-delimiting tokens, each beginning with a 1-byte control:

```text
control 0x00..=0x7F   literal run: count = control + 1 (1..=128)
                      followed by count symbols, verbatim
control 0x81..=0xFF   repeat run:  count = (control & 0x7F) + 1 (2..=128)
                      followed by one symbol, repeated count times
control 0x80          malformed (a repeat run of length 1)
```

* The maximum run length is 128 symbols per token; longer runs are split.
* Tokens are self-delimiting, so a symbol is never mistaken for a control and no
  escaping is required.
* A decoder accepts any well-formed stream and rejects a length-1 repeat, a
  truncated token, a run past the expected pixel count, or a stream whose total
  does not equal the tile's pixel count. The encoder's exact output (pinned by
  golden vectors) is not the only well-formed stream.
* Only an empty tile or a payload above the per-tile size limit is **not
  representable**; the encoder then falls back to another codec.

### 5.4 Spatial predictor + entropy codec (id 3) — ADR-0011

A tile is stored as a per-row predictor plan followed by an adaptive
arithmetic-coded list of signed residuals. The payload is:

```text
offset 0   u8    version (must be 1)
...        ..    tile width in pixels, unsigned LEB128
...        ..    one predictor-mode byte per tile row
...        ..    residual count, unsigned LEB128
...        ..    entropy-coded residual data
```

* The width is in the payload so a tile is self-describing. The decoder
  recomputes the row count from the width and the tile's expected length, and
  rejects a width that does not divide it.
* The residual count equals `pixel_count * channels`. Any other count is
  corrupt.

**Predictors** (whole samples only — a 16-bit channel is one value, never two
bytes):

| code | predictor  | prediction                                 |
|------|------------|--------------------------------------------|
| 0    | none       | `0`                                        |
| 1    | left       | the previous pixel in the row, same channel |
| 2    | up         | the pixel above, same channel              |
| 3    | upper-left | the pixel above-left, same channel         |
| 4    | average    | `(left + up) / 2`, rounded down            |
| 5    | Paeth      | the Paeth choice of left/up/upper-left     |
| 6    | gradient   | the LOCO-I choice of left/up               |

A neighbour outside the tile — left of the first column, above the first row,
upper-left of either — is `0`. Prediction resets at every tile boundary, so no
tile depends on another. A predictor code above 6 is corrupt.

**Residuals.** For a sample of `B` bytes, the residual is
`(sample - prediction) mod 2^(8B)` folded to `[-2^(8B-1), 2^(8B-1) - 1]`.
`prediction + residual mod 2^(8B)` reproduces the sample exactly. The predictors
are integer functions of whole samples; no floating point is involved.

**Entropy coder.** An adaptive binary arithmetic coder with 32-bit
`low`/`range`, E1/E2/E3 renormalization and a byte flush. Each context holds
an 11-bit probability that moves 1/32 of the way toward the symbol just seen.
A residual is coded as one sign bit plus one decision per magnitude plane,
most significant first; before the first 1-bit each plane uses its own "not
started yet" context, and after it each plane uses its own plane context. The
coder is a terminator on the residual count: exactly that many residuals are
decoded and the decoder never has to detect end-of-stream. Bytes read past the
end of the payload read as zero.

* The encoder chooses a predictor per row by measuring each candidate with the
  live coder's probability model and keeping the cheapest; ties go to the
  lowest predictor code. The measurement uses a baked-in fixed-point `log2`
  table, so the choice is identical on every host.
* The payload is not canonical: any well-formed stream decodes to the same
  pixels. The encoder's output is pinned by golden vectors.
* Only an empty tile or a payload above the per-tile size limit is **not
  representable**; the encoder then falls back to another codec.

---

## 6. Integrity

Two independent layers:

1. **Per-tile CRC32C** of the compressed payload bytes, stored in the entry.
2. **Whole-image SHA-256 canonical digest** over the canonical pixel buffer,
   defined in section 7. It is computed on demand and is not stored in v0.1.

CRC32C detects accidental corruption; it is not a security primitive. SHA-256
provides the strong identity used by `vpng verify` and by golden tests.

### 6.1 Which command checks what

Integrity is layered deliberately, and each command exposes one layer. No command
claims more than it does.

| Command | Reads | Detects | Cost |
|---------|-------|---------|------|
| `vpng info` | header + directory only | bad magic/version/size, header checksum, directory checksum, coverage and range violations | O(1) in tile count — **not** an integrity check |
| `vpng check` | every tile payload | all of the above **plus** every per-tile CRC32C | O(file size), allocates no pixel buffer |
| `vpng verify` | candidate file + original image | all of the above upon decode, **plus** exact pixel equality against a reference | O(file size) plus a full decode |

`info` is intentionally shallow: it must stay usable on a multi-gigabyte file.
It prints an explicit note that payloads were not verified, so its output cannot
be mistaken for a clean bill of health.

---

## 7. Canonical digest

```text
sha256( "VPNG-CANON-v1\0"
      || le_u32(width)
      || le_u32(height)
      || u8(color_model_id)
      || u8(bits_per_channel)
      || canonical_pixel_bytes )
```

The prefix domain-separates this digest from any other SHA-256 use. Changing the
prefix or the field order is a format-level change requiring a new ADR.

---

## 8. Resource limits

A decoder applies configurable limits before allocating or trusting anything
declared in the file. Defaults:

| Limit | Default |
|-------|---------|
| max width / height | 65,536 |
| max pixels | 1,073,741,824 |
| max tile count | 16,777,216 |
| max compressed tile payload | 268,435,456 bytes |
| max decompressed bytes (per tile) | 2,147,483,648 |
| max file size | 17,179,869,184 bytes |
| max expansion ratio (uncompressed / compressed) | 4096 |

Exceeding a limit yields `ResourceLimit`, exit code 5.

---

## 9. Size ceiling

`--max-size` is a decimal byte ceiling (`10MB` = 10,000,000). `--max-size-mib`
is a binary ceiling (`10MiB` = 10,485,760). See ADR-0006.

If no lossless representation fits, encoding fails with `SizeLimitExceeded`
(exit code 6) and reports the smallest lossless size found. The encoder never
writes a partial file and never trades away information to meet the ceiling.

---

## 10. Exit codes

| Code | Meaning |
|-----:|---------|
| 0 | success |
| 1 | generic failure / internal error |
| 2 | invalid arguments |
| 3 | unsupported feature or version |
| 4 | corrupt file |
| 5 | resource limit exceeded |
| 6 | size limit exceeded |
| 7 | verification failure |
| 8 | I/O failure |

---

## 11. What v0.1 deliberately does not define

Animation, layers, lossy or neural paths, floating-point/HDR channels, indexed
palettes, chunks and metadata, preview pyramids, multi-generation editing
journals, GPU decoding, and external dictionaries. Each has a documented
extension point and a reserved id or field, and each must be specified and
tested before it can be claimed.
