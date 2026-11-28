# ADR-0019: GPU block payloads, and the v0.5 version rule

**Status:** accepted
**Date:** 2026-11-27
**Related:** ADR-0002 (the directory entry), ADR-0004 (the codec id table),
ADR-0014 (levels and partitions), ADR-0015 (the v0.3 version rule),
ADR-0018 (layers, masks, and the v0.4 version rule), the roadmap §10.3 (CPU
and GPU responsibilities) and §21.1 (GPU texture block support, P15)
**Question:** The roadmap asks for "GPU texture block support (BC7/ASTC)" as
P15, on the same terms as P14: a format revision, on the record. The
questions are: where do the GPU blocks live, how does a reader find them,
what does the codec table do, and — the one that matters most — how does a
**lossy** payload sit next to a **lossless** one without ever being mistaken
for it?

## Decision

### 1. The GPU payload is a separate payload kind, never a substitute

A tile keeps **two** payloads:

* the **lossless payload**, named by the directory entry's `codec_id`, which
  is what every exact operation uses; and
* the **GPU payload**, an optional sibling, named by a **separate table**.

The GPU payload is **lossy**: BC1 and BC4 are block formats that throw
information away. That is the whole point of them — they are what a GPU can
sample as a texture — and it is also why they may never stand in for the
lossless payload.

So the rule is absolute:

* the directory entry's `codec_id` **never** names a GPU block codec;
* the decoder's exact paths — `decode_region`, `decode_layer`, `composite`,
  `verify`, `check` — **never** read the GPU payload;
* the GPU payload is read only by an explicitly named API that says it is
  lossy.

A file with a GPU payload therefore still reconstructs **exactly**, and
`vpng verify` still passes on it. The GPU payload is an *addition*, never a
substitution.

### 2. The GPU codecs get their own ids, and the codec table changes

P15 changes the codec table. Two ids are added:

| id | name | channels | bytes per 4×4 block |
|---:|------|---------:|--------------------:|
| 7 | `bc1` | 3 (R, G, B) | 8 |
| 8 | `bc4` | 1 | 8 |

They are **not** lossless tile codecs, so they are **not** in
`IMPLEMENTED_CODEC_IDS`, which stays the set the cost-based selector may
choose from and the set a directory entry may name. A file whose directory
entry names id 7 or 8 is refused with `UnsupportedFeature`: a GPU block codec
may not be used as a tile codec.

The ids live in `vpng-codecs` (the codec table's home) and the implementations
live in a new crate, `vpng-gpu`, because `vpng-codecs`'s own contract is that
a codec is *exactly reversible* and BC1 is not. Putting a lossy codec behind
that trait would be a lie; putting it in its own crate keeps the contract
true.

### 3. The header grows to 72 bytes, using the field that exists for it

The 64-byte header has no room left: bytes 56..64 hold the layer table's
location since v0.4, and the directory entry's 40 bytes are all in use.

So the header **grows**, and it grows through the field that was put there
for exactly this: `header_size` (bytes 12..16). A v0.5 file's header is **72
bytes**:

| Offset | Size | Field |
|-------:|-----:|-------|
| 0 | 8 | magic |
| 8 | 2 | major version |
| 10 | 2 | minor version |
| 12 | 4 | header size (64, or 72 from v0.5) |
| 16 | 24 | width, height, channels, bits, colour model, default codec, tile count |
| 32 | 8 | directory offset |
| 40 | 8 | declared file size |
| 48 | 4 | feature flags |
| 52 | 4 | header checksum (CRC32C) |
| 56 | 4 | layer table offset |
| 60 | 4 | layer table length |
| 64 | 4 | **GPU table offset** (v0.5) |
| 68 | 4 | **GPU table length** (v0.5) |

The header checksum covers bytes `0..52` and `56..header_size`. For a 64-byte
header that is exactly the rule v0.1–v0.4 use, so **no existing file's
checksum changes** and no existing byte moves. A v0.1–v0.4 file's header is
still 64 bytes and still says so.

**Why not derive the GPU table's position instead?** It would have been
possible — the table has a fixed size, one record per tile — and it would have
saved the header growth. It was rejected: an implicit position is a rule a
reader has to know and a writer cannot change, and the header already has a
field whose entire purpose is to say how long the header is. The growth costs
**eight bytes in a file that asked for the feature**, and nothing at all in a
file that did not.

### 4. The GPU table sits with the other tables and holds one record per tile

The file layout for a v0.5 file:

```text
header (72 bytes)
tile directory
layer table        (only when the header says so)
GPU table          (only when the header says so)
tile payloads and GPU payloads, interleaved in canonical tile order
```

The GPU table is a 24-byte header — record count, descriptor-array length, a
CRC32C over the descriptor array, a reserved word, and the total number of GPU
payload bytes — followed by one **20-byte record per tile**, in the same
canonical order as the directory.

The header is the layer table's own header (count, length, checksum, one
reserved word) plus the payload total, and it is **24 bytes** rather than the
32 an earlier draft of this record proposed: the extra reserved word bought
nothing, because a reader already learns the table's length from the main
header's `gpu table length` field and learns where the descriptor array
starts from this header's fixed size. The project's own rule — that a
feature costs bytes only in a file that asked for the feature — says the
smaller header is the right one.

One record per tile, rather than a record per payload that exists, because
the record is then *indexed by tile*, a reader can find a tile's GPU payload
without scanning, and the table's size is a function of the tile count alone.
A tile with no GPU payload says so: both codec ids are zero and the offset
and length are zero.

Each record carries the GPU payload's **absolute offset**, its **length**, the
**codec id** of its colour blocks, the **codec id** of its alpha blocks, and a
**CRC32C** of the payload. The absolute offset matters: it is what lets an
HTTP Range reader fetch exactly one tile's GPU payload and nothing else.

### 5. The GPU payloads are interleaved with the lossless payloads

Each tile's lossless payload is followed by its GPU payload, and the next
tile's lossless payload follows that.

The alternative — one contiguous region of GPU payloads — would be tidier to
describe and would need the whole GPU region in memory before it could be
written, which for the 16K photograph is about 70 MB. That is more than the
64 MB budget the project holds itself to for localized editing, and the
streaming encoder exists precisely so that the whole output is never held.

Interleaving costs nothing to a range reader: a consumer fetches
`[payload_offset, payload_offset + compressed_length)` and
`[gpu_offset, gpu_offset + gpu_length)`, and both are exact ranges. The bytes
that sit between them are bytes the consumer never asked for.

### 6. Only 8-bit formats have GPU payloads

BC1 and BC4 are 8-bit block formats. A 16-bit file has no GPU payload, and
the encoder **refuses** to write one rather than quietly dropping the
precision or quietly writing a lossy 8-bit copy of a 16-bit picture.

The format mapping is fixed:

| file format | colour blocks | alpha blocks |
|---|---|---|
| `rgb8` | BC1 | none |
| `rgba8` | BC1 | BC4 |
| `gray8` | BC1 (the grey value in all three channels) | none |
| `graya8` | BC1 (the grey value in all three channels) | BC4 |
| a **mask layer's** tile | none | BC4 (the coverage) |

A grey picture is replicated into RGB because that is what a GPU wants: a
texture is RGB, and the replication costs at most four levels of error on a
5-bit red channel. It is measured, not assumed.

A **mask layer** carries only its coverage, so its GPU payload is BC4 alone.
The mask's colour channels carry no meaning and are not encoded.

### 7. The minor version states what the file **contains**

The rule is extended again:

1. if the file carries a **GPU payload table**, it is a **v0.5** file;
2. otherwise, if the file carries a **layer table**, it is a **v0.4** file;
3. otherwise, if any entry names codec id 4, 5 or 6, it is a **v0.3** file;
4. otherwise, if the file uses a v0.2 feature, it is a **v0.2** file;
5. otherwise it is a **v0.1** file.

The default encoder path writes no GPU table, so it keeps writing the exact
v0.1 bytes the golden fixtures pin. **v0.5 moves no published byte.** The
GPU path is opt-in, like every other format feature in this project.

### 8. The editing engine refuses a file with a GPU table, and says so

The editing engine's commit path rewrites the directory and the payloads. It
does not know how to carry the GPU table and its per-tile records through that
rewrite.

So it **refuses**, with an `UnsupportedFeature` whose message says "GPU
table". Silently dropping the GPU payloads would lose work the user asked
for; writing a half-table would produce a file that claims payloads it does
not have. A refusal is loud, typed and recoverable.

## Rationale

* **The lossless payload has to stay normative.** The whole format exists to
  reconstruct pixels exactly. A GPU payload is a *preview in the format a
  GPU speaks*, and the moment it can be mistaken for the real thing the
  format's central promise is gone. Keeping it in a separate table, with
  separate codec ids, with its own checksum, and behind an explicitly named
  API, makes the mistake impossible to make by accident.
* **BC1 and BC4 are the honest first block codecs.** BC7 is the one the
  roadmap names, and BC7's mode 6 needs a subset search over endpoint
  candidates that is a research project of its own. BC1 (DXT1) and BC4
  (ATI1N) are the formats every GPU in the world already samples, they are
  small enough to implement exactly and to test against the reference
  arithmetic, and they cover the two cases that matter: colour, and a single
  channel. They are **not** claimed to be BC7 or ASTC.
* **A GPU is not required for correctness.** Roadmap §10.3 says so, and the
  design honours it: a reader with no GPU reads a v0.5 file exactly, ignores
  the GPU payloads, and gets the original pixels. The GPU payload is
  optional in the strongest sense — it is optional to *use*, and optional to
  *write*.
* **The size of both payloads is reported.** The release gate is that "the
  size of both is reported so the trade is visible rather than implied". A
  file that carries a GPU payload is bigger, and the report says by how much,
  and what the pixels cost in fidelity for it. Nothing is hidden behind a
  flag.

## Consequences

* The **64-byte header and the 40-byte directory entry keep their exact
  layouts** for every file that does not ask for a GPU payload. A v0.5 file
  has a **72-byte header**, and its directory starts at 72 rather than 64.
* A file may hold a GPU payload table, a layer table, a preview pyramid and
  promoted codecs **at the same time**. The GPU payload is orthogonal to all
  of them: it is a sibling of the lossless payload, not a replacement for it.
* The GPU payload path supports **8-bit formats only**.
* The **editing engine refuses a v0.5 file**. Carrying the table and its
  records through a commit is the next piece of work in that engine.
* The GPU codecs are **not** in `IMPLEMENTED_CODEC_IDS`, so the cost-based
  selector never picks one and the published corpus numbers do not move.
* BC7 and ASTC are **not** implemented. The ids are free for them.

## Evidence

`docs/P13_GPU_REPORT.md` — the twelve questions, the acceptance gates, and the
measurement over the whole 750-image corpus: every file's lossless payload
still reconstructs **exactly**, the GPU payload's fidelity is measured against
the canonical pixels, the size of both payloads is reported per file and per
category, and the default path's total is the published baseline's
**453,767,460** bytes. The format's rules are pinned by the tests in
`crates/vpng-testkit/tests/v5_formats.rs`, the encoder's by the tests in
`crates/vpng-encoder/tests/gpu_identity.rs`, the block codecs' by the unit
tests in `crates/vpng-gpu`, the refusal's by the tests in
`crates/vpng-editing/tests/v5_gpu.rs` and the command line's by the tests in
`crates/vpng-cli/tests/v5_features.rs`.
