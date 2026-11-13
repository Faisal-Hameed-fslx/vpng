# Decision records

One file per resolved design question. Every roadmap §18 question must end up
here before the relevant code is considered stable. Format: `ADR-NNNN-title.md`
with a status, the question, the decision, the rationale, and the consequences.

| ADR | Question (roadmap §18) | Status |
|-----|------------------------|--------|
| [0001](ADR-0001-header-checksum.md) | header checksum coverage (§18.1) | accepted |
| [0002](ADR-0002-directory-entry.md) | directory entry width and field order (§18.2) | accepted |
| [0003](ADR-0003-directory-authority.md) | directory inline vs chunk-based, single authority (§18.3) | accepted |
| [0004](ADR-0004-codec-ids.md) | codec id table; dictionary scope (§18.7, §18.8) | accepted |
| [0005](ADR-0005-grid-derivation.md) | grid/partition representation (§18.4) | accepted |
| [0006](ADR-0006-size-units.md) | MB vs MiB (§2.5) | accepted |
| [0007](ADR-0007-crc32c.md) | corruption checksum (§18.5) | accepted |
| [0008](ADR-0008-16bit.md) | 16-bit in the first release (§18.15) | accepted |
| [0009](ADR-0009-palette-payload.md) | palette payload layout and canonical index packing (§6.2) | accepted |

### Still open

* §18.6 SHA-256 canonical digest encoding — the definition is in the spec and
  `vpng-container`; a formal ADR will be added when the digest is frozen.
* §18.9 atomic replacement per platform, §18.10 append-only vs rewrite,
  §18.11 metadata policy, §18.12 preview algorithm, §18.13 cache eviction,
  §18.14 parallel scheduling — all belong to later phases (P4–P6).
