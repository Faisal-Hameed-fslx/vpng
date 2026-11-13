# Codec design notes

The trait every codec implements lives in `vpng-codecs`:

```rust
pub trait TileCodec: Send + Sync {
    fn codec_id(&self) -> u8;
    fn name(&self) -> &'static str;
    fn encode(&self, input: &[u8], ctx: &EncodeContext, out: &mut Vec<u8>) -> Result<()>;
    fn decode(&self, input: &[u8], expected_len: usize, ctx: &DecodeContext, out: &mut Vec<u8>)
        -> Result<()>;
}
```

Invariants every implementation must hold:

1. **Exact reversibility.** `decode(encode(x)) == x` for every valid tile buffer
   `x`. No quantization, no chroma handling, no perceptual shortcuts.
2. **Independent tiles.** Nothing outside `input` is read. No cross-tile state,
   no hidden dictionaries (ADR-0004, roadmap §18.8).
3. **Bounded output.** `decode` produces exactly `expected_len` bytes and checks
   limits before allocating. It never trusts the stream's declared length.
4. **Deterministic.** Same input, same context, same bytes out. Tie-breaking in
   selection must be defined, never left to hashmap order or timing.
5. **Honest size accounting.** Selection compares complete payload sizes,
   including every codec's own header and any termination bytes — not a
   half-formed entropy estimate.

## v0.1: Raw (id 0)

The payload is the tile's canonical bytes verbatim. It is the correctness
baseline, the golden reference, and the fallback that guarantees a valid encoding
always exists. `compressed_length == uncompressed_length` for every tile.

## v0.1: Palette (id 1) — ADR-0009

A per-tile palette built from the tile's own pixels only. Palette entries are the
exact canonical pixel bytes (so alpha and every channel survive), indices are
packed at the smallest valid width (1, 2, 4 or 8 bits) and read MSB-first, and
entries are ordered by first appearance. The payload is canonical: the index
width must be minimal for the entry count, every index must be in range, the
length must be exact, and the final byte's padding bits must be zero; a decoder
rejects anything else.

A tile with more than 256 distinct pixel values, an empty pixel buffer, or a
payload above the per-tile size limit is reported as **not representable**
(`UnsupportedFeature`), and a selector falls back to another codec. This is not
an input error.

Byte-level golden vectors for 1/2/3/16/256/>256 colors and the canonical-form
rejections live in `vpng-codecs/src/palette.rs`; whole-file golden vectors and
property tests live in `vpng-testkit/tests/palette.rs`.

## Planned codecs and their gates

Each must land with its own golden vectors and property tests before any size
claim is made.

* **Run-length (id 2).** Exact repeated symbols or pixel tuples, bounded run
  length, unambiguous escapes, no pathological expansion.
* **Spatial predictor + entropy (id 3).** Left, up, upper-left, average, Paeth,
  gradient, and an adaptive choice by measured cost. Reversible modular residual
  arithmetic per channel depth; no floating point. Prediction resets at tile
  boundaries.
* **LZ77 dictionary (id 4).** Bounded match length, distance, and memory; all
  references inside the tile; unambiguous literal/match tokens.
* **Predictor + dictionary (id 5).** Predictor residuals fed to the dictionary.
* **Context-adaptive entropy (id 6).** Canonical Huffman, range coding, and CABAC
  are all evaluated on *complete* payload size including model headers.

## Hybrid selection

For each tile: estimate candidate costs, run the promising candidates, include
all per-codec overhead, choose the smallest complete payload, fall back to Raw,
record the choice and measured cost, and break ties deterministically (lowest
codec id wins). Selection never relies only on an entropy estimate when a real
encoded size is available.

This is implemented in `vpng-encoder`'s `select_payload`: for each tile it tries
every id in `vpng_codecs::IMPLEMENTED_CODEC_IDS` in ascending order, skips a codec
that reports the tile as not representable, keeps the smallest payload, and
requires a strictly smaller payload to replace the incumbent (so equal sizes keep
the lower id). The concrete id chosen is written into the tile's directory entry;
the header's `default_codec` is informational. `EncodeOptions::with_codec` forces
a single codec and `with_auto_codec` restores selection.

## Test images exercised per codec

* Palette: 1, 2, 3, 16, 256, and >256 distinct colors, across all eight v0.1
  pixel formats, including forced-codec and auto-selection paths.
* Raw: every format and the incompressible-noise fallback.
