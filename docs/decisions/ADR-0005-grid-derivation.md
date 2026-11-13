# ADR-0005: Deriving the tile grid from the directory

**Status:** accepted
**Date:** 2026-11-12
**Question:** The header has no field for the nominal tile size, and the 8
reserved bytes must remain zero in v0.1. How does a decoder recover the grid?

## Decision

Derive it from the directory:

* Let `xs` be the distinct `tile x` origins in ascending order. If `xs` has two
  or more entries, the nominal tile width is `xs[1] - xs[0]`; otherwise it is
  the image width. `xs[0]` must be 0.
* Symmetrically for `ys` and the nominal tile height.
* Build the uniform grid with that tile size. Its tile count must equal the
  header's `tile_count`, and the directory entries must equal the grid's
  geometry in canonical order.

Rejected alternative: repurposing the 8 reserved bytes as tile width/height.
The roadmap states reserved fields must be zero in v0.1, and a decoder must not
depend on a value the spec says is zero.

Rejected alternative: storing the tile size in a chunk. v0.1 defines no chunks,
and a redundant tile size could disagree with the entries.

## Rationale

* Keeps the 64-byte header exactly as specified.
* The grid is fully determined by data that must already be present and
  consistent, so there is nothing extra to validate against.
* Uniform tiles are all v0.1 supports, so a uniform grid is not a limitation.

## Consequences

* A file whose directory does not describe one uniform grid is corrupt.
* When adaptive (variable-size) tiles arrive in P3, they need an explicit
  partition description; this ADR will be superseded, not edited.
