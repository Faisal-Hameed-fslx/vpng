# Editing model (design)

Editing is a first-class subsystem, not an afterthought, but it is **not yet
implemented**. This document records the intended design so that v0.1 container
choices do not preclude it.

## Document

A VPNG document is expected to hold: an immutable metadata snapshot, the tile
index, references to compressed payloads, a bounded decoded-tile cache, a dirty
tile set, transaction state, optional undo/redo history, and a current
generation id. The editor must never require the whole decoded image in RAM.

## Localized edit flow

1. Map the edit rectangle to affected tile ids.
2. Load only those tiles; decode into reusable buffers.
3. Apply the exact pixel operation.
4. Mark affected tiles dirty.
5. Re-encode only dirty tiles.
6. Reuse unmodified payloads byte for byte where the layout allows.
7. Build a new directory generation.
8. Write a temporary file or append a new safe generation.
9. Validate the new generation.
10. Atomically replace the old file where the platform supports it.

The only valid file state is never mutated before a complete, verified new state
exists.

## Transaction API (target)

```rust
pub trait EditTransaction {
    fn read_region(&mut self, rect: Rect) -> Result<PixelRegion>;
    fn write_region(&mut self, rect: Rect, pixels: &PixelRegion) -> Result<()>;
    fn commit(self) -> Result<CommitInfo>;
    fn abort(self) -> Result<()>;
}
```

The implementation must make invalid states hard to express; exact ownership and
lifetimes may differ from this sketch.

## Undo/redo

Tile-level copy-on-write initially: store references to prior and new tile
payloads, never duplicate unchanged tiles. Undo restores exact previous pixels;
redo restores exact edited pixels. History has an explicit byte budget and an
explicit eviction policy. Crash recovery must never mistake an incomplete
generation for committed data.

## Why v0.1 already supports this

* Tiles are independently decodable, so a localized edit needs only its tiles.
* Per-tile CRC allows a rewritten tile to be validated without re-reading the
  whole file.
* The directory is the single authority (ADR-0003), so a new generation is a
  new directory plus new payloads — not a two-index reconciliation problem.

## Atomic replacement per platform (roadmap §18.9, §18.10)

* Linux/macOS: write a sibling temp file, `fsync`, then `rename(2)` over the
  target.
* Windows: `MoveFileEx` with `MOVEFILE_REPLACE_EXISTING` (or
  `ReplaceFileW`) after flushing the temp file.
* Append-only generations are the fallback where atomic replace is unavailable;
  the generation is not "committed" until a directory generation referencing it
  is written and its checksum validates.

This ADR is pending: it will be recorded as ADR-0009 when P4 begins.
