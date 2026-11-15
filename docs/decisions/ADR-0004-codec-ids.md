# ADR-0004: Codec identifier table

**Status:** accepted
**Date:** 2026-11-12
**Question (roadmap §18.7, §18.8):** Reconcile the §5.5 id table with the §6
codec roadmap, and decide the dictionary scope.

## Decision

The §5.5 table is canonical and is kept verbatim:

| id | codec |
|---:|-------|
| 0 | Raw |
| 1 | Palette |
| 2 | Run-length |
| 3 | Spatial predictor + entropy |
| 4 | LZ77 dictionary |
| 5 | Predictor + dictionary |
| 6 | Context-adaptive entropy |

Mapping to the §6 letters: A=0, B=1, C=2, D=3 (and 5 when layered over a
dictionary), E=4, F=6. The §6 label "Dictionary" therefore is id 4 and the
§5.5 "Predictor + dictionary" is the distinct id 5.

Dictionary references are **tile-local only** in v1.0 (roadmap §18.8). No codec
may reference bytes outside its own tile.

## Rationale

* The roadmap already assigned ids; the §6 letters are descriptive labels, not
  identifiers. Numbering them differently in code would create exactly the
  ambiguity the roadmap warns against.
* Tile-local dictionaries preserve the core promise: a tile is independently
  decodable, so random access needs no hidden state.

## Consequences

* v0.1 implements ids 0 (Raw), 1 (Palette), 2 (Run-length), and 3 (Spatial
  predictor + entropy). Ids 4–6 are reserved and decoding one fails with
  `UnsupportedFeature`; the decoder never guesses. The payload layout of id 1 is
  fixed by ADR-0009, of id 2 by ADR-0010, and of id 3 by ADR-0011.
* Adding a codec later does not renumber anything.
