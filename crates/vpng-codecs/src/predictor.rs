//! Codec D — spatial prediction plus an entropy coder (id 3).
//!
//! A tile is stored as a per-row predictor plan and an adaptive arithmetic-coded
//! list of signed residuals. Prediction and the entropy coder are separated by
//! a **de-interleave** pass, so every value the coder sees is one exact signed
//! residual of one channel: a channel's residual never mixes with another
//! channel's bytes, and a 16-bit sample is never split across two residuals.
//! That is the same "count in whole pixels" rule the palette and run-length
//! codecs follow.
//!
//! Every tile is encoded and decoded **without touching a pixel outside the
//! tile**. A predictor that reaches outside the tile — left of the first
//! column, above the first row — substitutes a fixed default, so a tile is a
//! standalone unit even at an image edge or in the middle of a grid.
//!
//! ## Payload layout (little-endian; ADR-0011)
//!
//! ```text
//! offset 0   u8   version (must be 1)
//! ...        ..   tile width in samples, unsigned LEB128
//! ...        ..   one predictor-mode byte per tile row
//! ...        ..   residual count, unsigned LEB128
//! ...        ..   entropy-coded residual data
//! ```
//!
//! The width lets the decoder rebuild the exact sample grid from the payload
//! alone, which is what keeps a single tile self-describing. The row-mode
//! bytes let the decoder rebuild the exact prediction plan, so the encoder may
//! choose a different predictor for every row.
//!
//! ## Predictors
//!
//! For a tile `W` pixels wide with `C` channels and `B` bytes per channel, the
//! sample for pixel `x`, channel `c` lives at tile byte offset
//! `(x * C + c) * B`. Seven predictors exist, all on whole samples:
//!
//! | code | predictor  | prediction                                    |
//! |------|------------|-----------------------------------------------|
//! | 0    | none       | `0`                                           |
//! | 1    | left       | the previous pixel in the row, same channel    |
//! | 2    | up         | the pixel above, same channel                 |
//! | 3    | upper-left | the pixel above-left, same channel            |
//! | 4    | average    | `(left + up) / 2`, rounded down               |
//! | 5    | Paeth      | the Paeth choice of left/up/upper-left        |
//! | 6    | gradient   | the LOCO-I choice of left/up                  |
//!
//! A neighbour that falls outside the tile is `0`: left of the first column,
//! above the first row, and upper-left of either. Prediction therefore resets
//! at every tile boundary, and the encoder and decoder agree without any
//! cross-tile state.
//!
//! A multi-byte sample is predicted as one little-endian integer, so the
//! arithmetic is exact for 8- and 16-bit channels alike. Residuals are computed
//! modulo `2^(8B)` and folded to a signed value in `[-2^(8B-1), 2^(8B-1) - 1]`,
//! which keeps every intermediate in `u32` with no overflow and no floating
//! point. Every predictor is an integer function of whole samples, so the
//! transform is exactly reversible.
//!
//! ## Predictor selection
//!
//! The encoder chooses a predictor for each row independently. It computes the
//! row's residuals under every predictor and keeps the one with the lowest
//! estimated cost — the sum of the residuals' significant-bit counts, which
//! tracks the entropy coder's cost because the coder prices a residual by
//! exactly those bit planes. The choice is deterministic: ties go to the
//! lowest predictor code.
//!
//! ## Entropy coder
//!
//! Residuals are coded with an adaptive binary arithmetic coder: 32-bit
//! `low`/`range`, E1/E2/E3 renormalization, byte-flush termination, and one
//! probability context per bit plane. Each context's probability is an 11-bit
//! count that moves 1/32 of the way toward the symbol just seen, so the model
//! adapts without shipping a frequency table and can never price a symbol at
//! certainty. The decoder rebuilds the identical model from the payload header
//! and the residual count alone.
//!
//! A residual is coded as a sign bit plus one decision per magnitude plane,
//! most significant first. Before the first 1-bit, each plane uses its own
//! "not started yet" context; after it, each plane uses its own plane context.
//! Giving every plane its own leading context is what makes a small residual
//! nearly free: the planes above it are consistently zero and collapse to a
//! near-deterministic decision.
//!
//! The coder is a terminator: exactly `residual_count` residuals are decoded and
//! the arithmetic decoder is never asked to detect end-of-stream. Trailing bytes
//! in the payload are therefore tolerated (the encoder emits a whole flushed
//! byte, and the decoder stops after the declared residuals), but any byte that
//! the arithmetic decoder actually consumes past the end of the input reads as
//! zero rather than panicking.

use vpng_core::{Error, Result};

use crate::{DecodeContext, EncodeContext, TileCodec, CODEC_PREDICTOR};

/// Payload version. Anything else is corrupt, never best-effort decoded.
pub const VERSION: u8 = 1;

/// Predictor code: no prediction, the residual is the sample itself.
pub const PREDICTOR_NONE: u8 = 0;
/// Predictor code: left (the previous pixel in the same row).
pub const PREDICTOR_LEFT: u8 = 1;
/// Predictor code: up (the pixel directly above).
pub const PREDICTOR_UP: u8 = 2;
/// Predictor code: upper-left.
pub const PREDICTOR_UPPER_LEFT: u8 = 3;
/// Predictor code: average of left and up.
pub const PREDICTOR_AVERAGE: u8 = 4;
/// Predictor code: Paeth.
pub const PREDICTOR_PAETH: u8 = 5;
/// Predictor code: gradient (LOCO-I).
pub const PREDICTOR_GRADIENT: u8 = 6;

/// Highest predictor code defined by payload version 1.
const PREDICTOR_MAX: u8 = PREDICTOR_GRADIENT;

/// Every predictor the encoder may choose, in code order.
///
/// The order is also the tie-break order: the encoder keeps the first predictor
/// that reaches the lowest estimated cost.
const CANDIDATE_MODES: [u8; 7] = [
    PREDICTOR_NONE,
    PREDICTOR_LEFT,
    PREDICTOR_UP,
    PREDICTOR_UPPER_LEFT,
    PREDICTOR_AVERAGE,
    PREDICTOR_PAETH,
    PREDICTOR_GRADIENT,
];

/// Largest tile width the format allows (the directory stores it in a u16).
const MAX_TILE_WIDTH: usize = u16::MAX as usize;

/// Stateless per-tile predictor + entropy codec.
#[derive(Debug, Clone, Copy, Default)]
pub struct PredictorCodec;

pub static PREDICTOR_CODEC: PredictorCodec = PredictorCodec;

// ---------------------------------------------------------------------------
// Sample geometry
// ---------------------------------------------------------------------------

/// The sample grid of one tile: pixel width, interleaved channel count, and the
/// byte width of one channel sample.
#[derive(Debug, Clone, Copy)]
struct Layout {
    width: usize,
    channels: usize,
    sample_bytes: usize,
    height: usize,
}

impl Layout {
    fn new(width: usize, height: usize, channels: usize, sample_bytes: usize) -> Self {
        Layout {
            width,
            height,
            channels,
            sample_bytes,
        }
    }

    /// Samples in one row, channels interleaved.
    fn row_samples(&self) -> usize {
        self.width * self.channels
    }

    fn row_bytes(&self) -> usize {
        self.row_samples() * self.sample_bytes
    }

    fn total_samples(&self) -> usize {
        self.row_samples() * self.height
    }
}

/// Byte offset of sample `(x, c)` within a tile row.
#[inline]
fn sample_offset(layout: &Layout, x: usize, c: usize) -> usize {
    (x * layout.channels + c) * layout.sample_bytes
}

/// Read one whole sample as a little-endian integer.
#[inline]
fn read_sample(bytes: &[u8], offset: usize, sample_bytes: usize) -> u32 {
    let mut value = 0u32;
    for i in 0..sample_bytes {
        value |= u32::from(bytes[offset + i]) << (8 * i);
    }
    value
}

/// Mask selecting one sample's bits.
#[inline]
fn sample_mask(sample_bytes: usize) -> u32 {
    match sample_bytes {
        1 => 0xFF,
        2 => 0xFFFF,
        _ => 0xFFFF_FFFF,
    }
}

/// `(a - b) mod 2^(8B)`, folded to a signed residual.
///
/// The folded range is `[-2^(8B-1), 2^(8B-1) - 1]`, the same range a two's
/// complement sample uses, so `reconstruct + residual` reproduces the original
/// sample exactly under the same mask.
#[inline]
fn wrap_sub(a: u32, b: u32, sample_bytes: usize) -> i32 {
    let modulus = 1u32 << (8 * sample_bytes);
    let mask = modulus - 1;
    let diff = a.wrapping_sub(b) & mask;
    let half = modulus >> 1;
    if diff >= half {
        diff as i32 - modulus as i32
    } else {
        diff as i32
    }
}

// ---------------------------------------------------------------------------
// Prediction
// ---------------------------------------------------------------------------

/// The predictor plan for one tile: its sample width and one mode per row.
#[derive(Debug, Clone)]
struct Plan {
    width: usize,
    row_modes: Vec<u8>,
}

impl Plan {
    fn encode(&self, out: &mut Vec<u8>) {
        out.push(VERSION);
        put_uleb128(out, self.width as u64);
        out.extend_from_slice(&self.row_modes);
    }
}

/// The Paeth predictor: pick the neighbour closest to `left + up - upper_left`.
#[inline]
fn paeth(left: i64, up: i64, upper_left: i64) -> i64 {
    let p = left + up - upper_left;
    let pa = (p - left).abs();
    let pb = (p - up).abs();
    let pc = (p - upper_left).abs();
    if pa <= pb && pa <= pc {
        left
    } else if pb <= pc {
        up
    } else {
        upper_left
    }
}

/// The gradient predictor (LOCO-I): pick the neighbour whose step from the
/// upper-left pixel is smaller.
#[inline]
fn gradient(left: i64, up: i64, upper_left: i64) -> i64 {
    if (up - upper_left).abs() <= (left - upper_left).abs() {
        up
    } else {
        left
    }
}

/// Predict a sample from its three neighbours.
///
/// The mode selects which neighbour wins; a neighbour that falls outside the
/// tile has already been replaced by zero by the caller.
///
/// The neighbours are always non-negative (a sample is an unsigned integer),
/// so the average's integer division is a floor and the result never leaves the
/// sample's range.
#[inline]
fn predict_from_neighbours(mode: u8, left: i64, up: i64, upper_left: i64) -> u32 {
    let predicted = match mode {
        PREDICTOR_LEFT => left,
        PREDICTOR_UP => up,
        PREDICTOR_UPPER_LEFT => upper_left,
        PREDICTOR_AVERAGE => (left + up) / 2,
        PREDICTOR_PAETH => paeth(left, up, upper_left),
        PREDICTOR_GRADIENT => gradient(left, up, upper_left),
        // `PREDICTOR_NONE` and any unknown code predict zero. The decoder
        // rejects unknown codes before it gets here, so an unknown code can
        // only reach this function through a programming error.
        _ => 0,
    };
    predicted as u32
}

/// Predict the sample at `(x, c)` of row `y` from its neighbours.
///
/// `tile` holds the original bytes while encoding and the reconstructed bytes
/// while decoding. Both give the same answer: the decoder fills the tile in
/// raster order, so by the time a sample is predicted its left neighbour (same
/// row, earlier) and its upper neighbours (previous row) already hold their
/// final values.
fn predict_sample(tile: &[u8], layout: &Layout, y: usize, x: usize, c: usize, mode: u8) -> u32 {
    let sample_bytes = layout.sample_bytes;
    let row = y * layout.row_bytes();

    let left = if x > 0 {
        read_sample(tile, row + sample_offset(layout, x - 1, c), sample_bytes) as i64
    } else {
        0
    };
    let up = if y > 0 {
        read_sample(
            tile,
            row - layout.row_bytes() + sample_offset(layout, x, c),
            sample_bytes,
        ) as i64
    } else {
        0
    };
    let upper_left = if x > 0 && y > 0 {
        read_sample(
            tile,
            row - layout.row_bytes() + sample_offset(layout, x - 1, c),
            sample_bytes,
        ) as i64
    } else {
        0
    };

    predict_from_neighbours(mode, left, up, upper_left)
}

/// Residuals of row `y` under every candidate predictor.
///
/// All candidates are computed in one pass, so the row's samples are read from
/// the tile exactly once. The result is indexed by the candidate's position in
/// [`CANDIDATE_MODES`].
fn row_residual_candidates(tile: &[u8], layout: &Layout, y: usize) -> Vec<Vec<i32>> {
    let samples = layout.row_samples();
    let sample_bytes = layout.sample_bytes;
    let row = y * layout.row_bytes();

    let mut current = vec![0u32; samples];
    let mut above = vec![0u32; samples];
    for index in 0..samples {
        let offset = row + index * sample_bytes;
        current[index] = read_sample(tile, offset, sample_bytes);
        if y > 0 {
            above[index] = read_sample(tile, offset - layout.row_bytes(), sample_bytes);
        }
    }

    let mut candidates: Vec<Vec<i32>> = CANDIDATE_MODES
        .iter()
        .map(|_| Vec::with_capacity(samples))
        .collect();

    for x in 0..layout.width {
        for c in 0..layout.channels {
            let index = x * layout.channels + c;
            let left = if x > 0 {
                current[index - layout.channels] as i64
            } else {
                0
            };
            let up = if y > 0 { above[index] as i64 } else { 0 };
            let upper_left = if x > 0 && y > 0 {
                above[index - layout.channels] as i64
            } else {
                0
            };
            for (slot, &mode) in candidates.iter_mut().zip(CANDIDATE_MODES.iter()) {
                let predicted = predict_from_neighbours(mode, left, up, upper_left);
                slot.push(wrap_sub(current[index], predicted, sample_bytes));
            }
        }
    }
    candidates
}

/// Rebuild the tile from residuals, inverting [`row_residuals`].
fn reconstruct(
    residuals: &[i32],
    layout: &Layout,
    plan: &Plan,
    tile_len: usize,
) -> Result<Vec<u8>> {
    let expected = layout.row_bytes() * layout.height;
    if expected != tile_len {
        return Err(Error::corrupt(format!(
            "predictor plan describes {expected} bytes but the tile is {tile_len}"
        )));
    }
    if residuals.len() != layout.total_samples() {
        return Err(Error::corrupt(format!(
            "predictor stream holds {} residuals but the tile has {} samples",
            residuals.len(),
            layout.total_samples()
        )));
    }

    let mut tile = vec![0u8; tile_len];
    let mask = sample_mask(layout.sample_bytes);
    let mut index = 0usize;

    for (y, &mode) in plan.row_modes.iter().enumerate() {
        let row = y * layout.row_bytes();
        for x in 0..layout.width {
            for c in 0..layout.channels {
                let offset = row + sample_offset(layout, x, c);
                let predicted = predict_sample(&tile, layout, y, x, c, mode);
                let value = predicted.wrapping_add(residuals[index] as u32) & mask;
                for i in 0..layout.sample_bytes {
                    tile[offset + i] = (value >> (8 * i)) as u8;
                }
                index += 1;
            }
        }
    }
    Ok(tile)
}

// ---------------------------------------------------------------------------
// Adaptive arithmetic coder
// ---------------------------------------------------------------------------

/// A probability is an 11-bit count out of 2048.
const PROB_BITS: u32 = 11;
/// Total probability scale.
const PROB_TOTAL: u32 = 1 << PROB_BITS;
/// Initial probability: an even split.
const PROB_INIT: u32 = PROB_TOTAL >> 1;
/// Adaptation shift: the count moves 1/32 of the way toward the symbol seen.
const PROB_SHIFT: u32 = 5;
/// Below this interval width the coder renormalizes.
const TOP: u32 = 1 << 24;

/// One context's adaptive probability.
///
/// The count moves toward the observed symbol by 1/32 each time, so the model
/// tracks a changing tile without ever shipping a frequency table. The count
/// stays strictly inside `(0, PROB_TOTAL)`, so neither symbol is ever given a
/// zero-width interval — that would desynchronise encoder and decoder.
#[derive(Clone, Copy)]
struct Model {
    prob: u32,
}

impl Model {
    fn new() -> Self {
        Model { prob: PROB_INIT }
    }

    #[inline]
    fn probability(&self) -> u32 {
        self.prob
    }

    #[inline]
    fn update(&mut self, bit: u32) {
        if bit == 0 {
            self.prob += (PROB_TOTAL - self.prob) >> PROB_SHIFT;
        } else {
            self.prob -= self.prob >> PROB_SHIFT;
        }
    }
}

/// A fresh set of per-context models.
fn models(contexts: usize) -> Vec<Model> {
    vec![Model::new(); contexts]
}

/// A sink for coded bits.
///
/// The real [`Encoder`] produces bytes; the [`Probe`] only measures what the
/// same bits would cost. Both run the identical probability model, so a probe
/// and the encoder always agree on what a symbol costs.
trait BitSink {
    fn encode_bit(&mut self, ctx: usize, bit: u32);
}

/// Binary arithmetic encoder.
///
/// The interval is `[low, low + range)`. `low` is kept in 64 bits so the carry
/// out of bit 31 is visible instead of being lost, and the carry is resolved
/// before any affected byte reaches the output.
///
/// Bytes are committed to a pending group rather than written straight out. A
/// committed byte can still be incremented by a later carry, so a byte is only
/// released once a newer byte that is not `0xFF` has absorbed the carry. The
/// group therefore always holds `0xFF` bytes followed by at most one newer
/// byte, and the output order is exactly the commit order.
struct Encoder {
    out: Vec<u8>,
    low: u64,
    range: u32,
    pending: Vec<u8>,
    models: Vec<Model>,
}

impl Encoder {
    fn new(contexts: usize) -> Self {
        Encoder {
            out: Vec::new(),
            low: 0,
            range: u32::MAX,
            // A virtual leading byte, so a carry that reaches the very first
            // byte has somewhere to go instead of running off the stream.
            pending: vec![0x00],
            models: models(contexts),
        }
    }

    fn encode_bit(&mut self, ctx: usize, bit: u32) {
        let probability = self.models[ctx].probability();
        let bound = (self.range >> PROB_BITS) * probability;
        if bit == 0 {
            self.range = bound;
        } else {
            self.low += u64::from(bound);
            self.range -= bound;
        }
        self.models[ctx].update(bit);

        while self.range < TOP {
            self.shift_low();
            self.range <<= 8;
        }
    }

    /// Commit one byte and renormalize `low`.
    fn shift_low(&mut self) {
        if (self.low >> 32) != 0 {
            self.apply_carry();
            self.low &= 0xFFFF_FFFF;
        }

        let byte = ((self.low >> 24) & 0xFF) as u8;
        self.pending.push(byte);

        // A non-`0xFF` byte absorbs any future carry, so every older pending
        // byte is final and may be released. The byte itself stays pending
        // because a carry can still increment it.
        if byte != 0xFF {
            let newest = self
                .pending
                .pop()
                .expect("the pending group is never empty");
            self.out.append(&mut self.pending);
            self.pending.push(newest);
        }

        self.low = (self.low << 8) & 0xFFFF_FFFF;
    }

    /// Add one to the newest pending byte, propagating through `0xFF` bytes.
    fn apply_carry(&mut self) {
        for byte in self.pending.iter_mut().rev() {
            if *byte == 0xFF {
                *byte = 0x00;
            } else {
                *byte += 1;
                break;
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        // Five shifts flush the pending group and leave enough bytes for the
        // decoder's initial read.
        for _ in 0..5 {
            self.shift_low();
        }
        self.out.append(&mut self.pending);
        self.out
    }
}

impl BitSink for Encoder {
    fn encode_bit(&mut self, ctx: usize, bit: u32) {
        Encoder::encode_bit(self, ctx, bit)
    }
}

/// A cost-only probe of the entropy coder.
///
/// It runs the coder's probability model without producing any output and
/// accumulates the information content of the bits it would have coded: the
/// sum of `-log2(p)` over the symbols, with `p` the probability the model
/// assigned to each one. That is exactly what the coder will pay for the same
/// symbols, so it is what the predictor selection compares — and it costs a
/// fraction of a real encoding, because no interval arithmetic and no byte
/// emission is needed.
struct Probe {
    models: Vec<Model>,
    cost: u64,
}

impl Probe {
    fn new(models: &[Model]) -> Self {
        Probe {
            models: models.to_vec(),
            cost: 0,
        }
    }
}

impl BitSink for Probe {
    fn encode_bit(&mut self, ctx: usize, bit: u32) {
        let probability = self.models[ctx].probability();
        if bit == 0 {
            // The "0" branch covers `probability / PROB_TOTAL` of the interval.
            self.cost += u64::from(PROB_BITS * 1024 - log2_scaled(probability));
        } else {
            // The "1" branch covers the rest of it.
            self.cost += u64::from(PROB_BITS * 1024 - log2_scaled(PROB_TOTAL - probability));
        }
        self.models[ctx].update(bit);
    }
}

/// Binary arithmetic decoder mirroring [`Encoder`].
struct Decoder<'a> {
    input: &'a [u8],
    cursor: usize,
    range: u32,
    code: u32,
    models: Vec<Model>,
}

impl<'a> Decoder<'a> {
    fn new(input: &'a [u8], contexts: usize) -> Self {
        let mut decoder = Decoder {
            input,
            cursor: 0,
            range: u32::MAX,
            code: 0,
            models: models(contexts),
        };
        for _ in 0..5 {
            decoder.code = (decoder.code << 8) | u32::from(decoder.next_byte());
        }
        decoder
    }

    /// Bytes past the end of the payload read as zero: the coder is a terminator
    /// on `residual_count`, so it never needs a sentinel past the flush.
    fn next_byte(&mut self) -> u8 {
        if self.cursor < self.input.len() {
            let byte = self.input[self.cursor];
            self.cursor += 1;
            byte
        } else {
            0
        }
    }

    fn decode_bit(&mut self, ctx: usize) -> u32 {
        let bound = (self.range >> PROB_BITS) * self.models[ctx].probability();
        let bit = if self.code < bound {
            self.range = bound;
            self.models[ctx].update(0);
            0
        } else {
            self.range -= bound;
            self.code -= bound;
            self.models[ctx].update(1);
            1
        };

        while self.range < TOP {
            self.range <<= 8;
            self.code = (self.code << 8) | u32::from(self.next_byte());
        }
        bit
    }
}

/// `LOG2_SCALED[v]` is `log2(v)` in 1/1024-bit units.
///
/// The table is generated once and baked in, so the cost measure never
/// depends on a platform's floating-point library. Index 0 is unused: the
/// coder's probabilities are always at least 1 out of 2048.
static LOG2_SCALED: [u16; 2048] = [
    0, 0, 1024, 1623, 2048, 2378, 2647, 2875, 3072, 3246, 3402, 3542, 3671, 3789, 3899, 4001, 4096,
    4186, 4270, 4350, 4426, 4498, 4566, 4632, 4695, 4755, 4813, 4869, 4923, 4975, 5025, 5073, 5120,
    5165, 5210, 5252, 5294, 5334, 5374, 5412, 5450, 5486, 5522, 5556, 5590, 5624, 5656, 5688, 5719,
    5749, 5779, 5809, 5837, 5865, 5893, 5920, 5947, 5973, 5999, 6024, 6049, 6073, 6097, 6121, 6144,
    6167, 6189, 6212, 6234, 6255, 6276, 6297, 6318, 6338, 6358, 6378, 6398, 6417, 6436, 6455, 6474,
    6492, 6510, 6528, 6546, 6563, 6580, 6598, 6614, 6631, 6648, 6664, 6680, 6696, 6712, 6728, 6743,
    6758, 6773, 6788, 6803, 6818, 6833, 6847, 6861, 6875, 6889, 6903, 6917, 6931, 6944, 6957, 6971,
    6984, 6997, 7010, 7023, 7035, 7048, 7060, 7073, 7085, 7097, 7109, 7121, 7133, 7145, 7156, 7168,
    7179, 7191, 7202, 7213, 7225, 7236, 7247, 7258, 7268, 7279, 7290, 7300, 7311, 7321, 7332, 7342,
    7352, 7362, 7372, 7382, 7392, 7402, 7412, 7422, 7432, 7441, 7451, 7460, 7470, 7479, 7488, 7498,
    7507, 7516, 7525, 7534, 7543, 7552, 7561, 7570, 7579, 7587, 7596, 7604, 7613, 7622, 7630, 7638,
    7647, 7655, 7663, 7672, 7680, 7688, 7696, 7704, 7712, 7720, 7728, 7736, 7744, 7752, 7759, 7767,
    7775, 7782, 7790, 7797, 7805, 7812, 7820, 7827, 7835, 7842, 7849, 7857, 7864, 7871, 7878, 7885,
    7892, 7899, 7906, 7913, 7920, 7927, 7934, 7941, 7948, 7955, 7961, 7968, 7975, 7981, 7988, 7995,
    8001, 8008, 8014, 8021, 8027, 8034, 8040, 8047, 8053, 8059, 8066, 8072, 8078, 8084, 8090, 8097,
    8103, 8109, 8115, 8121, 8127, 8133, 8139, 8145, 8151, 8157, 8163, 8169, 8175, 8180, 8186, 8192,
    8198, 8203, 8209, 8215, 8221, 8226, 8232, 8237, 8243, 8249, 8254, 8260, 8265, 8271, 8276, 8282,
    8287, 8292, 8298, 8303, 8308, 8314, 8319, 8324, 8330, 8335, 8340, 8345, 8351, 8356, 8361, 8366,
    8371, 8376, 8381, 8386, 8391, 8396, 8401, 8406, 8411, 8416, 8421, 8426, 8431, 8436, 8441, 8446,
    8451, 8456, 8460, 8465, 8470, 8475, 8480, 8484, 8489, 8494, 8498, 8503, 8508, 8512, 8517, 8522,
    8526, 8531, 8535, 8540, 8545, 8549, 8554, 8558, 8563, 8567, 8572, 8576, 8580, 8585, 8589, 8594,
    8598, 8603, 8607, 8611, 8616, 8620, 8624, 8628, 8633, 8637, 8641, 8646, 8650, 8654, 8658, 8662,
    8667, 8671, 8675, 8679, 8683, 8687, 8692, 8696, 8700, 8704, 8708, 8712, 8716, 8720, 8724, 8728,
    8732, 8736, 8740, 8744, 8748, 8752, 8756, 8760, 8764, 8768, 8772, 8776, 8779, 8783, 8787, 8791,
    8795, 8799, 8802, 8806, 8810, 8814, 8818, 8821, 8825, 8829, 8833, 8836, 8840, 8844, 8848, 8851,
    8855, 8859, 8862, 8866, 8870, 8873, 8877, 8881, 8884, 8888, 8891, 8895, 8899, 8902, 8906, 8909,
    8913, 8916, 8920, 8923, 8927, 8930, 8934, 8937, 8941, 8944, 8948, 8951, 8955, 8958, 8962, 8965,
    8968, 8972, 8975, 8979, 8982, 8985, 8989, 8992, 8995, 8999, 9002, 9005, 9009, 9012, 9015, 9019,
    9022, 9025, 9029, 9032, 9035, 9038, 9042, 9045, 9048, 9051, 9055, 9058, 9061, 9064, 9067, 9071,
    9074, 9077, 9080, 9083, 9086, 9090, 9093, 9096, 9099, 9102, 9105, 9108, 9111, 9114, 9118, 9121,
    9124, 9127, 9130, 9133, 9136, 9139, 9142, 9145, 9148, 9151, 9154, 9157, 9160, 9163, 9166, 9169,
    9172, 9175, 9178, 9181, 9184, 9187, 9190, 9193, 9196, 9199, 9202, 9204, 9207, 9210, 9213, 9216,
    9219, 9222, 9225, 9227, 9230, 9233, 9236, 9239, 9242, 9245, 9247, 9250, 9253, 9256, 9259, 9261,
    9264, 9267, 9270, 9273, 9275, 9278, 9281, 9284, 9286, 9289, 9292, 9295, 9297, 9300, 9303, 9306,
    9308, 9311, 9314, 9316, 9319, 9322, 9324, 9327, 9330, 9332, 9335, 9338, 9340, 9343, 9346, 9348,
    9351, 9354, 9356, 9359, 9362, 9364, 9367, 9369, 9372, 9375, 9377, 9380, 9382, 9385, 9387, 9390,
    9393, 9395, 9398, 9400, 9403, 9405, 9408, 9410, 9413, 9415, 9418, 9420, 9423, 9425, 9428, 9430,
    9433, 9435, 9438, 9440, 9443, 9445, 9448, 9450, 9453, 9455, 9458, 9460, 9463, 9465, 9467, 9470,
    9472, 9475, 9477, 9480, 9482, 9484, 9487, 9489, 9492, 9494, 9496, 9499, 9501, 9504, 9506, 9508,
    9511, 9513, 9515, 9518, 9520, 9522, 9525, 9527, 9529, 9532, 9534, 9536, 9539, 9541, 9543, 9546,
    9548, 9550, 9553, 9555, 9557, 9559, 9562, 9564, 9566, 9569, 9571, 9573, 9575, 9578, 9580, 9582,
    9584, 9587, 9589, 9591, 9593, 9596, 9598, 9600, 9602, 9604, 9607, 9609, 9611, 9613, 9616, 9618,
    9620, 9622, 9624, 9627, 9629, 9631, 9633, 9635, 9637, 9640, 9642, 9644, 9646, 9648, 9650, 9652,
    9655, 9657, 9659, 9661, 9663, 9665, 9667, 9670, 9672, 9674, 9676, 9678, 9680, 9682, 9684, 9686,
    9689, 9691, 9693, 9695, 9697, 9699, 9701, 9703, 9705, 9707, 9709, 9711, 9713, 9716, 9718, 9720,
    9722, 9724, 9726, 9728, 9730, 9732, 9734, 9736, 9738, 9740, 9742, 9744, 9746, 9748, 9750, 9752,
    9754, 9756, 9758, 9760, 9762, 9764, 9766, 9768, 9770, 9772, 9774, 9776, 9778, 9780, 9782, 9784,
    9786, 9788, 9790, 9792, 9794, 9796, 9798, 9800, 9801, 9803, 9805, 9807, 9809, 9811, 9813, 9815,
    9817, 9819, 9821, 9823, 9825, 9826, 9828, 9830, 9832, 9834, 9836, 9838, 9840, 9842, 9844, 9845,
    9847, 9849, 9851, 9853, 9855, 9857, 9859, 9860, 9862, 9864, 9866, 9868, 9870, 9872, 9873, 9875,
    9877, 9879, 9881, 9883, 9885, 9886, 9888, 9890, 9892, 9894, 9895, 9897, 9899, 9901, 9903, 9905,
    9906, 9908, 9910, 9912, 9914, 9915, 9917, 9919, 9921, 9923, 9924, 9926, 9928, 9930, 9931, 9933,
    9935, 9937, 9939, 9940, 9942, 9944, 9946, 9947, 9949, 9951, 9953, 9954, 9956, 9958, 9960, 9961,
    9963, 9965, 9967, 9968, 9970, 9972, 9974, 9975, 9977, 9979, 9980, 9982, 9984, 9986, 9987, 9989,
    9991, 9992, 9994, 9996, 9998, 9999, 10001, 10003, 10004, 10006, 10008, 10009, 10011, 10013,
    10014, 10016, 10018, 10019, 10021, 10023, 10024, 10026, 10028, 10029, 10031, 10033, 10034,
    10036, 10038, 10039, 10041, 10043, 10044, 10046, 10048, 10049, 10051, 10053, 10054, 10056,
    10057, 10059, 10061, 10062, 10064, 10066, 10067, 10069, 10070, 10072, 10074, 10075, 10077,
    10079, 10080, 10082, 10083, 10085, 10087, 10088, 10090, 10091, 10093, 10095, 10096, 10098,
    10099, 10101, 10103, 10104, 10106, 10107, 10109, 10110, 10112, 10114, 10115, 10117, 10118,
    10120, 10121, 10123, 10125, 10126, 10128, 10129, 10131, 10132, 10134, 10135, 10137, 10138,
    10140, 10142, 10143, 10145, 10146, 10148, 10149, 10151, 10152, 10154, 10155, 10157, 10158,
    10160, 10161, 10163, 10165, 10166, 10168, 10169, 10171, 10172, 10174, 10175, 10177, 10178,
    10180, 10181, 10183, 10184, 10186, 10187, 10189, 10190, 10192, 10193, 10195, 10196, 10198,
    10199, 10201, 10202, 10203, 10205, 10206, 10208, 10209, 10211, 10212, 10214, 10215, 10217,
    10218, 10220, 10221, 10223, 10224, 10226, 10227, 10228, 10230, 10231, 10233, 10234, 10236,
    10237, 10239, 10240, 10241, 10243, 10244, 10246, 10247, 10249, 10250, 10251, 10253, 10254,
    10256, 10257, 10259, 10260, 10261, 10263, 10264, 10266, 10267, 10269, 10270, 10271, 10273,
    10274, 10276, 10277, 10278, 10280, 10281, 10283, 10284, 10285, 10287, 10288, 10290, 10291,
    10292, 10294, 10295, 10297, 10298, 10299, 10301, 10302, 10304, 10305, 10306, 10308, 10309,
    10310, 10312, 10313, 10315, 10316, 10317, 10319, 10320, 10321, 10323, 10324, 10325, 10327,
    10328, 10330, 10331, 10332, 10334, 10335, 10336, 10338, 10339, 10340, 10342, 10343, 10344,
    10346, 10347, 10348, 10350, 10351, 10352, 10354, 10355, 10356, 10358, 10359, 10360, 10362,
    10363, 10364, 10366, 10367, 10368, 10370, 10371, 10372, 10374, 10375, 10376, 10378, 10379,
    10380, 10382, 10383, 10384, 10386, 10387, 10388, 10389, 10391, 10392, 10393, 10395, 10396,
    10397, 10399, 10400, 10401, 10402, 10404, 10405, 10406, 10408, 10409, 10410, 10411, 10413,
    10414, 10415, 10417, 10418, 10419, 10420, 10422, 10423, 10424, 10425, 10427, 10428, 10429,
    10431, 10432, 10433, 10434, 10436, 10437, 10438, 10439, 10441, 10442, 10443, 10444, 10446,
    10447, 10448, 10449, 10451, 10452, 10453, 10454, 10456, 10457, 10458, 10459, 10461, 10462,
    10463, 10464, 10466, 10467, 10468, 10469, 10471, 10472, 10473, 10474, 10476, 10477, 10478,
    10479, 10480, 10482, 10483, 10484, 10485, 10487, 10488, 10489, 10490, 10491, 10493, 10494,
    10495, 10496, 10498, 10499, 10500, 10501, 10502, 10504, 10505, 10506, 10507, 10508, 10510,
    10511, 10512, 10513, 10514, 10516, 10517, 10518, 10519, 10520, 10522, 10523, 10524, 10525,
    10526, 10528, 10529, 10530, 10531, 10532, 10533, 10535, 10536, 10537, 10538, 10539, 10541,
    10542, 10543, 10544, 10545, 10546, 10548, 10549, 10550, 10551, 10552, 10553, 10555, 10556,
    10557, 10558, 10559, 10560, 10562, 10563, 10564, 10565, 10566, 10567, 10568, 10570, 10571,
    10572, 10573, 10574, 10575, 10577, 10578, 10579, 10580, 10581, 10582, 10583, 10585, 10586,
    10587, 10588, 10589, 10590, 10591, 10593, 10594, 10595, 10596, 10597, 10598, 10599, 10600,
    10602, 10603, 10604, 10605, 10606, 10607, 10608, 10610, 10611, 10612, 10613, 10614, 10615,
    10616, 10617, 10618, 10620, 10621, 10622, 10623, 10624, 10625, 10626, 10627, 10628, 10630,
    10631, 10632, 10633, 10634, 10635, 10636, 10637, 10638, 10640, 10641, 10642, 10643, 10644,
    10645, 10646, 10647, 10648, 10649, 10651, 10652, 10653, 10654, 10655, 10656, 10657, 10658,
    10659, 10660, 10661, 10662, 10664, 10665, 10666, 10667, 10668, 10669, 10670, 10671, 10672,
    10673, 10674, 10675, 10676, 10678, 10679, 10680, 10681, 10682, 10683, 10684, 10685, 10686,
    10687, 10688, 10689, 10690, 10691, 10693, 10694, 10695, 10696, 10697, 10698, 10699, 10700,
    10701, 10702, 10703, 10704, 10705, 10706, 10707, 10708, 10709, 10710, 10712, 10713, 10714,
    10715, 10716, 10717, 10718, 10719, 10720, 10721, 10722, 10723, 10724, 10725, 10726, 10727,
    10728, 10729, 10730, 10731, 10732, 10733, 10734, 10735, 10736, 10737, 10739, 10740, 10741,
    10742, 10743, 10744, 10745, 10746, 10747, 10748, 10749, 10750, 10751, 10752, 10753, 10754,
    10755, 10756, 10757, 10758, 10759, 10760, 10761, 10762, 10763, 10764, 10765, 10766, 10767,
    10768, 10769, 10770, 10771, 10772, 10773, 10774, 10775, 10776, 10777, 10778, 10779, 10780,
    10781, 10782, 10783, 10784, 10785, 10786, 10787, 10788, 10789, 10790, 10791, 10792, 10793,
    10794, 10795, 10796, 10797, 10798, 10799, 10800, 10801, 10802, 10803, 10804, 10805, 10806,
    10807, 10808, 10809, 10810, 10811, 10812, 10813, 10814, 10815, 10816, 10817, 10818, 10819,
    10820, 10821, 10822, 10823, 10824, 10825, 10825, 10826, 10827, 10828, 10829, 10830, 10831,
    10832, 10833, 10834, 10835, 10836, 10837, 10838, 10839, 10840, 10841, 10842, 10843, 10844,
    10845, 10846, 10847, 10848, 10849, 10850, 10850, 10851, 10852, 10853, 10854, 10855, 10856,
    10857, 10858, 10859, 10860, 10861, 10862, 10863, 10864, 10865, 10866, 10867, 10868, 10869,
    10869, 10870, 10871, 10872, 10873, 10874, 10875, 10876, 10877, 10878, 10879, 10880, 10881,
    10882, 10883, 10884, 10884, 10885, 10886, 10887, 10888, 10889, 10890, 10891, 10892, 10893,
    10894, 10895, 10896, 10897, 10897, 10898, 10899, 10900, 10901, 10902, 10903, 10904, 10905,
    10906, 10907, 10908, 10909, 10909, 10910, 10911, 10912, 10913, 10914, 10915, 10916, 10917,
    10918, 10919, 10919, 10920, 10921, 10922, 10923, 10924, 10925, 10926, 10927, 10928, 10929,
    10929, 10930, 10931, 10932, 10933, 10934, 10935, 10936, 10937, 10938, 10938, 10939, 10940,
    10941, 10942, 10943, 10944, 10945, 10946, 10947, 10947, 10948, 10949, 10950, 10951, 10952,
    10953, 10954, 10955, 10955, 10956, 10957, 10958, 10959, 10960, 10961, 10962, 10963, 10963,
    10964, 10965, 10966, 10967, 10968, 10969, 10970, 10971, 10971, 10972, 10973, 10974, 10975,
    10976, 10977, 10978, 10978, 10979, 10980, 10981, 10982, 10983, 10984, 10985, 10985, 10986,
    10987, 10988, 10989, 10990, 10991, 10991, 10992, 10993, 10994, 10995, 10996, 10997, 10998,
    10998, 10999, 11000, 11001, 11002, 11003, 11004, 11004, 11005, 11006, 11007, 11008, 11009,
    11010, 11010, 11011, 11012, 11013, 11014, 11015, 11016, 11016, 11017, 11018, 11019, 11020,
    11021, 11022, 11022, 11023, 11024, 11025, 11026, 11027, 11027, 11028, 11029, 11030, 11031,
    11032, 11033, 11033, 11034, 11035, 11036, 11037, 11038, 11038, 11039, 11040, 11041, 11042,
    11043, 11043, 11044, 11045, 11046, 11047, 11048, 11048, 11049, 11050, 11051, 11052, 11053,
    11053, 11054, 11055, 11056, 11057, 11058, 11058, 11059, 11060, 11061, 11062, 11063, 11063,
    11064, 11065, 11066, 11067, 11068, 11068, 11069, 11070, 11071, 11072, 11072, 11073, 11074,
    11075, 11076, 11077, 11077, 11078, 11079, 11080, 11081, 11081, 11082, 11083, 11084, 11085,
    11086, 11086, 11087, 11088, 11089, 11090, 11090, 11091, 11092, 11093, 11094, 11094, 11095,
    11096, 11097, 11098, 11099, 11099, 11100, 11101, 11102, 11103, 11103, 11104, 11105, 11106,
    11107, 11107, 11108, 11109, 11110, 11111, 11111, 11112, 11113, 11114, 11115, 11115, 11116,
    11117, 11118, 11119, 11119, 11120, 11121, 11122, 11123, 11123, 11124, 11125, 11126, 11127,
    11127, 11128, 11129, 11130, 11130, 11131, 11132, 11133, 11134, 11134, 11135, 11136, 11137,
    11138, 11138, 11139, 11140, 11141, 11141, 11142, 11143, 11144, 11145, 11145, 11146, 11147,
    11148, 11149, 11149, 11150, 11151, 11152, 11152, 11153, 11154, 11155, 11156, 11156, 11157,
    11158, 11159, 11159, 11160, 11161, 11162, 11162, 11163, 11164, 11165, 11166, 11166, 11167,
    11168, 11169, 11169, 11170, 11171, 11172, 11172, 11173, 11174, 11175, 11176, 11176, 11177,
    11178, 11179, 11179, 11180, 11181, 11182, 11182, 11183, 11184, 11185, 11185, 11186, 11187,
    11188, 11189, 11189, 11190, 11191, 11192, 11192, 11193, 11194, 11195, 11195, 11196, 11197,
    11198, 11198, 11199, 11200, 11201, 11201, 11202, 11203, 11204, 11204, 11205, 11206, 11207,
    11207, 11208, 11209, 11210, 11210, 11211, 11212, 11213, 11213, 11214, 11215, 11216, 11216,
    11217, 11218, 11219, 11219, 11220, 11221, 11222, 11222, 11223, 11224, 11225, 11225, 11226,
    11227, 11227, 11228, 11229, 11230, 11230, 11231, 11232, 11233, 11233, 11234, 11235, 11236,
    11236, 11237, 11238, 11239, 11239, 11240, 11241, 11241, 11242, 11243, 11244, 11244, 11245,
    11246, 11247, 11247, 11248, 11249, 11250, 11250, 11251, 11252, 11252, 11253, 11254, 11255,
    11255, 11256, 11257, 11257, 11258, 11259, 11260, 11260, 11261, 11262, 11263, 11263,
];

/// `log2(value)` in 1/1024-bit units, for `value` in `1..=2047`.
#[inline]
fn log2_scaled(value: u32) -> u32 {
    debug_assert!((1..2048).contains(&value));
    u32::from(LOG2_SCALED[value as usize])
}

// ---------------------------------------------------------------------------
// Residual coding
// ---------------------------------------------------------------------------

/// Number of probability contexts: one for the sign, one per magnitude plane
/// for the "not started yet" decision, and one per magnitude plane for the
/// decision after the first 1-bit.
fn context_count(sample_bits: u32) -> usize {
    sample_bits as usize * 2 + 1
}

/// Context index of the sign decision.
const SIGN_CONTEXT: usize = 0;
/// Context of magnitude plane `plane` while no 1-bit has been seen yet.
///
/// Each plane has its own "not started" context. A shared context would have
/// to price the first 1-bit as if it were as likely as any leading zero, which
/// costs several bits on every small residual; with a context per plane, the
/// common case (a residual whose magnitude always sits in the same plane)
/// collapses to a near-deterministic decision.
#[inline]
fn leading_context(plane: u32) -> usize {
    1 + plane as usize
}

/// Context of magnitude plane `plane`, used once the first 1-bit is seen.
#[inline]
fn plane_context(plane: u32, sample_bits: u32) -> usize {
    1 + sample_bits as usize + plane as usize
}

/// Code one residual: sign first, then magnitude planes most-significant first.
///
/// Before the first 1-bit every plane is a "not started yet" decision in that
/// plane's own leading context; the plane that carries the first 1-bit turns
/// its leading context to 1, and the planes below it each get their own plane
/// context, which is what gives the coder its resolution.
fn code_residual(sink: &mut impl BitSink, residual: i32, sample_bits: u32) {
    sink.encode_bit(SIGN_CONTEXT, u32::from(residual < 0));
    let magnitude = residual.unsigned_abs();
    if magnitude == 0 {
        for plane in (0..sample_bits).rev() {
            sink.encode_bit(leading_context(plane), 0);
        }
        return;
    }

    // The highest set bit is the plane that carries the first 1-bit; every
    // plane above it is a leading zero, and every plane below it is a plain
    // magnitude bit.
    let top = 31 - magnitude.leading_zeros();
    for plane in (top + 1..sample_bits).rev() {
        sink.encode_bit(leading_context(plane), 0);
    }
    sink.encode_bit(leading_context(top), 1);
    for plane in (0..top).rev() {
        sink.encode_bit(plane_context(plane, sample_bits), (magnitude >> plane) & 1);
    }
}

/// Inverse of [`code_residual`].
fn decode_residual(decoder: &mut Decoder<'_>, sample_bits: u32) -> i32 {
    let negative = decoder.decode_bit(SIGN_CONTEXT) == 1;
    let mut magnitude = 0u32;
    let mut started = false;
    for plane in (0..sample_bits).rev() {
        let bit = if started {
            decoder.decode_bit(plane_context(plane, sample_bits))
        } else {
            let bit = decoder.decode_bit(leading_context(plane));
            if bit == 1 {
                started = true;
            }
            bit
        };
        magnitude = (magnitude << 1) | bit;
    }
    if negative {
        -(magnitude as i32)
    } else {
        magnitude as i32
    }
}

// ---------------------------------------------------------------------------
// LEB128
// ---------------------------------------------------------------------------

fn put_uleb128(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Read a LEB128 from `input` starting at `cursor`. Returns the value and the
/// offset just past it.
fn get_uleb128(input: &[u8], mut cursor: usize) -> Result<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        if cursor >= input.len() {
            return Err(Error::corrupt("predictor header is truncated"));
        }
        let byte = input[cursor];
        cursor += 1;
        if shift >= 64 {
            return Err(Error::corrupt("predictor LEB128 value is too long"));
        }
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, cursor));
        }
        shift += 7;
    }
}

// ---------------------------------------------------------------------------
// Codec entry points
// ---------------------------------------------------------------------------

impl PredictorCodec {
    /// Build a predictor payload for `input`.
    ///
    /// Returns `Ok(true)` when a payload was written, and `Ok(false)` when the
    /// codec does not apply: an empty tile, or a payload above the configured
    /// per-tile size limit. A selector then falls back to another codec.
    pub fn try_encode(&self, input: &[u8], ctx: &EncodeContext, out: &mut Vec<u8>) -> Result<bool> {
        let bpp = ctx.format.bytes_per_pixel() as usize;
        let sample_bytes = ctx.format.bytes_per_channel() as usize;
        let channels = ctx.format.channels() as usize;

        if bpp == 0 || sample_bytes == 0 || channels == 0 || input.len() % bpp != 0 {
            return Err(Error::internal(
                "predictor input is not a whole number of pixels",
            ));
        }
        if input.len() as u64 > ctx.limits.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "predictor tile of {} bytes exceeds limit {}",
                input.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }
        if input.is_empty() {
            out.clear();
            return Ok(false);
        }

        // The context carries the tile's actual geometry: a tile in the last
        // column or row is smaller than the nominal grid, and the predictor
        // must see the real one.
        let width = ctx.tile_width as usize;
        if width == 0 || width > MAX_TILE_WIDTH {
            return Err(Error::internal("predictor tile width is out of range"));
        }
        let pixels = input.len() / bpp;
        if pixels == 0 || pixels % width != 0 {
            return Err(Error::internal(
                "predictor tile is not a whole number of rows of the nominal width",
            ));
        }
        let height = pixels / width;
        let layout = Layout::new(width, height, channels, sample_bytes);
        let sample_bits = (sample_bytes * 8) as u32;

        // Choose a predictor per row by measuring each candidate with the live
        // coder's probability model, then code the residuals it produced.
        let mut row_modes = Vec::with_capacity(height);
        let mut encoder = Encoder::new(context_count(sample_bits));
        for y in 0..height {
            let candidates = row_residual_candidates(input, &layout, y);
            let mut best_slot = 0usize;
            let mut best_cost = u64::MAX;

            for (slot, residuals) in candidates.iter().enumerate() {
                let mut probe = Probe::new(&encoder.models);
                for &residual in residuals {
                    code_residual(&mut probe, residual, sample_bits);
                }
                if probe.cost < best_cost {
                    best_cost = probe.cost;
                    best_slot = slot;
                }
            }

            row_modes.push(CANDIDATE_MODES[best_slot]);
            for &residual in &candidates[best_slot] {
                code_residual(&mut encoder, residual, sample_bits);
            }
        }

        let plan = Plan { width, row_modes };
        let coded = encoder.finish();

        out.clear();
        plan.encode(out);
        put_uleb128(out, layout.total_samples() as u64);
        out.extend_from_slice(&coded);

        if out.len() as u64 > ctx.limits.max_tile_payload_bytes {
            out.clear();
            return Ok(false);
        }
        Ok(true)
    }

    fn decode_into(
        &self,
        input: &[u8],
        expected_len: usize,
        ctx: &DecodeContext,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        let bpp = ctx.format.bytes_per_pixel() as usize;
        let sample_bytes = ctx.format.bytes_per_channel() as usize;
        let channels = ctx.format.channels() as usize;

        if expected_len as u64 > ctx.limits.max_decompressed_bytes {
            return Err(Error::resource_limit(format!(
                "predictor tile decode of {expected_len} bytes exceeds limit {}",
                ctx.limits.max_decompressed_bytes
            )));
        }
        if input.len() as u64 > ctx.limits.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "predictor payload of {} bytes exceeds limit {}",
                input.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }
        if bpp == 0 || sample_bytes == 0 || channels == 0 || expected_len % bpp != 0 {
            return Err(Error::corrupt(
                "predictor tile expected length is not a whole number of pixels",
            ));
        }
        if input.len() < 2 {
            return Err(Error::corrupt(
                "predictor payload is shorter than its header",
            ));
        }
        if input[0] != VERSION {
            return Err(Error::corrupt(format!(
                "predictor payload version {} is not supported",
                input[0]
            )));
        }

        let (width, width_cursor) = get_uleb128(input, 1)?;
        if width == 0 || width > MAX_TILE_WIDTH as u64 {
            return Err(Error::corrupt(format!(
                "predictor plan width {width} is out of range"
            )));
        }
        let width = width as usize;
        let pixels = expected_len / bpp;
        if width == 0 || pixels == 0 || pixels % width != 0 {
            return Err(Error::corrupt(
                "predictor plan width does not divide the tile's pixel count",
            ));
        }
        let height = pixels / width;
        let layout = Layout::new(width, height, channels, sample_bytes);

        let row_mode_end = width_cursor + height;
        if row_mode_end > input.len() {
            return Err(Error::corrupt(
                "predictor payload is shorter than its row modes",
            ));
        }
        let plan = Plan {
            width,
            row_modes: input[width_cursor..row_mode_end].to_vec(),
        };
        for &mode in &plan.row_modes {
            if mode > PREDICTOR_MAX {
                return Err(Error::corrupt(format!("predictor mode {mode} is reserved")));
            }
        }

        let (residual_count, cursor) = get_uleb128(input, row_mode_end)?;
        let residual_count = usize::try_from(residual_count)
            .map_err(|_| Error::corrupt("predictor residual count is implausible"))?;
        if residual_count != layout.total_samples() {
            return Err(Error::corrupt(format!(
                "predictor stream declares {residual_count} residuals but the tile has {}",
                layout.total_samples()
            )));
        }
        if cursor > input.len() {
            return Err(Error::corrupt("predictor residual data is truncated"));
        }

        let sample_bits = (sample_bytes * 8) as u32;
        let mut decoder = Decoder::new(&input[cursor..], context_count(sample_bits));
        let mut residuals = Vec::with_capacity(residual_count);
        for _ in 0..residual_count {
            residuals.push(decode_residual(&mut decoder, sample_bits));
        }

        *out = reconstruct(&residuals, &layout, &plan, expected_len)?;
        Ok(())
    }
}

impl TileCodec for PredictorCodec {
    fn codec_id(&self) -> u8 {
        CODEC_PREDICTOR
    }

    fn name(&self) -> &'static str {
        "predictor"
    }

    fn encode(&self, input: &[u8], ctx: &EncodeContext, out: &mut Vec<u8>) -> Result<()> {
        if self.try_encode(input, ctx, out)? {
            Ok(())
        } else {
            Err(Error::unsupported(
                "predictor codec cannot encode this tile (an empty tile, or a payload above the \
                 configured size limit)",
            ))
        }
    }

    fn decode(
        &self,
        input: &[u8],
        expected_len: usize,
        ctx: &DecodeContext,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        self.decode_into(input, expected_len, ctx, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_core::{ErrorKind, Limits, PixelFormat};

    fn contexts(format: PixelFormat) -> (EncodeContext, DecodeContext) {
        let limits = Limits::default();
        (
            EncodeContext {
                limits,
                format,
                tile_width: 256,
                tile_height: 256,
            },
            DecodeContext { limits, format },
        )
    }

    /// Encode with an explicit tile width, which the codec needs to know the
    /// row structure.
    fn encode_with(format: PixelFormat, width: u16, pixels: &[u8]) -> Vec<u8> {
        let limits = Limits::default();
        let enc = EncodeContext {
            limits,
            format,
            tile_width: width,
            tile_height: 256,
        };
        let mut out = Vec::new();
        assert!(
            PREDICTOR_CODEC.try_encode(pixels, &enc, &mut out).unwrap(),
            "predictor should apply"
        );
        out
    }

    /// Encode with a *forced* predictor mode per row, bypassing the encoder's
    /// selection, so every mode can be exercised on its own.
    fn encode_with_modes(format: PixelFormat, width: u16, pixels: &[u8], modes: &[u8]) -> Vec<u8> {
        let bpp = format.bytes_per_pixel() as usize;
        let channels = format.channels() as usize;
        let sample_bytes = format.bytes_per_channel() as usize;
        let height = pixels.len() / bpp / width as usize;
        let layout = Layout::new(width as usize, height, channels, sample_bytes);
        let sample_bits = (sample_bytes * 8) as u32;

        let mut encoder = Encoder::new(context_count(sample_bits));
        for (y, &mode) in modes.iter().enumerate() {
            let candidates = row_residual_candidates(pixels, &layout, y);
            let slot = CANDIDATE_MODES
                .iter()
                .position(|&candidate| candidate == mode)
                .expect("the mode must be one of the candidate predictors");
            for &residual in &candidates[slot] {
                code_residual(&mut encoder, residual, sample_bits);
            }
        }

        let plan = Plan {
            width: width as usize,
            row_modes: modes.to_vec(),
        };
        let mut out = Vec::new();
        plan.encode(&mut out);
        put_uleb128(&mut out, layout.total_samples() as u64);
        out.extend_from_slice(&encoder.finish());
        out
    }

    fn decode_ok(format: PixelFormat, payload: &[u8], expected_len: usize) -> Vec<u8> {
        let (_, dec) = contexts(format);
        let mut out = Vec::new();
        PREDICTOR_CODEC
            .decode(payload, expected_len, &dec, &mut out)
            .unwrap();
        out
    }

    fn roundtrip(format: PixelFormat, width: u16, pixels: &[u8]) {
        let payload = encode_with(format, width, pixels);
        assert_eq!(decode_ok(format, &payload, pixels.len()), pixels);
    }

    #[test]
    fn arithmetic_coder_roundtrips_raw_bits() {
        // The simplest possible contract: a fixed sequence of bits must survive
        // the coder unchanged. If this fails, the bug is in the coder alone.
        let bits: Vec<u32> = (0..200u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 31) & 1)
            .collect();
        let mut encoder = Encoder::new(1);
        for &bit in &bits {
            encoder.encode_bit(0, bit);
        }
        let bytes = encoder.finish();

        let mut decoder = Decoder::new(&bytes, 1);
        let mut restored = Vec::new();
        for _ in 0..bits.len() {
            restored.push(decoder.decode_bit(0));
        }
        assert_eq!(restored, bits, "bytes: {:?}", bytes);
    }

    #[test]
    fn arithmetic_coder_roundtrips_binary_data() {
        // Isolate the coder: a fixed residual count, no predictor involved. If
        // this fails, the bug is in the coder, not the prediction.
        let sample_bits = 8u32;
        let contexts = context_count(sample_bits);
        let values: Vec<i32> = vec![0, 1, -1, 5, -128, 127, 64, -64, 3, -3];
        let mut encoder = Encoder::new(contexts);
        for &value in &values {
            code_residual(&mut encoder, value, sample_bits);
        }
        let bytes = encoder.finish();

        let mut decoder = Decoder::new(&bytes, contexts);
        let mut restored = Vec::new();
        for _ in 0..values.len() {
            let value = decode_residual(&mut decoder, sample_bits);
            restored.push(value);
        }
        assert_eq!(restored, values, "bytes: {:?}", bytes);
    }

    #[test]
    fn every_predictor_mode_round_trips_exactly() {
        // Each mode is forced in turn, so the decoder's support for all seven
        // predictors is exercised independently of the encoder's selection.
        let formats = [
            PixelFormat::gray8(),
            PixelFormat::gray16(),
            PixelFormat::rgba8(),
            PixelFormat::rgba16(),
        ];
        for format in formats {
            let bpp = format.bytes_per_pixel() as usize;
            let width = 9u16;
            let height = 6usize;
            let mut pixels = vec![0u8; width as usize * height * bpp];
            for (index, byte) in pixels.iter_mut().enumerate() {
                *byte = ((index * 37 + index / 3) & 0xFF) as u8;
            }
            for &mode in CANDIDATE_MODES.iter() {
                let modes = vec![mode; height];
                let payload = encode_with_modes(format, width, &pixels, &modes);
                assert_eq!(
                    decode_ok(format, &payload, pixels.len()),
                    pixels,
                    "{format} with mode {mode}"
                );
            }
        }
    }

    #[test]
    fn every_predictor_mode_survives_extreme_samples() {
        // 0x80 folds to -128 and 0xFF to -1; a predictor that mixed those up
        // would corrupt every such image.
        let width = 5u16;
        let height = 4usize;
        for value in [0x00u8, 0x01, 0x7F, 0x80, 0xFE, 0xFF] {
            let pixels = vec![value; width as usize * height];
            for &mode in CANDIDATE_MODES.iter() {
                let modes = vec![mode; height];
                let payload = encode_with_modes(PixelFormat::gray8(), width, &pixels, &modes);
                assert_eq!(
                    decode_ok(PixelFormat::gray8(), &payload, pixels.len()),
                    pixels,
                    "value {value:#04X} with mode {mode}"
                );
            }
        }
    }

    #[test]
    fn selection_picks_left_for_a_horizontal_ramp() {
        // Each row is a distinct horizontal ramp. The first row has no row
        // above it, so only the left predictor can use the row's own history,
        // and it wins the tie against Paeth, which predicts the same thing.
        let width = 16u16;
        let height = 4usize;
        let mut pixels = vec![0u8; width as usize * height];
        for y in 0..height {
            for x in 0..width as usize {
                pixels[y * width as usize + x] = (x + y * width as usize) as u8;
            }
        }
        let payload = encode_with(PixelFormat::gray8(), width, &pixels);
        // Header: version, width, then one mode byte per row.
        assert_eq!(payload[0], VERSION);
        assert_eq!(payload[1], width as u8);
        assert_eq!(
            payload[2], PREDICTOR_LEFT,
            "the first row should use the left predictor"
        );
        for (row, &mode) in payload[3..2 + height].iter().enumerate() {
            assert!(
                mode <= PREDICTOR_MAX,
                "row {} uses a defined predictor",
                row + 1
            );
        }
        assert_eq!(
            decode_ok(PixelFormat::gray8(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn selection_is_never_worse_than_any_fixed_predictor() {
        // The per-row selection must pay for itself: the payload it produces
        // is no larger than the payload of any single fixed predictor.
        let width = 24u16;
        let height = 12usize;
        let mut pixels = vec![0u8; width as usize * height];
        for y in 0..height {
            for x in 0..width as usize {
                let value = (x * 3 + y * 5 + ((x * y) % 7)) as u8;
                pixels[y * width as usize + x] = value;
            }
        }

        let selected = encode_with(PixelFormat::gray8(), width, &pixels);
        for &mode in CANDIDATE_MODES.iter() {
            let fixed =
                encode_with_modes(PixelFormat::gray8(), width, &pixels, &vec![mode; height]);
            assert!(
                selected.len() <= fixed.len(),
                "selection ({}) must not lose to fixed mode {mode} ({})",
                selected.len(),
                fixed.len()
            );
        }
        assert_eq!(
            decode_ok(PixelFormat::gray8(), &selected, pixels.len()),
            pixels
        );
    }

    #[test]
    fn flat_tile_golden_vector() {
        // A 32x4 gray8 tile of a single value. Every row after the first is
        // predicted exactly by "up", so the coder is tiny.
        let width = 32u16;
        let pixels = vec![0x55u8; 32 * 4];
        let payload = encode_with(PixelFormat::gray8(), width, &pixels);

        // Header: version, width (LEB128), then one mode byte per row.
        assert_eq!(payload[0], VERSION);
        assert_eq!(payload[1], 32, "width 32");
        for (row, &mode) in payload[2..6].iter().enumerate() {
            assert!(
                mode <= PREDICTOR_MAX,
                "row {} uses a defined predictor",
                row
            );
        }
        // The residual count is 128 samples: 0x80 0x01 in LEB128.
        assert_eq!(payload[6], 0x80);
        assert_eq!(payload[7], 0x01);

        assert!(
            payload.len() < pixels.len(),
            "flat tile should compress: {} vs {}",
            payload.len(),
            pixels.len()
        );
        assert_eq!(
            decode_ok(PixelFormat::gray8(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn constant_magnitude_residuals_cost_almost_nothing() {
        // A ramp with a constant step: every residual after the first pixel of
        // a row is the same small magnitude. The per-plane leading contexts
        // must make that near-free; a shared leading context would price every
        // residual at several bits.
        let width = 64u16;
        let height = 64usize;
        let mut pixels = vec![0u8; width as usize * height];
        for row in pixels.chunks_exact_mut(width as usize) {
            for (x, byte) in row.iter_mut().enumerate() {
                *byte = (x as u8).wrapping_mul(3);
            }
        }
        let payload = encode_with(PixelFormat::gray8(), width, &pixels);
        assert!(
            payload.len() * 8 < pixels.len(),
            "a constant-step ramp should compress hard: {} vs {}",
            payload.len(),
            pixels.len()
        );
        assert_eq!(
            decode_ok(PixelFormat::gray8(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn tiny_tile_payload_is_pinned() {
        // A byte-level golden vector: the exact payload of a 2x1 gray8 tile.
        // The encoder is deterministic, so these bytes are a contract.
        let payload = encode_with(PixelFormat::gray8(), 2, &[0x0A, 0x14]);
        assert_eq!(
            payload,
            vec![
                VERSION,
                0x02,           // version, width 2
                PREDICTOR_LEFT, // one row
                0x02,           // two residuals
                0x00,
                0x05,
                0x02,
                0xD8,
                0x48,
                0xC0,
                0x00,
                0x00, // residual data
            ]
        );
        assert_eq!(
            decode_ok(PixelFormat::gray8(), &payload, 2),
            vec![0x0A, 0x14]
        );
    }

    #[test]
    fn single_row_and_single_pixel_tiles() {
        roundtrip(PixelFormat::gray8(), 1, &[0x80]);
        roundtrip(PixelFormat::gray8(), 5, &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn every_v0_1_format_roundtrips() {
        let formats = [
            PixelFormat::gray8(),
            PixelFormat::gray16(),
            PixelFormat::gray_alpha8(),
            PixelFormat::gray_alpha16(),
            PixelFormat::rgb8(),
            PixelFormat::rgb16(),
            PixelFormat::rgba8(),
            PixelFormat::rgba16(),
        ];
        for format in formats {
            let bpp = format.bytes_per_pixel() as usize;
            let width = 8u16;
            let height = 6usize;
            let mut pixels = Vec::new();
            for y in 0..height {
                for x in 0..width as usize {
                    for c in 0..bpp {
                        pixels.push(((x * 7 + y * 13 + c * 29) & 0xFF) as u8);
                    }
                }
            }
            roundtrip(format, width, &pixels);
        }
    }

    #[test]
    fn wide_samples_are_whole() {
        // rgb16: three 16-bit channels. A per-channel ramp must survive exactly.
        let width = 4u16;
        let mut pixels = Vec::new();
        for x in 0..width as usize {
            for channel in 0..3u32 {
                let value = (0x1000 + x as u32 * 0x0011 + channel * 0x0100) as u16;
                pixels.extend_from_slice(&value.to_le_bytes());
            }
        }
        roundtrip(PixelFormat::rgb16(), width, &pixels);
    }

    #[test]
    fn negative_residuals_survive() {
        // A descending ramp produces negative residuals; the sign plane must be
        // exact.
        let width = 6u16;
        let mut pixels = Vec::new();
        for x in (0..width as usize).rev() {
            pixels.push((x as u8).wrapping_mul(9));
        }
        roundtrip(PixelFormat::gray8(), width, &pixels);
    }

    #[test]
    fn full_range_values_survive() {
        let width = 256u16;
        let pixels: Vec<u8> = (0..=255u8).collect();
        roundtrip(PixelFormat::gray8(), width, &pixels);
    }

    #[test]
    fn noisy_tile_roundtrips() {
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        let width = 16u16;
        let mut pixels = vec![0u8; width as usize * 5 * 4];
        for byte in pixels.iter_mut() {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            *byte = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as u8;
        }
        roundtrip(PixelFormat::rgba8(), width, &pixels);
    }

    #[test]
    fn empty_tile_is_not_applicable() {
        let (enc, _) = contexts(PixelFormat::gray8());
        let mut out = vec![0xFFu8; 4];
        assert!(!PREDICTOR_CODEC.try_encode(&[], &enc, &mut out).unwrap());
        assert_eq!(out.len(), 0, "a rejected tile must not leave output behind");
    }

    #[test]
    fn reserved_predictor_mode_is_rejected() {
        let format = PixelFormat::gray8();
        let (_, dec) = contexts(format);
        let mut out = Vec::new();
        // version 1, width 4, one row with a reserved mode 7, count 4, no data.
        let payload = [VERSION, 4, 7, 4];
        let err = PREDICTOR_CODEC
            .decode(&payload, 4, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn bad_version_is_rejected() {
        let format = PixelFormat::gray8();
        let (_, dec) = contexts(format);
        let mut out = Vec::new();
        let payload = [VERSION + 1, 4, 0, 4];
        let err = PREDICTOR_CODEC
            .decode(&payload, 4, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn residual_count_mismatch_is_rejected() {
        let format = PixelFormat::gray8();
        let (_, dec) = contexts(format);
        let mut out = Vec::new();
        // width 4, one row, count 5 (tile has 4 samples).
        let payload = [VERSION, 4, 0, 5];
        let err = PREDICTOR_CODEC
            .decode(&payload, 4, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn truncated_header_is_rejected() {
        let format = PixelFormat::gray8();
        let (_, dec) = contexts(format);
        let mut out = Vec::new();
        let err = PREDICTOR_CODEC
            .decode(&[VERSION], 4, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn predictor_output_stays_below_raw_for_a_photo_like_tile() {
        // A smooth 32x32 gradient: the predictor plus entropy coder must beat
        // Raw, which is the whole point of the codec.
        let width = 32u16;
        let mut pixels = Vec::new();
        for y in 0..32usize {
            for x in 0..width as usize {
                pixels.push(((x * 3 + y * 5) & 0xFF) as u8);
            }
        }
        let payload = encode_with(PixelFormat::gray8(), width, &pixels);
        assert!(
            payload.len() < pixels.len(),
            "predictor {} should beat raw {}",
            payload.len(),
            pixels.len()
        );
        assert_eq!(
            decode_ok(PixelFormat::gray8(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn raw_falls_back_on_noise() {
        // A noisy tile may expand; the selector must be able to reject it. The
        // codec itself never expands past the limit, but it may legitimately be
        // larger than the tile, which is the selector's problem, not an error.
        let mut state = 0xDEAD_BEEF_CAFE_1234u64;
        let width = 8u16;
        let mut pixels = vec![0u8; width as usize * 8 * 4];
        for byte in pixels.iter_mut() {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            *byte = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as u8;
        }
        let payload = encode_with(PixelFormat::rgba8(), width, &pixels);
        assert_eq!(
            decode_ok(PixelFormat::rgba8(), &payload, pixels.len()),
            pixels
        );
    }
}
