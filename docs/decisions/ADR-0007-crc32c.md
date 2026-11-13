# ADR-0007: CRC32C for all container checksums

**Status:** accepted
**Date:** 2026-11-12
**Question (roadmap §18.5):** Which corruption-detection checksum?

## Decision

CRC32C (Castagnoli, polynomial `0x1EDC6F41`, reflected) for the header checksum,
the directory trailer, and each tile payload checksum. SHA-256 is separate and
used only for the whole-image canonical digest.

## Rationale

* CRC32C has good length-error detection for the sizes involved and a
  well-tested, hardware-accelerated implementation (`crc32c` crate).
* It is a documented standard with a published check value for `"123456789"`
  (`0xE3069283`), which the unit tests pin.
* It is *not* a security primitive, and it is not treated as one.

## Consequences

* The single CRC32C implementation is shared through
  `vpng_container::checksum`, with a `_parts` helper for the non-contiguous
  header coverage from ADR-0001.
* SHA-256 comes from the `sha2` crate; the roadmap forbids implementing it
  ourselves, and we do not.
