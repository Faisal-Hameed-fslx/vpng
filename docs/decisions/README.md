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
| [0010](ADR-0010-rle-tokens.md) | run-length token format, max run, ambiguity (§6.3) | accepted |
| [0011](ADR-0011-predictor-entropy.md) | predictor set, residual folding, entropy coder, per-row selection (§6.4) | accepted |
| [0012](ADR-0012-atomic-replacement.md) | atomic replacement per platform; append-only vs temporary-file rewrite (§18.9, §18.10) | accepted |
| [0013](ADR-0013-cache-eviction.md) | decoded-tile cache eviction policy (§18.13) | accepted |
| [0014](ADR-0014-levels-and-partitions.md) | levels, partitions, the level flags and the preview policy (§18.4, §18.12) | accepted |
| [0015](ADR-0015-promoting-codecs-4-to-6.md) | promoting the dictionary and context-adaptive codecs to ids 4–6, and the v0.3 version rule | accepted |
| [0016](ADR-0016-http-range-source.md) | the byte-source abstraction, the HTTP Range client, and what a non-conforming server gets | accepted |
| [0017](ADR-0017-wasm-abi.md) | the WebAssembly package's shape and its raw `extern "C"` ABI | accepted |

### Still open

* §18.6 SHA-256 canonical digest encoding — the definition is in the spec and
  `vpng-container`; a formal ADR will be added when the digest is frozen.
* §18.11 metadata policy, §18.14 parallel scheduling — all belong to later
  phases (P5–P8).
