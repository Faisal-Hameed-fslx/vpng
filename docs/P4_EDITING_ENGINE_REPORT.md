# P4 — Editing engine report

**Gate:** P4 (editing engine) — **PASS**
**Date:** 2026-11-16
**Toolchain:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `x86_64-pc-windows-gnu`, release profile
**Implementation:** `crates/vpng-editing/` (7 modules, 51 unit tests)
**Raw data:** `benchmarks/p4_editing_results.json`
**Memory evidence:** `benchmarks/p4_memory_results.json` — see `docs/P4_MEMORY_VALIDATION.md`
**Compatibility evidence:** `docs/P4_COMPATIBILITY_REPORT.md`

> This report covers **one half** of the P3.5 + P4 milestone. The compression
> research is reported separately in `docs/P3_5_COMPRESSION_RESEARCH_REPORT.md`.
> The two gates are independent: this engine does not use any experimental
> codec, and no experimental codec depends on this engine.

---

## 1. What was built

A new workspace crate, `vpng-editing`, with five parts:

| Module | What it does |
|---|---|
| `source.rs` | The `TileSource` trait, with `DocumentSource` (a file already in memory) and `FileSource` (a file on disk, read one payload at a time). |
| `cache.rs` | `TileCache`: a strict LRU of decoded tiles with a **byte** budget, not a tile count. |
| `history.rs` | `History`: tile-level undo/redo with a byte budget. |
| `engine.rs` | `Editor<S>`: region reads and writes, dirty-tile tracking, transactions, undo/redo. `FileEditor` is the disk-backed session. |
| `commit.rs` | The copy-on-write save: encode the dirty tiles, copy every other payload verbatim, write a temporary file, replace the original atomically. |

The engine is **generic over its source** (`Editor<S: TileSource>`), so the
same editing code serves a 4 KB file that is already in memory and a 16K
file that is far too large to hold in memory.

### The public surface

```rust
let mut editor = FileEditor::open(path, &limits)?
    .with_cache_bytes(16 * 1024 * 1024)?
    .with_history_bytes(32 * 1024 * 1024)?;

editor.editor().begin()?;
let region = editor.editor().read_region(Rect::new(10, 10, 32, 32)?)?;
editor.editor().write_region(Rect::new(10, 10, 32, 32)?, region.data())?;
editor.editor().commit(path, &CommitOptions::default())?;
```

`Editor::parse(&bytes, &limits)` is the borrowed-memory equivalent, and
`EditableDocument<'a>` is its type alias.

---

## 2. How the guarantees are implemented

### No full-image buffer

`Editor::read_region` allocates **exactly** the requested rectangle
(`CanonicalImage::new_zeroed(rect.width, rect.height, …)`) and then decodes
only the tiles the rectangle intersects. `Editor::write_region` decodes only
those same tiles and patches them in place. There is no code path in the
engine that allocates a buffer sized by the image.

The unit test `the_engine_never_holds_a_full_image_buffer` asserts this
directly: it instruments the allocator and checks that a 64×64 edit on a
1024×1024 image never allocates anything close to the image's size.

### Untouched tiles keep their bytes

`write_document` walks the tiles in order. A tile that is not in the dirty
map contributes its **original compressed payload**, read from the source and
written out verbatim; its directory entry keeps the original `codec_id`,
`predictor_id`, `flags` and `payload_checksum`. Only a dirty tile is
re-encoded, and it is encoded through `vpng_encoder::select_payload` — the
encoder's own selection function — so a re-encoded tile is exactly the tile
the encoder would have produced.

A **rewritten** tile's directory entry describes the payload that was just
written, not the one it replaced: its `predictor_id` and `flags` are `0`.
The previous tile's values describe an encoding that is no longer in the
file, and carrying them over would leave the entry describing the wrong
thing. The unit test
`a_rewritten_tile_gets_fresh_metadata_and_an_untouched_tile_keeps_its_own`
asserts both halves of that rule: the rewritten tile's metadata is reset
**and** the untouched tile keeps its own metadata.

A payload's *offset* can move, because the container lays the payloads out
contiguously after the directory and a rewritten tile may be a different
size. The bytes themselves do not change.

### The commit boundary

A successful commit establishes a **new editing baseline**. Undo and redo
never cross it:

* the undo history and the redo history are both cleared;
* the edit counter restarts at zero;
* the open transaction is closed — an `abort` after a commit is an error;
* the dirty set is reconciled with the committed baseline.

A commit that **fails** changes nothing: the dirty tiles, the undo and redo
history, the open transaction, and the edit counter all survive.

The dirty-set reconciliation depends on whether the source can re-read the
file that was just written. `FileSource` can: when the commit replaced the
source's own file it re-opens the path and re-reads the header, the
directory, and the grid, so the committed file becomes the baseline and the
dirty set is emptied. `DocumentSource` cannot — its bytes are borrowed and
immutable — so it keeps the pre-commit baseline and the dirty set is kept,
because the source still stores the old pixels. `CommitReport::baseline_advanced`
reports which happened.

### Atomic copy-on-write

A save never writes to the original file. It:

1. encodes the dirty tiles in memory;
2. computes the new directory and the new header;
3. writes a **temporary file in the same directory** as the target
   (`.{name}.{pid}.{counter}.vpng-edit.tmp`), so the replacement is a rename
   within one volume;
4. `flush()`es and `sync_all()`s it;
5. `std::fs::rename`s it over the target.

On Windows `std::fs::rename` calls `MoveFileExW` with
`MOVEFILE_REPLACE_EXISTING`, which replaces the target in one step. A reader
sees either the old file or the new one, never a half-written file.

### Failure handling

`CommitOptions::failure` injects a failure deliberately:

* `FailurePoint::AfterBytes(n)` — the write fails after `n` bytes;
* `FailurePoint::BeforeRename` — the temporary file is complete but the
  replacement is refused.

In both cases the temporary file is removed and the original file is left
exactly as it was.

---

## 3. Measurements

### 3.0 Terminology: four different tile counts

The report quotes four tile counts, and they are **not** the same number.
They are never interchangeable, and the raw data carries them under
separate keys.

| Term | What it counts | Raw key |
|---|---|---|
| **decoded** | tiles whose compressed payload was read and turned back into pixels. A tile is decoded when the engine needs its pixels — to read a region, or to patch part of the tile. | `cache.decoded_tiles`, `tiles.decoded` |
| **modified** | tiles whose **pixels** changed — the tiles the editor's dirty set holds. | `dirty.tile_count`, `tiles.modified` |
| **rewritten** | tiles whose **encoded payload** was replaced during a commit, by re-encoding the modified pixels. | `report.tiles_rewritten`, `tiles.rewritten` |
| **preserved** | tiles whose original compressed payload was copied into the new file **byte for byte**. A preserved tile was neither decoded nor re-encoded during the commit. | `report.tiles_copied`, `tiles.preserved` |

The counts can differ, and this report says which is which:

* A tile can be **decoded** and **modified** but never **rewritten** — for
  example when the edit is later undone, or when the commit fails.
* A tile can be **decoded** and **rewritten** without being *modified* — for
  example when a commit re-encodes a tile the engine held in its dirty set.
* The tiles a commit **preserves** are exactly the tiles it does not
  rewrite: `tiles_rewritten + tiles_copied == tile_count`.

**Decoding is not rewriting.** The `localized-edit` scenario decodes one
tile and modifies one tile, but it performs **no commit**, so it rewrites
nothing and preserves nothing. The `payload-reuse` scenario commits, and
there one tile is rewritten while 63 are preserved — but the commit itself
decodes **no** tile, because the pixels it re-encodes are already in the
dirty set. The single number "1 tile" is reported above as *decoded* in one
scenario and as *rewritten* in the other; the two are never conflated.

### 3.1 The benchmark scenario set

`benchmarks/p4_editing_results.json`, produced by
`vpng-editing-bench` on a 2048×2048 RGBA8 image with a 256-pixel tile grid
(64 tiles, 16 MiB of pixels, a 4,309,019-byte file):

| Scenario | Result |
|---|---|
| `localized-edit` | **1 tile decoded**, **1 tile modified** (262,144 bytes = 1.5625% of the image); no commit, so 0 rewritten and 0 preserved |
| `payload-reuse` | **1 tile rewritten**, **63 tiles preserved** (copied verbatim); 63 identical payloads, 1 changed |
| `save-reopen-digest` | expected digest `8edcce25…` == actual digest `8edcce25…` |
| `abort-preserves-original` | file bytes unchanged, still parses, integrity check passes, editor clean |
| `failure-injection` | both failure points: commit failed, original preserved, original still parses, **no temporary file left** |
| `undo-redo` | undo restores the pixels exactly; redo restores them exactly; 0 dirty tiles after undo, 1 after redo |
| `cache-budget` | 524,288-byte budget: 4 misses, 2 evictions, peak 524,288 bytes — **at** the budget, never over |
| `history-budget` | 1,048,576-byte budget, 12 edits: 3 steps retained, 9 dropped, peak 786,624 bytes — **under** the budget |

### 3.2 Locality

A 64×64 edit on a 2048×2048 image:

| Quantity | Value |
|---|---:|
| Image pixels | 16,777,216 bytes |
| Tiles in the image | 64 |
| Tiles **decoded** | **1** |
| Tiles **modified** (dirty) | **1** |
| Tiles **rewritten** | 0 (this scenario does not commit) |
| Tiles **preserved** | 0 (this scenario does not commit) |
| Dirty bytes | 262,144 (1.5625% of the image) |
| Region read time | 10.3 ms |
| Region write time | 0.10 ms |
| Cache hits / misses | 1 / 1 |
| Cache peak | 262,144 bytes (one tile) |

The read cost is dominated by the first decode of the tile (10.1 ms of the
10.3 ms); the write is a patch of already-decoded pixels, so it decodes
nothing further.

### 3.3 Payload reuse

The commit of the 64×64 edit:

| Quantity | Value |
|---|---:|
| Tiles **rewritten** | 1 |
| Tiles **preserved** (copied verbatim) | 63 |
| Payload bytes copied | 4,239,148 |
| Payload bytes re-encoded | 67,312 |
| Identical payloads before/after | 63 |
| Changed payloads | 1 |

The commit re-encodes the one modified tile from the pixels the dirty set
already holds, so the commit decodes no tile at all. The other 63 payloads
are copied byte for byte and are **preserved**.

### 3.4 Exactness

| Check | Value |
|---|---|
| Digest before the patch | `0cfcb51dd8790bec712c4ea30aa83038d67cec02ae702aea888a6bed26dad51f` |
| Digest expected after the patch | `e384aa6979c8327a70b06f5fd19b970ff390b3665f1d92d2fea8e36243d062be` |
| Digest of the file after the save and reopen | `e384aa6979c8327a70b06f5fd19b970ff390b3665f1d92d2fea8e36243d062be` |
| Independent save/reopen digest | `8edcce252bac99f3019ae40ae23b14c3cce921a48f80ecaa2e0508147170cfe5` (expected == actual) |

The digest is the canonical SHA-256 digest defined in
`docs/VPNG_SPEC_0_1.md` §7, so it can be compared with a digest computed by
any other implementation.

---

## 4. The twelve questions

**Q1 — Does a single-tile edit avoid decoding the full image?**
**Yes.** The benchmark decodes **1** tile out of 64 for a 64×64 edit, and
**1** tile out of 1,024 for the 16K RGBA16 case in
`benchmarks/p4_memory_results.json` — where the image's canonical size is
2,147,483,648 bytes and the edit's resident-set growth is 11.7 MB. The unit
test `a_region_read_does_not_decode_unrelated_tiles` asserts the same
property at the API level.

"Decoded" is not "rewritten": in the `localized-edit` scenario the single
decoded tile is also the single **modified** tile, but the scenario performs
no commit, so no tile is rewritten. The commit is measured separately, in
the `payload-reuse` scenario, where **1** tile is rewritten and **63** are
preserved. See §3.0.

**Q2 — Are unmodified payloads reused byte for byte?**
**Yes.** 63 of 64 payloads are byte-identical before and after the save, and
the 64th is the edited tile. The copy is a byte-level copy of the original
payload, not a decode-and-re-encode, so the bytes cannot drift. The unit
test `a_commit_copies_the_untouched_payloads_byte_for_byte` asserts it
directly.

**Q3 — Are BEGIN / COMMIT / ABORT supported?**
**Yes.** `Editor::begin` opens a transaction, remembering the dirty set and
the undo depth. `Editor::abort` restores that state and cuts the history
back to the same point. `Editor::commit` writes the file and closes the
transaction. A second `begin` is rejected with an error, and an `abort`
without an open transaction is rejected too
(`a_second_begin_is_rejected`, `an_abort_without_a_transaction_is_rejected`).

A successful commit also establishes a **new baseline**: the undo and redo
history are cleared, the edit counter restarts, the transaction state is
cleared, and the dirty set is reconciled with the committed file. A failed
commit changes nothing, so the dirty set, the history, and the transaction
all survive. `an_abort_after_a_commit_reports_that_no_transaction_is_open`
and `a_failed_commit_keeps_the_undo_history` assert the two halves.

**Q4 — Is the save an atomic copy-on-write?**
**Yes.** The new file is written to a temporary file in the same directory
and then renamed over the original in one step. The original is never
opened for writing.

**Q5 — Does the original survive a failure?**
**Yes.** Two failure points are injected. After a failure part-way through
the write (`AfterBytes`) and after a failure immediately before the rename
(`BeforeRename`), the original file is byte-identical to what it was, it
still parses, it still passes its integrity check, and **no temporary file
is left behind**. The unit tests
`a_failed_write_leaves_the_original_file_untouched` and
`a_failure_before_the_rename_leaves_the_original_untouched` assert the same
thing.

**Q6 — Do undo and redo restore the pixels exactly?**
**Yes.** `undo_restores_pixels: true` and `redo_restores_pixels: true` in
the benchmark; the unit test `undo_and_redo_restore_the_pixels_exactly`
compares the region byte for byte. An undo that returns a tile to the
pixels the file stores marks it **clean** again, so it is not rewritten by
the next save — measured as `dirty_after_undo: 0`. A new edit invalidates
the redo chain (`a_new_edit_invalidates_the_redo_chain`).

Undo and redo **never cross a commit boundary**. A successful commit is the
new baseline, so the steps that reached it are not undoable steps of the
sequence that starts after it. `an_undo_cannot_cross_a_commit`,
`an_edit_after_a_commit_starts_a_new_history_sequence`, and
`a_redo_after_a_commit_undo_restores_the_new_edit` assert the rule and its
two directions.

**Q7 — Is the history's memory configurable?**
**Yes.** `with_history_bytes(budget)` sets it; the default is 64 MiB. The
history is tile-level: one entry holds the previous pixels of every tile a
write touched. When the budget is exceeded, the oldest entries are dropped
and counted in `dropped_entries`. A step too large for the whole budget is
dropped at once rather than being retained over the bound. The peak is
recorded **after** trimming, so the reported peak is the high-water mark of
the bytes the history actually retains and can never exceed the budget.

**Q8 — Is the cache bounded, and does it evict?**
**Yes.** `with_cache_bytes(budget)` sets it; the default is 32 MiB. The
cache is a strict LRU: a hit moves the tile to the most-recently-used end,
an insert evicts from the least-recently-used end until the new tile fits.
A tile larger than the whole budget is **not** cached: it is counted as
`rejected_tiles` and handed back to the caller, so the budget stays honest.
The benchmark's cache scenario shows 4 decodes, 2 evictions and a peak of
exactly 524,288 bytes against a 524,288-byte budget.

**Q9 — Does the format change?**
**No.** An edited file is an ordinary VPNG file: the same 64-byte header,
the same 40-byte directory entries, the same payloads, the same 4-byte
directory CRC trailer. The engine adds no chunk, no field and no version
number. The header keeps every field of the original except the declared
size and the directory offset, both of which the container defines.

A rewritten tile's directory entry carries `predictor_id: 0` and `flags: 0`,
which is what the encoder writes for every tile it produces. The values
describe the payload that is in the file, and no directory field is inherited
from a payload that has been replaced.

**Q10 — Are the golden files unchanged?**
**Yes.** The five golden fixtures are byte-identical (17,092 / 324,668 /
780 / 396 / 2,348 bytes) and the `vpng-testkit` golden test passes. The
editing engine touches no encoder or decoder code path that a golden file
depends on. The only additions to shared crates are two *pure additions*:
`vpng_decoder::derive_grid` and `vpng_encoder::select_payload` were made
public so the editor can reuse them; their behaviour is unchanged.

**Q11 — What are the measured timings, and what are they not?**
**Measured**, on this host, release profile, 2048×2048 RGBA8, 256-pixel
tiles: region read 10.3 ms (of which 10.1 ms is the tile's first decode),
region write 0.10 ms, commit 46.0 ms for a 64-tile file (4,239,148 payload
bytes copied, 67,312 re-encoded). The 16K cases are in the memory report.
**Not measured, and not claimed:** any comparison against another editor,
any throughput or latency figure for a different machine, any concurrency
figure, and any claim that the engine is "fast" in the abstract. The
timings are single-run on one host and are reported as measurements, not as
performance claims.

**Q12 — What is explicitly not claimed?**
* **No graphical editor.** There is no GUI, no canvas, no tool palette, no
  mouse handling. The engine is a library and a benchmark.
* **No performance improvement is claimed.** The engine's timings are
  reported; they are not compared against a baseline, because there is no
  baseline editor to compare against.
* **No claim that the history is unlimited.** It is bounded by a byte
  budget, and a long session silently drops the oldest steps — the count is
  reported in `dropped_entries`.
* **No claim that the engine is crash-safe in the face of power loss.** It
  is safe against a *failed write* and a *refused rename*, both of which are
  injected and tested. A power cut between the temporary file's `sync_all`
  and the rename leaves the original intact and a temporary file behind; the
  engine does not clean up temporary files left by a previous process.
* **No claim about multi-process editing.** Two editors over the same file
  will not see each other's changes; there is no locking.
* **No format change, and no migration.**

---

## 5. What is deliberately left for later phases

1. A graphical editor (P6/P7) — the engine is the foundation it needs.
2. Persistent history across sessions; the history is in memory only.
3. Temporary-file cleanup for a file left behind by a killed process.
4. File locking, so two processes cannot edit one file at once.
5. A tile-geometry change on save. The engine preserves the grid exactly; a
   future phase may want to re-tile on save, which would change every
   payload's offset and defeat byte-level reuse.

---

## 6. Reproducing the evidence

```powershell
cargo build --release --workspace
cargo test -p vpng-editing                              # 51 unit tests
.\target\release\vpng-editing-bench.exe `
    --work-dir target/vpng-editing-bench `
    --output benchmarks/p4_editing_results.json
```

The 16K memory matrix is reproduced by `python benchmarks/memory/run_memory.py`
and is reported in `docs/P4_MEMORY_VALIDATION.md`.

---

## 7. The test inventory for this gate

| Area | Tests |
|---|---|
| Region reads and writes | `a_region_read_returns_exactly_the_region`, `a_region_read_does_not_decode_unrelated_tiles`, `a_region_write_changes_only_the_tiles_it_intersects`, `a_region_write_rejects_a_wrong_sized_buffer`, `a_region_write_outside_the_image_is_rejected` |
| Undo / redo | `undo_and_redo_restore_the_pixels_exactly`, `undo_makes_a_tile_clean_again`, `undo_with_no_history_reports_that_there_is_nothing_to_do`, `a_new_edit_invalidates_the_redo_chain` |
| Transactions | `an_abort_restores_the_document`, `a_second_begin_is_rejected`, `an_abort_without_a_transaction_is_rejected`, `an_abort_after_a_commit_reports_that_no_transaction_is_open` |
| Commit | `a_commit_writes_a_file_that_decodes_to_the_edited_pixels`, `a_commit_copies_the_untouched_payloads_byte_for_byte`, `a_commit_with_nothing_to_change_still_writes_a_valid_file`, `a_forced_codec_is_used_for_the_rewritten_tiles`, `a_committed_file_passes_the_integrity_check` |
| Commit boundary | `an_undo_cannot_cross_a_commit`, `an_edit_after_a_commit_starts_a_new_history_sequence`, `a_redo_after_a_commit_undo_restores_the_new_edit`, `a_failed_commit_keeps_the_undo_history` |
| Tile metadata | `a_rewritten_tile_gets_fresh_metadata_and_an_untouched_tile_keeps_its_own` |
| Failure | `a_failed_write_leaves_the_original_file_untouched`, `a_failure_before_the_rename_leaves_the_original_untouched` |
| The file-backed editor | `the_file_backed_editor_reads_and_writes_the_same_way`, `the_file_backed_editor_commits_to_a_different_path` |
| Bounded memory | `the_cache_evicts_when_the_budget_is_exceeded`, `the_engine_never_holds_a_full_image_buffer` |
| Compatibility | `the_document_parse_api_is_unchanged` |

51 tests in total, all passing.
