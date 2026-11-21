# ADR-0014: Levels, partitions and the preview pyramid

**Status:** accepted
**Date:** 2026-11-21
**Supersedes:** ADR-0005 (grid derivation) for v0.2 files. ADR-0005 remains the
rule for v0.1 files, whose single level is a uniform grid.
**Question:** The header has no field for the nominal tile size, and the 8
reserved bytes must remain zero. How does a decoder recover the geometry of a
file that has a preview pyramid, or tiles of different sizes?

## Decision

Replace the "one uniform grid" model with a **levels and partitions** model,
and carry the level index in the directory entry's `flags` field.

* A **level** is one set of tiles. Level 0 is the original image; every other
  level holds derived preview pixels.
* A level's tiles form a **partition**: rectangles that cover the level's
  rectangle exactly once — no gap, no overlap. The tiles of a partition may
  have **different sizes**.
* A level's rectangle is the bounding box of its tiles, anchored at the
  origin. Level 0's rectangle is the image; a derived level's rectangle is
  strictly smaller in both dimensions than the level below it.
* The level index goes in the **low byte** of the entry's `flags` field; the
  upper byte is reserved and must be zero.
* The entries are sorted by **`(level, y, x)`**. For a v0.1 file — one level —
  this is exactly v0.1's `(y, x)` order.
* The header's `feature_flags` bit 0 (`FEATURE_PYRAMID`) is set exactly when
  the directory holds more than one level, and a reader checks both
  directions.
* The minor version becomes **2**. The 64-byte header and the 40-byte entry
  keep their layouts; only the version byte and the rules above change.

The **preview pixels** are not a format rule. The format says only that level
0 is exact and that a derived level is smaller. The reference encoder's
downsampling — a 2 × 2 box average, integer arithmetic, a half rounding up,
alpha averaged like every other channel — is an **encoder policy**, recorded
here so that a second implementation can reproduce the same file. A decoder
never needs it: it decodes the pixels that are in the file.

The **adaptive planner** is an encoder policy in the same sense. The format
requires only an exact partition; the planner's coarse cut, busyness score,
split order and tile budget are the reference implementation's choices.

## Rationale

* **The layout does not move.** The version bump changes two bytes' meaning
  and nothing else. Every existing reader, every golden fixture and every
  v0.1 file keeps working, and the default encoder path keeps writing v0.1
  bytes, so the fixtures do not move either.
* **There is no second authority.** The level is carried by the entries
  themselves, and the levels are derived from the directory by one shared
  function used by both the in-memory decoder and the file-backed reader. A
  separate level table could disagree with the entries; this cannot.
* **A partition is a weaker, safer requirement than a grid.** A uniform grid
  is a special case of an exact partition, so the v0.1 rule is a subset of
  the v0.2 rule, and one validator covers both.
* **The flags field was already there.** It was reserved and had to be zero
  in v0.1; giving the low byte a meaning in v0.2 costs no bytes and no
  offset changes. A v0.1 reader rejects a v0.2 file on the version anyway.
* **The format does not over-specify.** Pinning the preview algorithm in the
  *format* would make every future preview improvement a format change. The
  encoder policy is pinned in the ADR and in the code, which is where a
  reproducibility promise belongs.

## Consequences

* A v0.1 reader rejects a v0.2 file with `UnsupportedFeature` (the minor
  version is newer). A v0.2 reader reads a v0.1 file as one uniform level.
* A file whose level's tiles have a gap or an overlap is corrupt. The
  validator is a sweep line and is `O(n log n)` in the number of tiles.
* A derived level that is not strictly smaller than the level below it is
  corrupt.
* An encoder that builds a pyramid or an adaptive partition must see the
  whole picture, because both need its content. The default path — a uniform
  grid, no pyramid — does not, and keeps its bounded-memory streaming
  property. This is a real cost and it is stated in the reports.
* An **edit** invalidates the previews: they were computed from pixels the
  edit changed. The editing engine drops the preview levels on commit,
  clears the pyramid flag, and reports how many levels it dropped. The
  untouched tiles keep their payload bytes.
* The **compression ratio** is always "original pixels over whole file", so a
  pyramid cannot make a picture look better than it is.
