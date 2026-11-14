# Threat model

**Scope:** reading, encoding, and editing `.vpng` files. All file bytes are
treated as attacker-controlled.

## 1. Assets

* Exact pixel data (the whole point of the format).
* Availability of the decoder and editor to the user.
* Process memory and CPU.
* Integrity of the only valid copy of a file during editing.

## 2. Adversaries

* A corrupt or malicious `.vpng` file (D1).
* A malicious `.png` used as encoder input (D2).
* A partially written or concurrently modified file (D3).
* Ordinary bugs in this implementation (D4).

## 3. Threats and mitigations

| # | Threat | Mitigation | Where |
|---|--------|-----------|-------|
| T1 | Integer overflow in size math | all arithmetic via `checked_*` helpers; no unchecked multiply of untrusted values | `vpng-core::checked_*` |
| T2 | Huge declared dimensions → OOM | `Limits::validate_dimensions` before allocation | `vpng-core::limits` |
| T3 | Huge tile count → OOM | `validate_tile_count`, directory size checked before slicing | decoder, container |
| T4 | Offsets/lengths outside the file | `checked_range` on every payload and the directory | decoder |
| T5 | Overlapping or out-of-order tiles | exact-coverage and canonical-order validation; payload overlap rejection | decoder, container |
| T6 | Decompression bomb | per-tile expansion-ratio cap and decompressed-size cap, checked *before* allocation | `Limits::validate_tile_payload` |
| T7 | Corrupted payload silently accepted | per-tile CRC32C compared before decode; `vpng check` validates *all* payloads with no reference image (`vpng info` reads only header + directory and says so) | decoder, CLI |
| T8 | Header tampered to change semantics | CRC32C over the whole header except its own field | container |
| T9 | Unknown feature silently ignored | unknown feature-flag bits and unknown codec ids are hard errors | container, codecs |
| T10 | Truncated or trailing bytes | `declared_file_size` must equal the real length | decoder |
| T11 | Panic on malformed input | no indexing without prior length checks; no `unwrap` on untrusted data; `malformed.rs` + `mutation_sweep.rs` sweeps | all crates |
| T12 | Recursion / stack exhaustion | parsing is flat and iterative; no recursive descent | container, decoder |
| T13 | Metadata code execution | v0.1 has no metadata chunks at all | spec section 1.1 |
| T14 | Network access during decode | none exists; no network code in any crate | all crates |
| T15 | Interrupted edit destroys the file | not implemented yet; P4 mandates atomic replacement | roadmap P4 |
| T16 | Silent weakening of losslessness | exact-bytes tests, canonical digest checks, `verify` command | tests, CLI |

## 4. Out of scope for v0.1

* Side channels (timing, cache) — no secrets are processed.
* Untrusted *encoder* input beyond dimension/format validation; a malformed PNG
  is the `png` crate's problem and surfaces as a typed error.
* Denial of service via huge but legal files: bounded by `Limits`, which the
  caller can lower.

## 5. Explicit non-goals

CRC32C is not a security primitive and is never used to authenticate. SHA-256 is
used only for identity comparison, not as a MAC; file authenticity is out of
scope and must be provided by the deployment (e.g. signed transport).

## 6. Verification plan

* `malformed.rs`: targeted mutations of every header field, the directory, its
  CRC trailer, and payload bytes; truncation sweep; single-byte-flip sweep;
  unknown codec id. Each must yield a typed error and must not panic.
* `mutation_sweep.rs`: every truncation and every single-byte substitution
  (three values per offset) on one small Raw, Palette, and Run-length file, plus
  2,000 seeded multi-byte corruptions per file. This is a deterministic sweep,
  **not** coverage-guided fuzzing; it complements `malformed.rs` and is bounded
  by construction.
* Next phase: `cargo-fuzz` targets for the header parser, directory parser, and
  each codec, plus a `cargo test` run under the address sanitizer where the
  platform supports it.
* `vpng check`: CRC32C over every tile payload without pixel decode; exits 0 when
  intact and 4 on the first mismatch. `vpng info` intentionally reads only the
  header and directory, prints that payloads were not verified, and is not
  claimed to be an integrity check.
