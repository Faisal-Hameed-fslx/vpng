# Memory model (design and targets)

## Principles

* Reusable tile buffers, not per-tile allocations.
* A bounded decoded-tile cache with measurable accounting.
* A bounded worker queue for parallel decode.
* Streaming readers/writers.
* A caller-configurable memory budget.
* Explicit decompressed-size limits.
* Never allocate from an untrusted declared length before validating it against
  a configured limit.

## Where the limits are checked in v0.1

* `vpng-core::Limits` carries the ceilings (see `docs/VPNG_SPEC_0_1.md` §8).
* `vpng-decoder` validates the file size, dimensions, tile count, and every tile
  payload before touching pixels.
* `decode_all` is the **only** path that allocates a full-image buffer, and only
  because the caller asked for full decode of a whole image.
* `decode_region` allocates exactly the requested rectangle, never the image.
* `decode_tile` allocates one tile.

## Targets to measure (not yet measured — see `docs/ROADMAP_STATUS.md`)

| Workload | Target |
|----------|--------|
| One 256² RGBA8 tile | 256 KiB decoded pixels |
| One 512² RGBA8 tile | 1 MiB decoded pixels |
| Four 512² tiles | 4 MiB decoded pixels |
| 16K region editing | ≤ 64 MB codec/editor CPU working set |
| Streaming view | ≤ 32 MB decoder working set where practical |

Report both internal allocation accounting and process peak RSS, and explain the
difference. Exclude OS page cache, the display framebuffer, GPU textures, and
memory owned by the calling application; say so in every report.

## Known limits of the current code

* No tile cache exists yet, so repeated tile decodes re-decode.
* No streaming reader yet; the decoder borrows a complete in-memory buffer.
* No parallel decode yet; tiles decode sequentially.
* No worker/queue bounding yet.

These are P3/P6 work items and are listed in `docs/ROADMAP_STATUS.md`.
