# ADR-0001: Header checksum coverage

**Status:** accepted
**Date:** 2026-11-12
**Question (roadmap §18.1):** What exactly does the 64-byte header checksum cover?

## Decision

The 4-byte field at offset 52 is CRC32C over the concatenation of bytes `0..52`
and `56..64`. The checksum field itself, bytes `52..56`, is excluded.

## Rationale

* Excluding the checksum field from its own input makes the value
  self-consistent and avoids a fixed-point search.
* Covering the reserved field means a future reader cannot be tricked by a
  non-zero reserved value that a writer set without recomputing.
* This mirrors widely understood container practice and is trivial to implement
  with a streaming CRC.

## Consequences

* Any header mutation outside the checksum field is detected.
* The algorithm is CRC32C (see ADR-0007), not CRC32/Adler.
* Writers must zero bytes `52..56` before computing and then write the result.
