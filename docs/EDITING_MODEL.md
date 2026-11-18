# Editing model

Editing is a first-class subsystem, not an afterthought. It is **implemented**
in the `vpng-editing` crate and its design is recorded here.

The implementation and this document describe the same thing. Where the
document quotes a type or a method, the type or the method exists; the
evidence for the claims is in `docs/P4_EDITING_ENGINE_REPORT.md`.

---

## Document

A VPNG editing session holds:

* a **source** of tiles — either a file already in memory
  (`DocumentSource`) or a file on disk (`FileSource`);
* a **bounded cache** of decoded tiles (`TileCache`), with a byte budget and
  a strict LRU eviction policy (ADR-0013);
* a **dirty tile set** — the tiles whose pixels differ from the pixels the
  file stores for them;
* a **tile-level undo/redo history** (`History`), with a byte budget;
* **transaction state**, when a transaction is open.

It never holds the whole decoded image. A region read allocates exactly the
region it returns; a region write touches exactly the tiles the region
intersects. The unit test `the_engine_never_holds_a_full_image_buffer`
asserts this against the allocator.

The engine is generic over its source — `Editor<S: TileSource>` — so the
same editing code serves a small file that is already in memory and a 16K
file that is far too large to hold in memory.

---

## Terminology

Four different tile counts appear in the reports and the benchmark output,
and they are **not** interchangeable. This document uses them as follows.

| Term | Meaning |
|---|---|
| **decoded** | the tile's compressed payload was read and turned back into pixels. A tile is decoded when the engine needs its pixels — to read a region, or to patch part of the tile. |
| **modified** | the tile's **pixels** changed. A modified tile is one the editor's dirty set holds. |
| **rewritten** | the tile's **encoded payload** was replaced during a commit. A rewritten tile is re-encoded with the encoder's own `select_payload`. |
| **preserved** | the tile's original compressed payload was copied into the new file **byte for byte**. A preserved tile was neither decoded nor re-encoded during the commit. |

The counts can differ, and the reports say which is which:

* A tile can be **decoded** and **modified** but never **rewritten** — for
  example when the edit is later undone, or when the commit fails.
* A tile can be **decoded** and **rewritten** without being *modified* — for
  example when a commit re-encodes a tile the engine held in its dirty set.
* The tiles a commit **preserves** are exactly the tiles it does not
  rewrite. `tiles_rewritten + tiles_copied == tile_count`.

"Decoded" and "rewritten" are therefore never reported as the same number.
Where a benchmark reports a single tile, it reports which of the four terms
it means.

---

## Localized edit flow

The implemented flow, in order:

1. Map the edit rectangle to the tiles it intersects
   (`TileGrid::geometries_for_rect`).
2. Decode only those tiles, into the bounded cache.
3. Apply the exact pixel operation — patch the intersecting part of each
   tile.
4. Mark the affected tiles dirty, and record the previous pixels in the
   history.
5. On save, re-encode only the dirty tiles, through the encoder's own
   `select_payload`, so a re-encoded tile is exactly the tile the encoder
   would have produced. A rewritten tile's directory entry describes the
   payload that was just written: its `predictor_id` and `flags` are `0`,
   never the values of the tile it replaced.
6. Copy every unmodified payload **byte for byte** from the source.
7. Build the new directory and the new header.
8. Write a temporary file in the same directory as the target.
9. `flush` and `sync_all` the temporary file.
10. Replace the target in one step: `std::fs::rename`, which on Windows is
    `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`.
11. Move the session's **baseline** to the committed file: the undo and redo
    history is cleared, the edit counter restarts, the open transaction ends,
    and the dirty set is reconciled with the new baseline.

The original file is never written to. The only valid file state is the
one that is already complete; a new state exists only once the temporary
file is whole and has replaced the target.

---

## Transaction API

```rust
impl<S: TileSource> Editor<S> {
    pub fn begin(&mut self) -> Result<()>;
    pub fn abort(&mut self) -> Result<()>;
    pub fn commit(&mut self, target: &Path, options: &CommitOptions) -> Result<CommitReport>;

    pub fn read_region(&mut self, rect: Rect) -> Result<CanonicalImage>;
    pub fn write_region(&mut self, rect: Rect, pixels: &[u8]) -> Result<()>;

    pub fn undo(&mut self) -> Result<bool>;
    pub fn redo(&mut self) -> Result<bool>;
}
```

The states that cannot be expressed are rejected: a second `begin` is an
error, an `abort` without an open transaction is an error, and a
`write_region` whose buffer length does not match the rectangle is an
error.

The two sources give two entry points:

```rust
let mut editor = Editor::parse(&bytes, &limits)?;        // borrowed, in memory
let mut editor = FileEditor::open(path, &limits)?;        // on disk
```

`EditableDocument<'a>` is the type alias for the borrowed form.

---

## Undo / redo

The history is **tile-level** and holds the *previous pixels* of every tile
a write touched. An undo restores those pixels and marks a tile **clean**
again if the pixels now match what the file stores — so an undone tile is
not rewritten by the next save. A redo restores the edited pixels.

### The commit boundary

**A successful commit establishes a new baseline, and undo and redo never
cross it.** The committed file is what the session edits from that moment
on, so the steps that *reached* the baseline are not undoable steps of the
sequence that starts after it.

After a successful commit the engine:

* clears the **undo** history;
* clears the **redo** history;
* resets the **edit counter** (`edit_count()` returns `0`);
* clears the **transaction state** — an `abort` after a commit is an error,
  because no transaction is open any more;
* reconciles the **dirty set** with the committed baseline.

A commit that **fails** changes nothing at all. The dirty tiles, the undo
and redo history, the open transaction, and the edit counter all survive, so
the user can try again or undo the edits. The history is cleared only
*after* the file has been successfully written and renamed into place.

### How the baseline moves

The dirty-set reconciliation depends on whether the source can re-read the
file that was just written:

* `FileSource` **can**. When the commit replaced the source's own file, the
  source re-opens the path and re-reads the header, the directory, and the
  grid. The committed file stores exactly the pixels the session holds, so
  the dirty set becomes empty again. The cache entries for the rewritten
  tiles are replaced with the pixels the commit wrote, so a later read
  cannot see the stale pre-edit pixels. `CommitReport::baseline_advanced`
  is `true`.
* `DocumentSource` **cannot**. Its bytes are borrowed and immutable, so it
  keeps the pre-commit baseline. The dirty set is therefore **kept**: the
  source still stores the old pixels, and dropping the dirty set would
  silently revert the edit on the next save. `CommitReport::baseline_advanced`
  is `false`.

A commit to a path other than the source's own file also leaves the
baseline where it was, for the same reason.

The history has an explicit byte budget and an explicit eviction policy:

* the oldest entries are dropped when the budget is exceeded, and the
  count of dropped entries is reported in `dropped_entries`;
* an entry too large for the whole budget is dropped at once, rather than
  being retained over the bound;
* a new edit invalidates the redo chain.

The peak is recorded **after** trimming, so the reported peak is the
high-water mark of the bytes the history actually retains and can never
exceed the budget.

A commit starts a **new history sequence**: the peak and the dropped-entry
count are reset, so they describe the sequence that is now in progress
rather than the one that just ended.

The history is in memory only. A crash recovery must never mistake an
incomplete generation for committed data; because the engine never writes
to the original file and replaces it in one step, there is no incomplete
generation to mistake.

---

## Why v0.1 already supports this

* Tiles are independently decodable, so a localized edit needs only its
  tiles.
* Per-tile CRC32C allows a rewritten tile to be validated without
  re-reading the whole file.
* The directory is the single authority (ADR-0003), so a new generation is
  a new directory plus new payloads — not a two-index reconciliation
  problem.

---

## Atomic replacement per platform (roadmap §18.9, §18.10)

Recorded in full in `docs/decisions/ADR-0012-atomic-replacement.md`.

* **Linux / macOS:** write a sibling temporary file, `fsync`, then
  `rename(2)` over the target.
* **Windows:** `std::fs::rename`, which is `MoveFileExW` with
  `MOVEFILE_REPLACE_EXISTING`, after flushing and `sync_all`ing the
  temporary file.
* **Append-only generations** remain the fallback for a platform where an
  atomic replace is unavailable. The generation is not "committed" until a
  directory generation referencing it is written and its checksum
  validates.

The temporary file is written in the **same directory** as the target, so
the replacement is a rename within one volume and cannot cross a
filesystem boundary.

---

## Failure handling

`CommitOptions::failure` injects a failure deliberately, so the behaviour
can be tested rather than asserted:

| Injection point | What happens | Result |
|---|---|---|
| `FailurePoint::AfterBytes(n)` | the write fails after `n` bytes | the original is untouched; the partial temporary file is removed |
| `FailurePoint::BeforeRename` | the temporary file is complete but the replacement is refused | the original is untouched; the temporary file is removed |

Both are exercised by the benchmark and by the unit tests. In both cases
the original file is byte-identical to what it was, it still parses, it
still passes its integrity check, and no temporary file is left behind.

---

## Deliberate limitations

These are choices, not oversights, and each one is a task for a later
phase:

1. **No graphical editor.** The engine is a library and a benchmark. A GUI
   is P6/P7 work.
2. **The grid never changes on save.** The engine preserves the tile grid
   exactly, which is what makes byte-level payload reuse possible. A save
   that re-tiled the image would change every payload's offset.
3. **The history is in memory only.** It does not survive a process
   restart.
4. **Temporary files left by a killed process are not cleaned up.** The
   engine removes its own temporary file on a failure it observed; it does
   not sweep the directory.
5. **There is no file locking.** Two editors over the same file will not
   see each other's changes, and the second save wins.
6. **There is no undo of a commit.** Undo and redo operate on the session's
   tiles; a committed file is a new baseline. The engine enforces this: a
   successful commit clears both history stacks, so an undo cannot reach
   across the commit.

---

## Evidence

* `docs/P4_EDITING_ENGINE_REPORT.md` — the twelve questions, the
  measurements, and the test inventory.
* `benchmarks/p4_editing_results.json` — the eight benchmark scenarios.
* `docs/P4_MEMORY_VALIDATION.md` — the 12-case memory matrix, up to
  16384×16384 RGBA16.
* `docs/P4_COMPATIBILITY_REPORT.md` — the format's compatibility, the
  fuzzing evidence and the CI gates.
