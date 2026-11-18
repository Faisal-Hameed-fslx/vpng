# ADR-0013: The decoded-tile cache is a byte-budgeted LRU

**Status:** accepted
**Date:** 2026-11-16
**Question (roadmap §18.13):** What is the cache eviction policy for decoded
tiles?

## Decision

The decoded-tile cache is a **strict least-recently-used cache with a byte
budget**.

* The budget is in **bytes of decoded pixel data**, not in tiles. A tile's
  size depends on the tile dimensions and the pixel format, so a tile count
  would not bound the memory.
* A **hit** moves the tile to the most-recently-used end.
* An **insert** evicts from the least-recently-used end until the new tile
  fits.
* A tile **larger than the whole budget is not cached**. It is counted as
  `rejected_tiles` and handed back to the caller. The budget is a ceiling
  the cache honours even when a single tile does not fit inside it.
* The cache reports hits, misses, evictions, the tiles it decoded, the
  tiles it refused to cache, the bytes it currently holds, its high-water
  mark, and the time spent decoding.

The default budget is 32 MiB; `with_cache_bytes` overrides it. The 16K
validation runs use 16 MiB.

## Rationale

* **The budget has to be in bytes.** The engine is required to hold a
  localized edit of a 16384×16384 image inside 64 MB of working memory. A
  cache bounded in tiles would hold 8× more pixels at 8 bits per channel
  than at 16, and the bound would not be a bound at all.
* **LRU is the right default for tile access.** An editing session moves
  over an image and returns to places it has been. LRU keeps the recently
  touched neighbourhood resident and discards the far-away tiles, which are
  the ones least likely to be touched again.
* **The ceiling must not be exceeded by the tile that breaks it.** A tile
  that is larger than the whole budget is a case where "cache it" and
  "stay under the budget" cannot both hold. Staying under the budget wins,
  and the refusal is *counted* rather than hidden — a caller can see that
  the cache declined to hold the tile.
* **A strict policy is easier to reason about than an approximation.** No
  approximate-LRU, no segmented cache, no scan on eviction. The eviction
  count in the statistics is the exact number of tiles removed.

## Consequences

* The cache lives in `crates/vpng-editing/src/cache.rs` and is exercised by
  `the_cache_evicts_when_the_budget_is_exceeded` and by the benchmark's
  `cache-budget` scenario, which fills a 524,288-byte budget exactly, with
  2 evictions and a peak of exactly 524,288 bytes — never over.
* The high-water mark is the cache's own accounting, so it can be compared
  against the budget directly.
* **The cache is a performance device, not a correctness device.** Every
  tile can be re-decoded from the file at any time; evicting one costs
  time and nothing else. That is why the budget can be lowered without
  changing any result.
* The cache holds **decoded pixel buffers only**. A tile's compressed
  payload is not part of the cache's byte accounting, because the payload
  is not held by the cache.
* The undo history has its own, separate budget and its own accounting; see
  `crates/vpng-editing/src/history.rs`. The two budgets are independent,
  and each is reported separately.
