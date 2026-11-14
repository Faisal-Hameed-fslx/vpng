# ADR-0010: Run-length token format

**Status:** accepted
**Date:** 2026-11-14
**Question (roadmap §6.3):** How does the Run-length codec (id 2) represent
repeated symbols, what is the maximum run length, and how are literal and repeat
markers kept unambiguous?

## Decision

Runs are measured in **whole pixels (symbols)**, never in bytes, so a run always
repeats one exact pixel tuple; no channel or alpha byte can drift out of step.
The payload is a sequence of self-delimiting tokens. Each token starts with a
1-byte control:

```text
control 0x00..=0x7F   literal run: count = control + 1 (1..=128)
                      followed by count symbols, verbatim
control 0x81..=0xFF   repeat run:  count = (control & 0x7F) + 1 (2..=128)
                      followed by one symbol, repeated count times
control 0x80          malformed: a repeat run of length 1
```

* **Maximum run length is [`MAX_RUN`] = 128** symbols per token; a longer run is
  split across several tokens.
* The stream is **self-delimiting** because every token carries its own control
  byte and exact count. Symbols are read only where a token says to read them, so
  a symbol can never be mistaken for a control and **no escaping is needed**.
* A repeat token whose control is `0x80` (a run of one symbol) is malformed and
  rejected; a repeat of one is always better expressed as a literal.

The **encoder** is deterministic: it emits one token (or a split sequence of
tokens) for each maximal run of length ≥ 2, and groups adjacent single-symbol
runs, up to 128, into literal tokens. When a run of length `L` is split it is
divided so that no chunk has length 1 (for example 129 → 127 + 2).

The **decoder** accepts any well-formed stream, not only the encoder's canonical
one. It rejects: a length-1 repeat, a truncated literal or repeat, a run that
would produce more symbols than the tile needs, and a stream whose symbol total
does not exactly equal the tile's pixel count.

The codec reports a tile as **not representable** (`UnsupportedFeature`) only for
an empty tile or when the payload would exceed the configured per-tile limit. A
selector then falls back to another codec.

## Rationale

* Counting in pixels rather than bytes is what makes run-length coding safe for
  every v0.1 format, including 16-bit channels and alpha: a run copies an exact
  pixel, so there is no re-synchronization hazard.
* A single control byte with a tag bit is the classic PackBits shape: compact,
  trivially decodable, and impossible to misread.
* Because tokens are self-delimiting, no byte-stuffing or escape sequence exists
  to get wrong, which is the failure mode the roadmap warns about.
* Capping at 128 bounds the decoder's per-token work and keeps the token grammar
  small; long flat regions are still handled in a few bytes each.

## Consequences

* A tile with runs is encoded in a few bytes per run; a tile with no runs expands
  to one control byte plus one symbol per pixel. That expansion is harmless
  because the encoder's cost-based selector only chooses run-length coding when
  the complete payload beats the other candidates, and Raw is always available.
* The format is not canonical: a well-formed stream that differs from the
  encoder's greedy output (for example, two adjacent repeats of one symbol) still
  decodes to the same pixels. Golden vectors pin the encoder's output; the
  decoder's contract is exactness plus malformed-input rejection.
* A future codec may define a different run representation under a different
  codec id; id 2 is pinned to this layout.
