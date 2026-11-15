# ADR-0011: Predictor + entropy codec

**Status:** accepted
**Date:** 2026-11-15
**Question (roadmap §6.4):** Which spatial predictors does codec id 3 use, how
are the residuals represented, how does the entropy coder work, and how is the
predictor chosen per tile?

## Decision

A tile is stored as a **per-row predictor plan** followed by an
**adaptive arithmetic-coded list of signed residuals**.

### Payload layout

```text
offset 0   u8    version (must be 1)
...        ..    tile width in pixels, unsigned LEB128
...        ..    one predictor-mode byte per tile row
...        ..    residual count, unsigned LEB128
...        ..    entropy-coded residual data
```

The width is in the payload so a tile is self-describing: the decoder rebuilds
the exact sample grid from the payload alone. The residual count equals the
tile's pixel count times its channel count, and the decoder rejects any other
count.

### Predictors

Seven predictors are defined, all on **whole samples** (a 16-bit channel is one
value, never two bytes):

| code | predictor  | prediction                                 |
|------|------------|--------------------------------------------|
| 0    | none       | `0`                                        |
| 1    | left       | the previous pixel in the row, same channel |
| 2    | up         | the pixel above, same channel              |
| 3    | upper-left | the pixel above-left, same channel         |
| 4    | average    | `(left + up) / 2`, rounded down            |
| 5    | Paeth      | the Paeth choice of left/up/upper-left     |
| 6    | gradient   | the LOCO-I choice of left/up               |

A neighbour outside the tile — left of the first column, above the first row,
upper-left of either — is `0`. Prediction therefore **resets at every tile
boundary**, and no tile ever depends on another tile.

### Residuals

For a sample of `B` bytes, the residual is `(sample - prediction) mod 2^(8B)`
folded to the signed range `[-2^(8B-1), 2^(8B-1) - 1]`. The fold is exact:
`prediction + residual mod 2^(8B)` reproduces the sample bit for bit. Every
predictor is an integer function of whole samples, so the transform is exactly
reversible and uses **no floating point**.

### Entropy coder

An adaptive binary arithmetic coder with 32-bit `low`/`range`, E1/E2/E3
renormalization and a byte flush. Each context holds an 11-bit probability that
moves 1/32 of the way toward the symbol just seen, so no frequency table is
shipped and no symbol is ever priced at certainty.

A residual is coded as **one sign bit plus one decision per magnitude plane**,
most significant first. Before the first 1-bit, each plane uses its own
"not started yet" context; after it, each plane uses its own plane context.
Giving every plane its own leading context is what makes a small residual
nearly free: the planes above it are consistently zero and collapse to a
near-deterministic decision.

The coder is a **terminator** on the residual count: exactly that many
residuals are decoded and the decoder never has to detect end-of-stream.

### Predictor selection

The encoder chooses a predictor **per row**. It computes the row's residuals
under all seven predictors and measures each candidate with the live coder's
probability model, keeping the one with the lowest cost. The measurement is the
coder's own information content — the sum of `-log2(p)` over the symbols — so
it accounts for how the adaptive model has settled by that point in the tile.
Ties go to the lowest predictor code, so the output is deterministic.

## Rationale

* **Whole samples, not bytes.** A 16-bit channel must never be split across
  two residuals; the same "count in whole pixels" rule the palette and
  run-length codecs follow keeps every format exact.
* **Predictors reset at tile boundaries.** Random access stays exact and a tile
  can be decoded without its neighbours. The roadmap requires this unless the
  format explicitly encodes the dependency, and encoding it would cost more
  than it saves.
* **A row, not a tile, carries the predictor.** A photograph's rows differ: some
  are horizontally smooth, some vertically smooth. One mode byte per row costs
  one byte per row and buys a per-row choice.
* **One leading context per plane, not one shared context.** With a shared
  context the coder must price the first 1-bit of a residual as if it were as
  likely as any leading zero, which costs several bits on *every* small
  residual. With a context per plane, a residual whose magnitude always sits in
  the same plane collapses to a near-deterministic decision. Measured on a
  64×64 gray8 tile of a constant-step ramp, this changed the payload from
  2400 bytes to 396 bytes.
* **The selector measures, it does not guess.** An order-0 entropy estimate
  cannot see that the adaptive model has already adapted, so it mis-ranks
  predictors: it punishes a predictor for producing many tiny residuals even
  though the coder will code those nearly for free. Measuring with the live
  model avoids that, and a probe that skips interval arithmetic and byte
  emission keeps it affordable.
* **Fixed-point costs, no floating point.** The cost measure uses a baked-in
  `log2` table, so the encoder's choice is identical on every host.

## Consequences

* The codec is **exact by construction**: `prediction + residual` reconstructs
  the sample under the same mask, and the entropy coder is a terminator on a
  declared count. A corrupt payload is rejected, never best-effort decoded.
* The payload is **not canonical**: any well-formed stream decodes to the same
  pixels, and the encoder's particular choice of predictors is pinned by golden
  vectors rather than by the format.
* Encoding costs roughly **two to three times** what the same coder would cost
  without the per-row selection, because every candidate is measured before one
  is kept. This is a deliberate trade for a smaller payload.
* The selection is **greedy per row**: the choice for one row is made with the
  model state that the previous rows produced, and a different earlier choice
  could in principle have led to a better overall result.
* A future **context-adaptive entropy codec** (id 6) may model the residual
  magnitude directly and remove the switching cost the current per-plane
  contexts pay when a row changes magnitude class.
* Codec id 3 is pinned to this layout; a different predictor or entropy
  representation belongs under a different codec id.
