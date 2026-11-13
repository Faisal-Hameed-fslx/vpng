# ADR-0006: `MB` is decimal, `MiB` is binary

**Status:** accepted
**Date:** 2026-11-12
**Question (roadmap §2.5):** What does `--max-size 10MB` mean?

## Decision

* `--max-size` accepts an integer with an optional suffix. Bare integers and the
  suffixes `kB`/`MB` are **decimal** powers of 1000. `10MB` = 10,000,000 bytes.
* The suffixes `KiB`/`MiB` are **binary** powers of 1024.
* `--max-size-mib <n>` is an explicit binary alias: `10` = 10,485,760 bytes.

The roadmap fixes the decimal ceiling: "a decimal 10,000,000-byte ceiling unless
the CLI explicitly offers a separate MiB option."

## Rationale

* Matches the roadmap's stated default exactly.
* Prevents the classic off-by-4.9% surprise where "10 MB" silently means
  10,485,760.
* Offers the binary form explicitly, so a user who wants MiB can say so.

## Consequences

* The size accounting in the encoder counts header, directory, metadata (none in
  v0.1), padding (none), and every payload.
* `SizeLimitExceeded` reports the smallest lossless size found so the user can
  see how far over they are.
