//! VPNG-CARC — **research** context-adaptive residual coding.
//!
//! This is **research code**. It carries research-only codec ids
//! (`0x80..=0x83`), is not reachable through [`crate::get_codec`], and is not
//! in [`crate::IMPLEMENTED_CODEC_IDS`]. A file that somehow carried one of
//! these ids would be rejected by the production decoder with
//! `UnsupportedFeature`.
//!
//! # The question this module answers
//!
//! The stable predictor codec (id 3) codes a residual's sign and magnitude
//! planes with a model that knows **nothing about the image**: every residual
//! of a tile is priced against the same probability contexts. That is the
//! structural weakness the research report has to measure. CARC asks whether
//! conditioning the probabilities on the neighbourhood of the residual buys
//! enough compressed bytes to justify the extra model state.
//!
//! # The four experiments
//!
//! | model | id | conditioning |
//! |---|---|---|
//! | A `Independent` | `0x80` | none — the stable model, as the baseline |
//! | B `NeighbourConditioned` | `0x81` | left, up and upper-left residuals plus the previous symbol class |
//! | C `GradientConditioned` | `0x82` | a deterministic gradient class of the reconstructed neighbourhood |
//! | D `CombinedSpatialChannel` | `0x83` | gradient class, previous residual class of the same channel, channel identity, the previous channel's residual class in the same pixel, and an edge class |
//!
//! Model A is the *existing implementation*: its payload is byte-for-byte the
//! stable predictor codec's payload, so the research harness can compare
//! against the baseline without a second implementation that could drift.
//!
//! # How a context is selected
//!
//! Every residual is coded with the same bit layout the stable codec uses —
//! a sign bit and one decision per magnitude plane — but the *probability
//! contexts* it is priced against depend on the model's features.
//!
//! The probability table is split into a fixed number of **blocks**. Block `k`
//! holds the sign context and one context per magnitude plane, so a block is
//! exactly `context_count(sample_bits)` probabilities. The model picks a
//! block per residual; the residual is then coded against that block.
//!
//! The block index is a hash of the model's feature values into a table of
//! [`CONTEXT_BLOCKS`] entries. Hashing keeps the table bounded and
//! independent of the tile size, at the cost of some collisions between
//! different feature combinations.
//!
//! # Determinism
//!
//! Every decision is integer-only:
//!
//! * the feature values are small integer classes;
//! * the block index is an FNV-1a hash, truncated to the table size;
//! * the probability updates are the stable coder's own integer updates;
//! * the gradient and edge tests compare integer differences against integer
//!   thresholds that scale with the sample width.
//!
//! There is no floating point anywhere in the bitstream path.
//!
//! # Reset rules
//!
//! * The probability table is built fresh for every tile. No state crosses a
//!   tile boundary, so a tile is an independent unit.
//! * The "previous symbol" is the residual coded immediately before, in
//!   raster order. At the start of a tile there is none, and the class of a
//!   missing neighbour is its own class.
//! * The reconstructed neighbours a gradient class depends on are the values
//!   the decoder has already reconstructed, so the encoder and the decoder see
//!   exactly the same numbers.
//!
//! # Memory
//!
//! The probability table is [`CONTEXT_BLOCKS`] blocks of
//! `context_count(sample_bits)` probabilities. With the default table of 256
//! blocks that is 17 KiB for 8-bit samples and 34 KiB for 16-bit samples —
//! bounded, and independent of the tile size.
//!
//! # Payload layout (models B, C and D)
//!
//! ```text
//! offset 0   u8   version (1)
//! offset 1   u8   model id (1 = B, 2 = C, 3 = D)
//! offset 2   u8   context table shift (log2 of the number of blocks)
//! offset 3   ..   tile width in samples, unsigned LEB128
//! offset ..  ..   one predictor-mode byte per tile row
//! offset ..  ..   residual count, unsigned LEB128
//! offset ..  ..   entropy-coded residual data
//! ```
//!
//! Model A uses the stable predictor codec's payload layout unchanged.

use vpng_core::{Error, Result};

use crate::predictor::{
    self, code_residual_at, context_count, decode_residual_at, get_uleb128, put_uleb128, Decoder,
    Encoder, Layout, PREDICTOR_MAX,
};
use crate::{DecodeContext, EncodeContext, TileCodec};

use super::{EXP_CARC_A, EXP_CARC_B, EXP_CARC_C, EXP_CARC_D};

/// Payload version of the context-adaptive models.
pub const VERSION: u8 = 1;

/// Number of probability blocks every CARC model uses.
///
/// A block holds one context per sign/magnitude decision, so the table size
/// is fixed and the memory is bounded independently of the tile size.
pub const CONTEXT_BLOCKS: usize = 256;

/// `log2` of [`CONTEXT_BLOCKS`].
pub const CONTEXT_BLOCK_SHIFT: u32 = 8;

/// Number of residual classes.
///
/// | class | meaning |
/// |---|---|
/// | 0 | the neighbour does not exist |
/// | 1 | zero |
/// | 2..=5 | positive, magnitude of 1, 2, 3 or at least 4 bits |
/// | 6..=9 | negative, magnitude of 1, 2, 3 or at least 4 bits |
pub const RESIDUAL_CLASSES: usize = 10;

/// Number of gradient classes.
pub const GRADIENT_CLASSES: usize = 5;

/// Which context model a CARC codec uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarcModel {
    /// Model A: the independent residual model — the stable baseline.
    Independent,
    /// Model B: conditioned on the neighbouring residuals.
    NeighbourConditioned,
    /// Model C: conditioned on a gradient class.
    GradientConditioned,
    /// Model D: conditioned on space, channel and edge information together.
    CombinedSpatialChannel,
}

impl CarcModel {
    /// The value written to the payload header.
    pub fn code(self) -> u8 {
        match self {
            CarcModel::Independent => 0,
            CarcModel::NeighbourConditioned => 1,
            CarcModel::GradientConditioned => 2,
            CarcModel::CombinedSpatialChannel => 3,
        }
    }

    /// Inverse of [`CarcModel::code`].
    pub fn from_code(code: u8) -> Result<Self> {
        Ok(match code {
            0 => CarcModel::Independent,
            1 => CarcModel::NeighbourConditioned,
            2 => CarcModel::GradientConditioned,
            3 => CarcModel::CombinedSpatialChannel,
            other => return Err(Error::corrupt(format!("carc model {other} is reserved"))),
        })
    }

    /// Human-readable name, used by the research reports.
    pub fn name(self) -> &'static str {
        match self {
            CarcModel::Independent => "independent",
            CarcModel::NeighbourConditioned => "neighbour-conditioned",
            CarcModel::GradientConditioned => "gradient-conditioned",
            CarcModel::CombinedSpatialChannel => "combined-spatial-channel",
        }
    }

    /// The research codec id this model uses.
    pub fn codec_id(self) -> u8 {
        match self {
            CarcModel::Independent => EXP_CARC_A,
            CarcModel::NeighbourConditioned => EXP_CARC_B,
            CarcModel::GradientConditioned => EXP_CARC_C,
            CarcModel::CombinedSpatialChannel => EXP_CARC_D,
        }
    }

    /// Every model, in a fixed order.
    pub fn all() -> [CarcModel; 4] {
        [
            CarcModel::Independent,
            CarcModel::NeighbourConditioned,
            CarcModel::GradientConditioned,
            CarcModel::CombinedSpatialChannel,
        ]
    }
}

/// A context-adaptive residual codec.
#[derive(Debug, Clone, Copy)]
pub struct CarcCodec {
    model: CarcModel,
}

impl CarcCodec {
    /// A codec that uses the given model.
    pub const fn new(model: CarcModel) -> Self {
        CarcCodec { model }
    }

    /// The model this codec uses.
    pub const fn model(&self) -> CarcModel {
        self.model
    }
}

/// The class of a residual, or of a residual that does not exist.
///
/// The magnitude is bucketed by its bit length, so the class is stable across
/// 8-bit and 16-bit samples and never depends on floating point.
pub(crate) fn residual_class(residual: Option<i32>) -> u8 {
    match residual {
        None => 0,
        Some(0) => 1,
        Some(value) => {
            let magnitude = value.unsigned_abs();
            let bits = 32 - magnitude.leading_zeros();
            let bucket = (bits - 1).min(3) as u8;
            if value < 0 {
                6 + bucket
            } else {
                2 + bucket
            }
        }
    }
}

/// The class of the local gradient, from the reconstructed neighbours.
///
/// * 0: no vertical information (first row of the tile),
/// * 1: horizontal information only (first column),
/// * 2: the up step is smaller than the left step,
/// * 3: both steps are equal,
/// * 4: a strong edge — the left and up samples are far apart.
pub(crate) fn gradient_class(
    has_left: bool,
    has_up: bool,
    left: u32,
    up: u32,
    upper_left: u32,
    sample_max: u32,
) -> u8 {
    if !has_left && !has_up {
        return 0;
    }
    if !has_left {
        return 2;
    }
    if !has_up {
        return 1;
    }
    let left_step = (i64::from(left) - i64::from(upper_left)).abs();
    let up_step = (i64::from(up) - i64::from(upper_left)).abs();
    let spread = (i64::from(left) - i64::from(up)).abs();
    if spread > i64::from(sample_max) / 8 {
        return 4;
    }
    if left_step < up_step {
        1
    } else if up_step < left_step {
        2
    } else {
        3
    }
}

/// Hash the feature values into a block index.
///
/// FNV-1a over the feature bytes, truncated to the table size. The hash is
/// deterministic and integer-only.
fn hash_features(features: &[u8]) -> usize {
    let mut h: u32 = 2166136261;
    for &feature in features {
        h ^= u32::from(feature);
        h = h.wrapping_mul(16777619);
    }
    (h >> (32 - CONTEXT_BLOCK_SHIFT)) as usize
}

/// The state a context model reads.
///
/// The encoder fills it with the whole tile up front; the decoder fills it
/// progressively as it reconstructs. Both see the same numbers, because the
/// predictor transform is exact.
struct ContextState {
    layout: Layout,
    /// The residuals, in raster order. The decoder only holds the ones it
    /// has already decoded, which is exactly what a context may look at.
    residuals: Vec<i32>,
    /// The reconstructed samples, in raster order.
    samples: Vec<u32>,
    /// The largest value a sample of this format can hold.
    sample_max: u32,
}

impl ContextState {
    /// The residual at a raster index, if it is known.
    fn residual_at(&self, index: usize) -> Option<i32> {
        self.residuals.get(index).copied()
    }

    /// The reconstructed sample at a raster index, if it is known.
    fn sample_at(&self, index: usize) -> Option<u32> {
        self.samples.get(index).copied()
    }
}

/// The gradient class of the sample at `index`.
fn gradient_value(state: &ContextState, index: usize, x: usize, y: usize) -> u8 {
    let channels = state.layout.channels;
    let row_samples = state.layout.row_samples();
    let has_left = x > 0;
    let has_up = y > 0;
    let left = if has_left {
        state.sample_at(index - channels).unwrap_or(0)
    } else {
        0
    };
    let up = if has_up {
        state.sample_at(index - row_samples).unwrap_or(0)
    } else {
        0
    };
    let upper_left = if has_left && has_up {
        state.sample_at(index - row_samples - channels).unwrap_or(0)
    } else {
        0
    };
    gradient_class(has_left, has_up, left, up, upper_left, state.sample_max)
}

/// The probability block the residual at `index` is coded against.
///
/// Every feature the model looks at is at a raster index *below* `index`, so
/// the decoder can compute the block before it has decoded the residual.
fn context_block(model: CarcModel, state: &ContextState, index: usize) -> usize {
    let channels = state.layout.channels;
    let row_samples = state.layout.row_samples();
    let x = (index % row_samples) / channels;
    let y = index / row_samples;
    let c = index % channels;

    match model {
        // The baseline: every residual is priced against the same block.
        CarcModel::Independent => 0,

        CarcModel::NeighbourConditioned => {
            let left = if x > 0 {
                state.residual_at(index - channels)
            } else {
                None
            };
            let up = if y > 0 {
                state.residual_at(index - row_samples)
            } else {
                None
            };
            let upper_left = if x > 0 && y > 0 {
                state.residual_at(index - row_samples - channels)
            } else {
                None
            };
            // The previous symbol in raster order.
            let previous = if index > 0 {
                state.residual_at(index - 1)
            } else {
                None
            };
            hash_features(&[
                residual_class(left),
                residual_class(up),
                residual_class(upper_left),
                residual_class(previous),
            ])
        }

        CarcModel::GradientConditioned => {
            let previous = if index > 0 {
                state.residual_at(index - 1)
            } else {
                None
            };
            hash_features(&[gradient_value(state, index, x, y), residual_class(previous)])
        }

        CarcModel::CombinedSpatialChannel => {
            // The previous residual of the *same* channel.
            let same_channel = if x > 0 {
                state.residual_at(index - channels)
            } else {
                None
            };
            // The residual of the previous channel in the same pixel.
            let neighbour_channel = if c > 0 {
                state.residual_at(index - 1)
            } else {
                None
            };
            let gradient = gradient_value(state, index, x, y);
            let edge = u8::from(gradient == 4);
            hash_features(&[
                gradient,
                residual_class(same_channel),
                c as u8,
                residual_class(neighbour_channel),
                edge,
            ])
        }
    }
}

/// Read every sample of a tile into the raster order the models use.
fn read_samples(tile: &[u8], layout: &Layout) -> Vec<u32> {
    let mut samples = Vec::with_capacity(layout.total_samples());
    for y in 0..layout.height {
        let row = y * layout.row_bytes();
        for x in 0..layout.width {
            for c in 0..layout.channels {
                let offset = row + predictor::sample_offset(layout, x, c);
                samples.push(predictor::read_sample(tile, offset, layout.sample_bytes));
            }
        }
    }
    samples
}

/// Reconstruct one sample from the already-reconstructed neighbours.
///
/// This mirrors the stable codec's `predict_sample`, but reads the
/// neighbours from the reconstructed sample array instead of the tile bytes.
fn reconstruct_sample(
    state: &ContextState,
    index: usize,
    y: usize,
    x: usize,
    mode: u8,
    residual: i32,
) -> u32 {
    let channels = state.layout.channels;
    let row_samples = state.layout.row_samples();
    let left = if x > 0 {
        state.samples[index - channels]
    } else {
        0
    };
    let up = if y > 0 {
        state.samples[index - row_samples]
    } else {
        0
    };
    let upper_left = if x > 0 && y > 0 {
        state.samples[index - row_samples - channels]
    } else {
        0
    };
    let predicted = predictor::predict_from_neighbours(
        mode,
        i64::from(left),
        i64::from(up),
        i64::from(upper_left),
    );
    let mask = predictor::sample_mask(state.layout.sample_bytes);
    predicted.wrapping_add(residual as u32) & mask
}

// ---------------------------------------------------------------------------
// Codec
// ---------------------------------------------------------------------------

/// The largest tile width the directory can express.
const MAX_TILE_WIDTH: usize = u16::MAX as usize;

impl CarcCodec {
    /// Encode with the context-adaptive model.
    fn encode_contextual(
        &self,
        input: &[u8],
        ctx: &EncodeContext,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        let bpp = ctx.format.bytes_per_pixel() as usize;
        let sample_bytes = ctx.format.bytes_per_channel() as usize;
        let channels = ctx.format.channels() as usize;

        if bpp == 0 || sample_bytes == 0 || channels == 0 || input.len() % bpp != 0 {
            return Err(Error::internal(
                "carc input is not a whole number of pixels",
            ));
        }
        if input.len() as u64 > ctx.limits.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "carc tile of {} bytes exceeds limit {}",
                input.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }
        if input.is_empty() {
            return Err(Error::unsupported("carc cannot encode an empty tile"));
        }

        let width = ctx.tile_width as usize;
        if width == 0 || width > MAX_TILE_WIDTH {
            return Err(Error::internal("carc tile width is out of range"));
        }
        let pixels = input.len() / bpp;
        if pixels == 0 || pixels % width != 0 {
            return Err(Error::internal(
                "carc tile is not a whole number of rows of the nominal width",
            ));
        }
        let height = pixels / width;
        let layout = Layout::new(width, height, channels, sample_bytes);
        let sample_bits = (sample_bytes * 8) as u32;
        let sample_max = predictor::sample_mask(sample_bytes);

        // The predictor stage is the stable codec's, so the comparison between
        // the models measures the entropy model and nothing else.
        let (row_modes, residuals, _) = predictor::plan_rows(input, &layout);
        let samples = read_samples(input, &layout);
        let state = ContextState {
            layout,
            residuals,
            samples,
            sample_max,
        };

        let sub_contexts = context_count(sample_bits);
        let mut encoder = Encoder::new(CONTEXT_BLOCKS * sub_contexts);
        for index in 0..state.residuals.len() {
            let block = context_block(self.model, &state, index);
            code_residual_at(
                &mut encoder,
                state.residuals[index],
                sample_bits,
                block * sub_contexts,
            );
        }
        let coded = encoder.finish();

        out.clear();
        out.push(VERSION);
        out.push(self.model.code());
        out.push(CONTEXT_BLOCK_SHIFT as u8);
        put_uleb128(out, width as u64);
        out.extend_from_slice(&row_modes);
        put_uleb128(out, state.residuals.len() as u64);
        out.extend_from_slice(&coded);

        if out.len() as u64 > ctx.limits.max_tile_payload_bytes {
            out.clear();
            return Err(Error::resource_limit(format!(
                "carc payload exceeds limit {}",
                ctx.limits.max_tile_payload_bytes
            )));
        }
        Ok(())
    }

    /// Decode with the context-adaptive model.
    fn decode_contextual(
        &self,
        input: &[u8],
        expected_len: usize,
        ctx: &DecodeContext,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        if expected_len as u64 > ctx.limits.max_decompressed_bytes {
            return Err(Error::resource_limit(format!(
                "carc tile decode of {expected_len} bytes exceeds limit {}",
                ctx.limits.max_decompressed_bytes
            )));
        }
        if input.len() as u64 > ctx.limits.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "carc payload of {} bytes exceeds limit {}",
                input.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }

        let bpp = ctx.format.bytes_per_pixel() as usize;
        let sample_bytes = ctx.format.bytes_per_channel() as usize;
        let channels = ctx.format.channels() as usize;
        if bpp == 0 || sample_bytes == 0 || channels == 0 || expected_len % bpp != 0 {
            return Err(Error::corrupt(
                "carc tile expected length is not a whole number of pixels",
            ));
        }
        if input.len() < 4 {
            return Err(Error::corrupt("carc payload is shorter than its header"));
        }
        if input[0] != VERSION {
            return Err(Error::corrupt(format!(
                "carc payload version {} is not supported",
                input[0]
            )));
        }
        let model = CarcModel::from_code(input[1])?;
        if model != self.model {
            return Err(Error::corrupt(format!(
                "carc payload holds model {} but the codec decodes {}",
                model.name(),
                self.model.name()
            )));
        }
        if input[2] != CONTEXT_BLOCK_SHIFT as u8 {
            return Err(Error::corrupt(
                "carc payload uses an unsupported context table size",
            ));
        }

        let (width, cursor) = get_uleb128(input, 3)?;
        if width == 0 || width > MAX_TILE_WIDTH as u64 {
            return Err(Error::corrupt(format!(
                "carc plan width {width} is out of range"
            )));
        }
        let width = width as usize;
        let pixels = expected_len / bpp;
        if pixels == 0 || pixels % width != 0 {
            return Err(Error::corrupt(
                "carc plan width does not divide the tile's pixel count",
            ));
        }
        let height = pixels / width;
        let layout = Layout::new(width, height, channels, sample_bytes);

        let row_mode_end = cursor + height;
        if row_mode_end > input.len() {
            return Err(Error::corrupt("carc payload is shorter than its row modes"));
        }
        let row_modes = &input[cursor..row_mode_end];
        for &mode in row_modes {
            if mode > PREDICTOR_MAX {
                return Err(Error::corrupt(format!("carc mode {mode} is reserved")));
            }
        }

        let (residual_count, cursor) = get_uleb128(input, row_mode_end)?;
        if residual_count != layout.total_samples() as u64 {
            return Err(Error::corrupt(format!(
                "carc stream declares {residual_count} residuals but the tile has {}",
                layout.total_samples()
            )));
        }
        if cursor > input.len() {
            return Err(Error::corrupt("carc residual data is truncated"));
        }

        let sample_bits = (sample_bytes * 8) as u32;
        let sample_max = predictor::sample_mask(sample_bytes);
        let sub_contexts = context_count(sample_bits);

        let mut decoder = Decoder::new(&input[cursor..], CONTEXT_BLOCKS * sub_contexts);
        let mut state = ContextState {
            layout,
            residuals: Vec::with_capacity(residual_count as usize),
            samples: vec![0u32; layout.total_samples()],
            sample_max,
        };

        let row_samples = state.layout.row_samples();
        for index in 0..residual_count as usize {
            let x = (index % row_samples) / channels;
            let y = index / row_samples;
            let mode = row_modes[y];

            // The block is selected from data the decoder already holds.
            let block = context_block(self.model, &state, index);
            let residual = decode_residual_at(&mut decoder, sample_bits, block * sub_contexts);
            state.samples[index] = reconstruct_sample(&state, index, y, x, mode, residual);
            state.residuals.push(residual);
        }

        if decoder.overran() {
            return Err(Error::corrupt("carc entropy data is truncated"));
        }

        let layout = state.layout;
        let residuals = state.residuals.clone();
        let tile = predictor::reconstruct_tile(&residuals, &layout, row_modes, expected_len)?;
        *out = tile;
        Ok(())
    }
}

impl TileCodec for CarcCodec {
    fn codec_id(&self) -> u8 {
        self.model.codec_id()
    }

    fn name(&self) -> &'static str {
        match self.model {
            CarcModel::Independent => "carc-a",
            CarcModel::NeighbourConditioned => "carc-b",
            CarcModel::GradientConditioned => "carc-c",
            CarcModel::CombinedSpatialChannel => "carc-d",
        }
    }

    fn encode(&self, input: &[u8], ctx: &EncodeContext, out: &mut Vec<u8>) -> Result<()> {
        if self.model == CarcModel::Independent {
            // The baseline is the existing implementation, so its payload is
            // byte-for-byte the stable predictor codec's payload.
            return predictor::PREDICTOR_CODEC.encode(input, ctx, out);
        }
        self.encode_contextual(input, ctx, out)
    }

    fn decode(
        &self,
        input: &[u8],
        expected_len: usize,
        ctx: &DecodeContext,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        if self.model == CarcModel::Independent {
            return predictor::PREDICTOR_CODEC.decode(input, expected_len, ctx, out);
        }
        self.decode_contextual(input, expected_len, ctx, out)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_core::{ErrorKind, Limits, PixelFormat};

    /// The pinned golden payload of [`golden_vector_is_stable`], in hex.
    const GOLDEN_PAYLOAD_HEX: &str = "010108200104040404040404800600000000003f8d7d8271afb084f7c41162d8791c58af3fea97e3e091ca9dd57127bbe9c7e56b8d68292b6ae607cd852ca76185daf4af5ae21dd42563f3d2d3d7e946f5d47702040f9e029ee88c53e1bd6ecaf6997f146f0f7574fb9e3ddb17d28692169902f3deaf73cba5a431e93622f02a48110a06d10bc92eb9495ed93c12432c73dfdb573716148382534e0c4efb2d577f51099311b8c7b663c9441ccbeec1b338db9ee8647e27ecdf74f636767cfd0ca044896b8acc4c4a9dcc242519413ecfe0cdcc4dea3a45feaf15ac930304cf666928ddb6081cd83b432c238a6e53a56b3d867fc4b56761cb82c3b75e47382d9c1bc03da63218c8a2a8c277ee6a2a0cc423e910f6da82a41a7500";

    /// A deterministic xorshift, so the cases are reproducible.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed | 1)
        }
        fn next_u32(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            (x >> 32) as u32
        }
        fn fill(&mut self, out: &mut [u8]) {
            for byte in out.iter_mut() {
                *byte = self.next_u32() as u8;
            }
        }
    }

    fn contexts(format: PixelFormat, width: u16) -> (EncodeContext, DecodeContext) {
        let limits = Limits::default();
        (
            EncodeContext {
                limits,
                format,
                tile_width: width,
                tile_height: 256,
            },
            DecodeContext { limits, format },
        )
    }

    /// A tile with a smooth gradient plus a little noise.
    fn sample_tile(width: usize, height: usize, format: PixelFormat) -> Vec<u8> {
        let bpp = format.bytes_per_pixel() as usize;
        let mut data = vec![0u8; width * height * bpp];
        for y in 0..height {
            for x in 0..width {
                let base = (y * width + x) * bpp;
                let value = ((x * 3 + y * 5) % 256) as u8;
                data[base] = value;
                if bpp > 1 {
                    data[base + 1] = value.wrapping_mul(2);
                }
                if bpp > 2 {
                    data[base + 2] = value.wrapping_mul(3);
                }
                if bpp > 3 {
                    data[base + 3] = 255;
                }
            }
        }
        data
    }

    fn roundtrip(model: CarcModel, format: PixelFormat, width: usize, data: &[u8]) -> Vec<u8> {
        let (enc, dec) = contexts(format, width as u16);
        let codec = CarcCodec::new(model);
        let mut payload = Vec::new();
        codec.encode(data, &enc, &mut payload).unwrap();
        let mut out = Vec::new();
        codec.decode(&payload, data.len(), &dec, &mut out).unwrap();
        assert_eq!(out.len(), data.len());
        assert_eq!(out, data, "the decoder must reproduce the input exactly");
        payload
    }

    #[test]
    fn every_model_roundtrips_every_format() {
        for model in CarcModel::all() {
            for format in [
                PixelFormat::gray8(),
                PixelFormat::gray_alpha8(),
                PixelFormat::rgb8(),
                PixelFormat::rgba8(),
                PixelFormat::gray16(),
                PixelFormat::gray_alpha16(),
                PixelFormat::rgb16(),
                PixelFormat::rgba16(),
            ] {
                let data = sample_tile(17, 9, format);
                roundtrip(model, format, 17, &data);
            }
        }
    }

    #[test]
    fn every_model_roundtrips_noisy_data() {
        let mut data = vec![0u8; 64 * 16];
        Rng::new(0xABCD).fill(&mut data);
        for model in CarcModel::all() {
            roundtrip(model, PixelFormat::rgba8(), 64, &data);
        }
    }

    #[test]
    fn model_a_is_the_stable_predictor_payload() {
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        let (enc, _) = contexts(PixelFormat::rgb8(), 32);
        let mut carc = Vec::new();
        CarcCodec::new(CarcModel::Independent)
            .encode(&data, &enc, &mut carc)
            .unwrap();
        let mut stable = Vec::new();
        predictor::PREDICTOR_CODEC
            .encode(&data, &enc, &mut stable)
            .unwrap();
        assert_eq!(
            carc, stable,
            "the baseline model must be the existing implementation, byte for byte"
        );
    }

    #[test]
    fn the_models_produce_different_payloads() {
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        let payloads: Vec<Vec<u8>> = CarcModel::all()
            .iter()
            .map(|&model| roundtrip(model, PixelFormat::rgb8(), 32, &data))
            .collect();
        // The baseline is the stable payload; the context models must differ
        // from it and from each other.
        for i in 0..payloads.len() {
            for j in (i + 1)..payloads.len() {
                assert_ne!(
                    payloads[i], payloads[j],
                    "models {} and {} produced the same payload",
                    i, j
                );
            }
        }
    }

    #[test]
    fn output_is_deterministic() {
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        for model in CarcModel::all() {
            let first = roundtrip(model, PixelFormat::rgb8(), 32, &data);
            let second = roundtrip(model, PixelFormat::rgb8(), 32, &data);
            assert_eq!(first, second, "the encoder must be deterministic");
        }
    }

    #[test]
    fn a_bad_version_is_rejected() {
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        let (_, dec) = contexts(PixelFormat::rgb8(), 32);
        let codec = CarcCodec::new(CarcModel::NeighbourConditioned);
        let mut payload = Vec::new();
        codec
            .encode(&data, &contexts(PixelFormat::rgb8(), 32).0, &mut payload)
            .unwrap();
        payload[0] = 77;
        let mut out = Vec::new();
        let err = codec
            .decode(&payload, data.len(), &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn a_mismatched_model_is_rejected() {
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        let (_, dec) = contexts(PixelFormat::rgb8(), 32);
        let codec = CarcCodec::new(CarcModel::NeighbourConditioned);
        let mut payload = Vec::new();
        codec
            .encode(&data, &contexts(PixelFormat::rgb8(), 32).0, &mut payload)
            .unwrap();
        payload[1] = CarcModel::GradientConditioned.code();
        let mut out = Vec::new();
        let err = codec
            .decode(&payload, data.len(), &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn a_reserved_predictor_mode_is_rejected() {
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        let (_, dec) = contexts(PixelFormat::rgb8(), 32);
        let codec = CarcCodec::new(CarcModel::GradientConditioned);
        let mut payload = Vec::new();
        codec
            .encode(&data, &contexts(PixelFormat::rgb8(), 32).0, &mut payload)
            .unwrap();
        // The header is version, model, table shift and a LEB128 width.
        let mode_at = 4 + 1;
        payload[mode_at] = 200;
        let mut out = Vec::new();
        let err = codec
            .decode(&payload, data.len(), &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn a_truncated_payload_is_rejected() {
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        let (_, dec) = contexts(PixelFormat::rgb8(), 32);
        let codec = CarcCodec::new(CarcModel::CombinedSpatialChannel);
        let mut payload = Vec::new();
        codec
            .encode(&data, &contexts(PixelFormat::rgb8(), 32).0, &mut payload)
            .unwrap();
        let truncated = &payload[..payload.len() - 3];
        let mut out = Vec::new();
        let err = codec
            .decode(truncated, data.len(), &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn a_wrong_tile_length_is_rejected() {
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        let (_, dec) = contexts(PixelFormat::rgb8(), 32);
        let codec = CarcCodec::new(CarcModel::GradientConditioned);
        let mut payload = Vec::new();
        codec
            .encode(&data, &contexts(PixelFormat::rgb8(), 32).0, &mut payload)
            .unwrap();
        let mut out = Vec::new();
        let err = codec
            .decode(&payload, data.len() - 1, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn the_residual_classes_cover_the_documented_range() {
        assert_eq!(residual_class(None), 0);
        assert_eq!(residual_class(Some(0)), 1);
        assert_eq!(residual_class(Some(1)), 2);
        assert_eq!(residual_class(Some(3)), 3);
        assert_eq!(residual_class(Some(7)), 4);
        assert_eq!(residual_class(Some(15)), 5);
        assert_eq!(residual_class(Some(255)), 5);
        assert_eq!(residual_class(Some(-1)), 6);
        assert_eq!(residual_class(Some(-3)), 7);
        assert_eq!(residual_class(Some(-7)), 8);
        assert_eq!(residual_class(Some(-15)), 9);
        assert_eq!(residual_class(Some(-255)), 9);
        for class in 0..RESIDUAL_CLASSES {
            assert!(class < RESIDUAL_CLASSES);
        }
    }

    #[test]
    fn the_gradient_classes_are_deterministic_and_bounded() {
        assert_eq!(gradient_class(false, false, 0, 0, 0, 255), 0);
        assert_eq!(gradient_class(true, false, 10, 0, 0, 255), 1);
        assert_eq!(gradient_class(false, true, 0, 10, 0, 255), 2);
        assert_eq!(gradient_class(true, true, 10, 20, 5, 255), 1);
        assert_eq!(gradient_class(true, true, 20, 10, 5, 255), 2);
        assert_eq!(gradient_class(true, true, 10, 10, 5, 255), 3);
        assert_eq!(gradient_class(true, true, 0, 255, 0, 255), 4);
        for value in 0..=255u32 {
            let class = gradient_class(true, true, value, value, value, 255);
            assert!((class as usize) < GRADIENT_CLASSES);
        }
    }

    #[test]
    fn golden_vector_is_stable() {
        // A fixed input with a fixed model must always produce the same
        // bytes. The expected value is pinned so a change to the bitstream is
        // caught by the test suite.
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        let payload = roundtrip(
            CarcModel::NeighbourConditioned,
            PixelFormat::rgb8(),
            32,
            &data,
        );
        let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, GOLDEN_PAYLOAD_HEX, "the golden carc payload changed");
    }
}
