# ADR-0003: The directory is the single source of truth

**Status:** accepted
**Date:** 2026-11-12
**Question (roadmap §18.3):** Is the directory inline or chunk-based, and which
source is authoritative if a `TIDX` chunk also exists?

## Decision

The directory is a fixed region addressed by the header's `directory_offset` and
`tile_count`. It is authoritative. v0.1 defines **no** `TIDX` chunk at all, so
two sources of truth cannot exist.

## Rationale

* A single authoritative directory removes an entire class of bug: a file whose
  two indexes disagree, or whose chunk wins on one reader and loses on another.
* The header already carries the offset and count, so a chunk would be
  redundant.
* A future version may add a `TIDX` chunk only as a *redundant cache* with a
  documented precedence rule, or not at all.

## Consequences

* `repair-index` (roadmap §12) can only ever rebuild whatever the directory
  region itself plus the payload regions imply; it can never invent pixels.
* Readers must reject a file whose `directory_offset`/`tile_count` cannot
  describe a well-formed directory.
