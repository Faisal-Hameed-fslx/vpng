# ADR-0018: Layers, masks, and the v0.4 version rule

**Status:** accepted
**Date:** 2026-11-27
**Related:** ADR-0002 (the directory entry), ADR-0004 (the codec id table),
ADR-0005 (grid derivation), ADR-0014 (levels and partitions), ADR-0015 (the
v0.3 version rule), ADR-0016 (the HTTP Range source), ADR-0017 (the
WebAssembly ABI)
**Question:** The roadmap asks for "layers, masks and non-destructive
editing" as one of the format's milestones. The questions are: where do the
layer names and their rectangles live, how does a reader know which tile
belongs to which layer, may a file have both a preview pyramid and layers,
what exactly is a mask, and what does the minor version say about all of it?

## Decision

### 1. The layer table lives between the directory and the payloads

The 64-byte header's **last eight bytes** — reserved in v0.1–v0.3, and
required to be zero there — become the **layer table's offset and length**
(u32 each). The table itself sits between the tile directory and the tile
payloads.

Putting the table *after* the directory rather than inside it is what lets
the directory's entry layout stay exactly 40 bytes. Putting it *before* the
directory would have meant either moving the directory offset or growing the
header, and both would have moved bytes in every existing file.

The table is a 16-byte header — count, descriptor-array length, a CRC32C
over the descriptor array, one reserved word — followed by the descriptors.
Each descriptor is 40 bytes plus its UTF-8 name.

The header's table length is zero exactly when there is no table, and the
feature flag `FEATURE_LAYERS` is set exactly when the length is non-zero.
Both halves are checked, so a file that claims a table and does not have one
is corrupt.

### 2. The layer index travels in the directory entry's `flags`

The entry's `flags` field is a u16. Since v0.2 its low byte has carried the
**level index**. In v0.4, bits 8..11 carry the **layer index**, and bits
12..15 are reserved.

Four bits means **at most 16 layers**. That is a deliberate limit: it keeps
the index inside the field that already exists, so the entry stays 40 bytes,
and sixteen layers is far more than the format's use cases need.

In a v0.2 or v0.3 file, bits 8..15 remain reserved and must be zero, exactly
as they were in v0.1. A reader applies the rule that matches the file's
minor version, so an old file cannot accidentally carry a layer index.

### 3. The canonical order becomes `(layer, level, y, x)`

For a file with no layer table, the layer index is zero for every entry, so
the order is exactly v0.2's `(level, y, x)` order and **nothing moves**.

A reader rejects a file whose entries are out of canonical order. The reason
is practical: a reader must be able to find a layer's tiles without scanning
the whole directory, and contiguity is what makes that possible.

### 4. A layered file may not have a preview pyramid

`FEATURE_PYRAMID` and `FEATURE_LAYERS` are **mutually exclusive**, and every
entry of a layered file is at level 0.

Two reasons:

* the accounting would otherwise be ambiguous — a layer's "uncompressed
  bytes" would have to say whether it includes the derived levels;
* the pyramid exists to make a **large flat picture** cheap to preview, and a
  layered file is already a set of independently decodable grids, each of
  which can be previewed by itself.

The alternative — a pyramid per layer — was considered and rejected: it
multiplies the derived data by the layer count, and it makes the per-layer
byte accounting need a second rule.

### 5. A mask is a layer whose alpha channel is its coverage

A **mask** is a layer with a kind flag set. Its **alpha channel** is the
coverage: 0 means the layer it masks is invisible there, 255 means fully
visible.

* A file with a mask must use a format with an **8-bit alpha channel**
  (`graya8` or `rgba8`). A format without one cannot carry a mask, and the
  encoder refuses to write one.
* A mask layer is **never composited on its own**. It is coverage for another
  layer, not a picture.
* A layer references **at most one** mask; a mask may be referenced by any
  number of layers.
* The mask's rectangle must **cover** the rectangle of the layer it masks. It
  may be larger; it may not be smaller.

A separate "mask" structure — a rectangle plus a coverage buffer, rather than
a layer — was considered and rejected. It would need its own directory-like
index, its own checksums and its own random-access story, and it would make
the layer count and the mask count two different things to reason about. A
mask as a layer reuses everything: the directory, the codecs, the tile
geometry and the checksum.

### 6. Layers are encoded independently; cross-layer prediction is out of scope

Each layer's tiles are encoded on their own, with the same per-tile codec
selection as any other file. **No layer predicts from another**, and no
state crosses a layer boundary.

The consequence is a property worth having and worth measuring: **adding an
overlay does not change the base layer's bytes.** The corpus measurement
checks that for every image: the base layer's compressed and uncompressed byte
counts are the same numbers in the one-layer file and in the two-layer file.

Cross-layer prediction would be a real win for an overlay that closely
resembles the base. It was left out because it makes the layers
**inter-dependent**, which in turn means a layer cannot be decoded, re-encoded
or replaced on its own — and "a layer can be decoded on its own" is the
property the layer model exists to provide.

### 7. The compositing is straight alpha, integer, and deterministic

The blend is straight-alpha "source over". The weight of a pixel is

```text
w = source_alpha * opacity * coverage / (max * 255 * max)
```

carried as a **fraction**, not as a floating-point number, so the result does
not depend on the host's floating-point behaviour. Every operation is
integer, so the same file composites to the same bytes on every platform. The
alpha channel blends on its own, as the weight itself.

**A one-layer file composites to exactly its own pixels.** The blend of a
single layer is the identity, not an approximation, and no rounding can creep
in. This is pinned by the test suite and measured on 750 of 750 corpus
images.

### 8. The minor version states what the file **contains**

The rule is extended, not replaced:

1. if the file carries a **layer table**, it is a **v0.4** file;
2. otherwise, if any entry names codec id 4, 5 or 6, it is a **v0.3** file;
3. otherwise, if it uses a v0.2 feature (a pyramid or an adaptive partition),
   it is a **v0.2** file;
4. otherwise it is a **v0.1** file.

The default encoder path writes no layer table, permits only ids 0–3 and
requests neither a pyramid nor an adaptive partition, so it keeps writing the
exact v0.1 bytes the golden fixtures pin. **v0.4 moves no published byte** —
and the corpus measurement confirms it: the total is 453,767,460 bytes, the
published stable baseline's, to the byte.

A v0.4 file with one layer is a valid v0.4 file. An older reader rejects it
for the ordinary reason: the minor version is newer than the reader knows.

### 9. The editing engine refuses a layered file, and says so

The editing engine's commit path rewrites the directory and the payloads. It
does not yet know how to carry the layer table and the per-layer accounting
through that rewrite.

So it **refuses**, with an `UnsupportedFeature` whose message says "layer
table". Silently dropping the layers would lose the user's work, and a
half-written layer table would produce a file that claims to have layers it
does not have. A refusal is loud, typed and recoverable; the alternatives are
not.

## Rationale

* **The reserved bytes were put there for this.** A format revision that
  grows the header or the directory entry moves bytes in every file ever
  written. v0.4 uses two fields that were *reserved*, so no existing file
  changes and no existing byte moves. The price is a four-bit layer limit and
  a fixed-cost table, and both are cheap.
* **The table's cost is fixed, and it is 60 bytes.** Measured over 750
  images: exactly 60 bytes per file, 45,000 bytes over the corpus,
  0.009917% of the corpus's bytes. It is **not** proportional, so on a
  pixel-art file it is **1.643426%** — and on the corpus's smallest file,
  111 bytes, it is **54.054054%**. Those numbers are reported rather than
  averaged away. A user who does not need layers pays nothing.
* **Independent layers are worth more than smaller overlays.** A format whose
  layers can be decoded, replaced and re-encoded one at a time is a format a
  document editor can build on. A format that predicts across layers is
  smaller and is not.
* **The accounting must be exact.** A reader recomputes every layer's
  uncompressed and compressed byte counts from the directory and rejects a
  mismatch, so the per-layer ratio in a file is a measurement and not a
  claim.
* **A refusal beats a silent loss.** The editing engine's refusal is the
  honest behaviour while the commit path cannot carry layers.

## Consequences

* The **64-byte header and the 40-byte directory entry keep their exact
  layouts**. The two fields that changed were reserved.
* A file may hold **at most 16 layers**, and a layered file may not have a
  preview pyramid.
* The **layered path is opt-in**. The default path writes no table and the
  published bytes do not move.
* The **editing engine refuses a layered file**. Carrying the table and the
  accounting through a commit is the next piece of work in that engine.
* The **HTTP Range reader and the WebAssembly module read the base layer**.
  The pieces are there — `decode_layer_from` is public and the range reader
  fetches the table — but the compositing path is not wired up, and
  `vpng http-decode-region` says so when a file has more than one layer.
* The **composite is a whole-canvas operation**. It holds the whole canvas:
  527,650,816 bytes for the 16K photograph's one-layer composite. That is
  not the 64 MB localized-editing target, which is a different operation.

## Evidence

`docs/P12_LAYERS_REPORT.md` — the twelve questions, the acceptance gates,
and the measurement over the whole 750-image corpus: **750 of 750**
one-layer composites are **byte-identical** to the canonical pixels; **750 of
750** base layers of two-layer files are byte-identical to the original; **0**
files failed `vpng check`; the layer table costs exactly **60 bytes** per
file — and **54.054054%** of the corpus's smallest file, which is the
unfavourable end of that number; and the default path's total is the
published baseline's **453,767,460** bytes. The format's rules are pinned by
sixteen tests in `crates/vpng-testkit/tests/v4_formats.rs`, the encoder's by
six in `crates/vpng-encoder/tests/layer_identity.rs`, the blend's by nine
unit tests in `crates/vpng-decoder/src/composite.rs`, the refusal's by three
in `crates/vpng-editing/tests/v4_layers.rs` and the command line's by nine
in `crates/vpng-cli/tests/v4_features.rs`.
