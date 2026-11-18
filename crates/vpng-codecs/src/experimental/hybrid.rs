//! The hybrid research codecs: predictor, LZ77 and entropy coding combined.
//!
//! This is **research code**. It carries research-only codec ids
//! (`0x91..=0x93`), is not reachable through [`crate::get_codec`], and is not
//! in [`crate::IMPLEMENTED_CODEC_IDS`]. A file that somehow carried one of
//! these ids would be rejected by the production decoder with
//! `UnsupportedFeature`.
//!
//! # The three hybrids
//!
//! | kind | id | pipeline |
//! |---|---|---|
//! | `PredictorThenLz77` | `0x91` | predictor → LZ77 |
//! | `Lz77ThenEntropy` | `0x92` | LZ77 → entropy |
//! | `PredictorThenLz77ThenEntropy` | `0x93` | predictor → LZ77 → entropy |
//!
//! The stages are **not** combined automatically. Each hybrid is a separate
//! candidate, and the research harness selects between them by the complete
//! payload size — headers included — with a deterministic tie-break.
//!
//! # Why the predictor stage feeds *bytes* to LZ77
//!
//! The stable predictor codec codes a residual as a sign and a magnitude, one
//! decision per plane. LZ77 needs a byte stream it can search. The hybrid
//! therefore folds each signed residual into a fixed-width unsigned byte:
//!
//! ```text
//! byte = residual + 2^(8B-1)
//! ```
//!
//! where `B` is the sample width. The fold is exact and reversible: the
//! residual range `[-2^(8B-1), 2^(8B-1) - 1]` maps onto `[0, 2^(8B) - 1]`.
//! A 16-bit sample stays a 16-bit value; the hybrid never splits a sample
//! across two residuals.
//!
//! # The entropy stage
//!
//! The entropy stage reuses the stable arithmetic coder but gives each
//! *symbol group* of the LZ77 token stream its own probability context: flag
//! bytes, literal bytes, length codes, distance codes and extra bytes all
//! have different statistics and should not share one model.
//!
//! The group of every byte is implied by the token structure, so the decoder
//! rebuilds it without any extra bytes in the stream.
//!
//! # Payload layout
//!
//! ```text
//! offset 0   u8   hybrid version (1)
//! offset 1   u8   kind code
//! ```
//!
//! The predictor stages then carry the stable predictor's plan:
//!
//! ```text
//! offset ..  u8   predictor version (1)
//! offset ..  ..   tile width in samples, unsigned LEB128
//! offset ..  ..   one predictor-mode byte per tile row
//! ```
//!
//! and every kind carries the LZ77 stage's own header (version, strategy,
//! minimum and maximum match, window and hash sizes, chain length, and the
//! length of the stage's output), followed by the token stream — verbatim for
//! `PredictorThenLz77`, entropy-coded for the other two.
//!
//! # Tile locality
//!
//! Every stage is tile-local. The predictor reads only the tile, the LZ77
//! dictionary only holds the tile's own bytes, and the entropy model is reset
//! at the start of every tile.

use vpng_core::{Error, Result};

use crate::experimental::{entropy, lz77};
use crate::predictor::{self, get_uleb128, put_uleb128, Layout, PREDICTOR_MAX};
use crate::{DecodeContext, EncodeContext, TileCodec};

use super::{EXP_LZ77_ENTROPY, EXP_PRED_LZ77, EXP_PRED_LZ77_ENTROPY};

/// Payload version of the hybrid codecs.
pub const VERSION: u8 = 1;

/// The largest tile width the directory can express.
const MAX_TILE_WIDTH: usize = u16::MAX as usize;

/// Which pipeline a hybrid codec runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HybridKind {
    /// Predictor, then LZ77 over the residual bytes.
    PredictorThenLz77,
    /// LZ77 over the tile bytes, then entropy coding of the token stream.
    Lz77ThenEntropy,
    /// Predictor, then LZ77, then entropy coding of the token stream.
    PredictorThenLz77ThenEntropy,
}

impl HybridKind {
    /// The value written to the payload header.
    pub fn code(self) -> u8 {
        match self {
            HybridKind::PredictorThenLz77 => 0,
            HybridKind::Lz77ThenEntropy => 1,
            HybridKind::PredictorThenLz77ThenEntropy => 2,
        }
    }

    /// Inverse of [`HybridKind::code`].
    pub fn from_code(code: u8) -> Result<Self> {
        Ok(match code {
            0 => HybridKind::PredictorThenLz77,
            1 => HybridKind::Lz77ThenEntropy,
            2 => HybridKind::PredictorThenLz77ThenEntropy,
            other => return Err(Error::corrupt(format!("hybrid kind {other} is reserved"))),
        })
    }

    /// Human-readable name, used by the research reports.
    pub fn name(self) -> &'static str {
        match self {
            HybridKind::PredictorThenLz77 => "pred-lz77",
            HybridKind::Lz77ThenEntropy => "lz77-entropy",
            HybridKind::PredictorThenLz77ThenEntropy => "pred-lz77-entropy",
        }
    }

    /// The research codec id this kind uses.
    pub fn codec_id(self) -> u8 {
        match self {
            HybridKind::PredictorThenLz77 => EXP_PRED_LZ77,
            HybridKind::Lz77ThenEntropy => EXP_LZ77_ENTROPY,
            HybridKind::PredictorThenLz77ThenEntropy => EXP_PRED_LZ77_ENTROPY,
        }
    }

    /// Every kind, in a fixed order.
    pub fn all() -> [HybridKind; 3] {
        [
            HybridKind::PredictorThenLz77,
            HybridKind::Lz77ThenEntropy,
            HybridKind::PredictorThenLz77ThenEntropy,
        ]
    }

    /// Whether the pipeline runs the predictor stage.
    pub fn has_predictor(self) -> bool {
        !matches!(self, HybridKind::Lz77ThenEntropy)
    }

    /// Whether the pipeline runs the entropy stage.
    pub fn has_entropy(self) -> bool {
        !matches!(self, HybridKind::PredictorThenLz77)
    }
}

/// A hybrid research codec.
#[derive(Debug, Clone, Copy)]
pub struct HybridCodec {
    kind: HybridKind,
    params: lz77::Lz77Params,
}

impl HybridCodec {
    /// A hybrid with the default LZ77 parameters.
    pub const fn new(kind: HybridKind) -> Self {
        HybridCodec {
            kind,
            params: lz77::Lz77Params::DEFAULT,
        }
    }

    /// A hybrid with explicit LZ77 parameters.
    pub const fn with_params(kind: HybridKind, params: lz77::Lz77Params) -> Self {
        HybridCodec { kind, params }
    }

    /// The kind this codec runs.
    pub const fn kind(&self) -> HybridKind {
        self.kind
    }

    /// The LZ77 parameters this codec uses.
    pub const fn params(&self) -> &lz77::Lz77Params {
        &self.params
    }
}

/// Fold signed residuals into unsigned fixed-width bytes.
///
/// The fold is exact: the residual range maps onto the whole byte range, so
/// the reverse is exact too.
pub(crate) fn residuals_to_bytes(residuals: &[i32], sample_bytes: usize) -> Vec<u8> {
    let half = 1i64 << (sample_bytes * 8 - 1);
    let mut out = Vec::with_capacity(residuals.len() * sample_bytes);
    for &residual in residuals {
        let value = (i64::from(residual) + half) as u32;
        for i in 0..sample_bytes {
            out.push((value >> (8 * i)) as u8);
        }
    }
    out
}

/// Unfold fixed-width bytes back into signed residuals.
pub(crate) fn bytes_to_residuals(bytes: &[u8], sample_bytes: usize) -> Vec<i32> {
    let half = 1i64 << (sample_bytes * 8 - 1);
    let mask = (1u32 << (sample_bytes * 8)) - 1;
    let mut out = Vec::with_capacity(bytes.len() / sample_bytes);
    for chunk in bytes.chunks_exact(sample_bytes) {
        let mut value = 0u32;
        for (i, &byte) in chunk.iter().enumerate() {
            value |= u32::from(byte) << (8 * i);
        }
        let signed = i64::from(value & mask) - half;
        out.push(signed as i32);
    }
    out
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// The geometry of the tile a hybrid is encoding.
struct TileGeometry {
    layout: Layout,
    sample_bytes: usize,
}

fn tile_geometry(input: &[u8], ctx: &EncodeContext) -> Result<TileGeometry> {
    let bpp = ctx.format.bytes_per_pixel() as usize;
    let sample_bytes = ctx.format.bytes_per_channel() as usize;
    let channels = ctx.format.channels() as usize;

    if bpp == 0 || sample_bytes == 0 || channels == 0 || input.len() % bpp != 0 {
        return Err(Error::internal(
            "hybrid input is not a whole number of pixels",
        ));
    }
    if input.is_empty() {
        return Err(Error::unsupported("hybrid cannot encode an empty tile"));
    }
    let width = ctx.tile_width as usize;
    if width == 0 || width > MAX_TILE_WIDTH {
        return Err(Error::internal("hybrid tile width is out of range"));
    }
    let pixels = input.len() / bpp;
    if pixels == 0 || pixels % width != 0 {
        return Err(Error::internal(
            "hybrid tile is not a whole number of rows of the nominal width",
        ));
    }
    let height = pixels / width;
    Ok(TileGeometry {
        layout: Layout::new(width, height, channels, sample_bytes),
        sample_bytes,
    })
}

impl HybridCodec {
    fn encode_hybrid(&self, input: &[u8], ctx: &EncodeContext, out: &mut Vec<u8>) -> Result<()> {
        self.params.validate()?;
        if input.len() as u64 > ctx.limits.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "hybrid tile of {} bytes exceeds limit {}",
                input.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }

        let geometry = tile_geometry(input, ctx)?;
        let layout = geometry.layout;

        // Stage one: the predictor, when the pipeline runs it.
        let row_modes = if self.kind.has_predictor() {
            let (modes, residuals, _) = predictor::plan_rows(input, &layout);
            Some((modes, residuals))
        } else {
            None
        };

        // Stage two: the byte stream the LZ77 stage searches.
        let stage_bytes: Vec<u8> = match &row_modes {
            Some((_, residuals)) => residuals_to_bytes(residuals, geometry.sample_bytes),
            None => input.to_vec(),
        };

        // Stage three: the token stream, optionally entropy-coded.
        let tokens = lz77::tokenize(&stage_bytes, &self.params)?;
        let stream = lz77::tokens_to_stream(&tokens);
        let body: Vec<u8> = if self.kind.has_entropy() {
            let groups: Vec<usize> = stream.iter().map(|tagged| tagged.group).collect();
            let bytes: Vec<u8> = stream.iter().map(|tagged| tagged.byte).collect();
            entropy::encode_bytes(&groups, &bytes)
        } else {
            stream.iter().map(|tagged| tagged.byte).collect()
        };

        out.clear();
        out.push(VERSION);
        out.push(self.kind.code());
        if let Some((modes, _)) = &row_modes {
            out.push(predictor::VERSION);
            put_uleb128(out, layout.width as u64);
            out.extend_from_slice(modes);
        }
        lz77::write_header(out, &self.params, stage_bytes.len() as u64);
        out.extend_from_slice(&body);

        if out.len() as u64 > ctx.limits.max_tile_payload_bytes {
            out.clear();
            return Err(Error::resource_limit(format!(
                "hybrid payload exceeds limit {}",
                ctx.limits.max_tile_payload_bytes
            )));
        }
        Ok(())
    }

    fn decode_hybrid(
        &self,
        input: &[u8],
        expected_len: usize,
        ctx: &DecodeContext,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        if expected_len as u64 > ctx.limits.max_decompressed_bytes {
            return Err(Error::resource_limit(format!(
                "hybrid tile decode of {expected_len} bytes exceeds limit {}",
                ctx.limits.max_decompressed_bytes
            )));
        }
        if input.len() as u64 > ctx.limits.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "hybrid payload of {} bytes exceeds limit {}",
                input.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }

        let bpp = ctx.format.bytes_per_pixel() as usize;
        let sample_bytes = ctx.format.bytes_per_channel() as usize;
        let channels = ctx.format.channels() as usize;
        if bpp == 0 || sample_bytes == 0 || channels == 0 || expected_len % bpp != 0 {
            return Err(Error::corrupt(
                "hybrid tile expected length is not a whole number of pixels",
            ));
        }
        if input.len() < 2 {
            return Err(Error::corrupt("hybrid payload is shorter than its header"));
        }
        if input[0] != VERSION {
            return Err(Error::corrupt(format!(
                "hybrid payload version {} is not supported",
                input[0]
            )));
        }
        let kind = HybridKind::from_code(input[1])?;
        if kind != self.kind {
            return Err(Error::corrupt(format!(
                "hybrid payload holds {} but the codec decodes {}",
                kind.name(),
                self.kind.name()
            )));
        }
        let mut cursor = 2usize;

        // The predictor plan, when the pipeline runs the predictor.
        let (row_modes, layout) = if kind.has_predictor() {
            if cursor >= input.len() {
                return Err(Error::corrupt("hybrid payload is truncated"));
            }
            if input[cursor] != predictor::VERSION {
                return Err(Error::corrupt(format!(
                    "hybrid predictor version {} is not supported",
                    input[cursor]
                )));
            }
            cursor += 1;
            let (width, next) = get_uleb128(input, cursor)?;
            if width == 0 || width > MAX_TILE_WIDTH as u64 {
                return Err(Error::corrupt(format!(
                    "hybrid plan width {width} is out of range"
                )));
            }
            let width = width as usize;
            let pixels = expected_len / bpp;
            if pixels == 0 || pixels % width != 0 {
                return Err(Error::corrupt(
                    "hybrid plan width does not divide the tile's pixel count",
                ));
            }
            let height = pixels / width;
            cursor = next;
            let row_mode_end = cursor + height;
            if row_mode_end > input.len() {
                return Err(Error::corrupt(
                    "hybrid payload is shorter than its row modes",
                ));
            }
            let modes = input[cursor..row_mode_end].to_vec();
            for &mode in &modes {
                if mode > PREDICTOR_MAX {
                    return Err(Error::corrupt(format!(
                        "hybrid predictor mode {mode} is reserved"
                    )));
                }
            }
            cursor = row_mode_end;
            (
                Some(modes),
                Layout::new(width, height, channels, sample_bytes),
            )
        } else {
            (None, Layout::new(1, 1, channels, sample_bytes))
        };

        // The LZ77 stage's own header.
        let (params, output_len, next) = lz77::read_header(&input[cursor..])?;
        cursor += next;

        let expected_stage_len = match &row_modes {
            Some(_) => layout.total_samples() * sample_bytes,
            None => expected_len,
        };
        if output_len != expected_stage_len as u64 {
            return Err(Error::corrupt(format!(
                "hybrid lz77 stage declares {output_len} bytes but the stage holds {}",
                expected_stage_len
            )));
        }

        // The token stream.
        let decoded = if kind.has_entropy() {
            let mut source = lz77::EntropySource::new(&input[cursor..]);
            let bytes = lz77::decode_tokens(&mut source, &params, expected_stage_len)?;
            if source.overran() {
                return Err(Error::corrupt("hybrid entropy data is truncated"));
            }
            bytes
        } else {
            let mut source = lz77::RawSource::new(&input[cursor..]);
            lz77::decode_tokens(&mut source, &params, expected_stage_len)?
        };

        match row_modes {
            None => {
                // The LZ77 stage ran directly on the tile bytes.
                *out = decoded;
            }
            Some(modes) => {
                let residuals = bytes_to_residuals(&decoded, sample_bytes);
                let tile = predictor::reconstruct_tile(&residuals, &layout, &modes, expected_len)?;
                *out = tile;
            }
        }
        Ok(())
    }
}

impl TileCodec for HybridCodec {
    fn codec_id(&self) -> u8 {
        self.kind.codec_id()
    }

    fn name(&self) -> &'static str {
        self.kind.name()
    }

    fn encode(&self, input: &[u8], ctx: &EncodeContext, out: &mut Vec<u8>) -> Result<()> {
        self.encode_hybrid(input, ctx, out)
    }

    fn decode(
        &self,
        input: &[u8],
        expected_len: usize,
        ctx: &DecodeContext,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        self.decode_hybrid(input, expected_len, ctx, out)
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
    const GOLDEN_PAYLOAD_HEX: &str = "01020120010101050505050501000302010f0e10008006004080789a9e9fb51d285d45c0cb45b57519896be8c199910bc1cd2bbe3d7be5c6153236eaf907260a4287e1190000";

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

    fn sample_tile(width: usize, height: usize, format: PixelFormat) -> Vec<u8> {
        let bpp = format.bytes_per_pixel() as usize;
        let mut data = vec![0u8; width * height * bpp];
        for y in 0..height {
            for x in 0..width {
                let base = (y * width + x) * bpp;
                let value = ((x * 2 + y * 3) % 256) as u8;
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

    fn roundtrip(kind: HybridKind, format: PixelFormat, width: usize, data: &[u8]) -> Vec<u8> {
        let (enc, dec) = contexts(format, width as u16);
        let codec = HybridCodec::new(kind);
        let mut payload = Vec::new();
        codec.encode(data, &enc, &mut payload).unwrap();
        let mut out = Vec::new();
        codec.decode(&payload, data.len(), &dec, &mut out).unwrap();
        assert_eq!(out.len(), data.len());
        assert_eq!(out, data, "the decoder must reproduce the input exactly");
        payload
    }

    #[test]
    fn every_hybrid_roundtrips_every_format() {
        for kind in HybridKind::all() {
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
                roundtrip(kind, format, 17, &data);
            }
        }
    }

    #[test]
    fn every_hybrid_roundtrips_noisy_data() {
        let mut data = vec![0u8; 64 * 16];
        Rng::new(0xFEED).fill(&mut data);
        for kind in HybridKind::all() {
            roundtrip(kind, PixelFormat::rgba8(), 64, &data);
        }
    }

    #[test]
    fn every_hybrid_roundtrips_flat_data() {
        let data = vec![0x7Fu8; 32 * 16];
        for kind in HybridKind::all() {
            roundtrip(kind, PixelFormat::gray8(), 32, &data);
        }
    }

    #[test]
    fn the_hybrids_produce_different_payloads() {
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        let payloads: Vec<Vec<u8>> = HybridKind::all()
            .iter()
            .map(|&kind| roundtrip(kind, PixelFormat::rgb8(), 32, &data))
            .collect();
        for i in 0..payloads.len() {
            for j in (i + 1)..payloads.len() {
                assert_ne!(
                    payloads[i], payloads[j],
                    "hybrids {} and {} produced the same payload",
                    i, j
                );
            }
        }
    }

    #[test]
    fn output_is_deterministic() {
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        for kind in HybridKind::all() {
            let first = roundtrip(kind, PixelFormat::rgb8(), 32, &data);
            let second = roundtrip(kind, PixelFormat::rgb8(), 32, &data);
            assert_eq!(first, second, "the encoder must be deterministic");
        }
    }

    #[test]
    fn a_bad_version_is_rejected() {
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        let (_, dec) = contexts(PixelFormat::rgb8(), 32);
        let codec = HybridCodec::new(HybridKind::Lz77ThenEntropy);
        let mut payload = Vec::new();
        codec
            .encode(&data, &contexts(PixelFormat::rgb8(), 32).0, &mut payload)
            .unwrap();
        payload[0] = 55;
        let mut out = Vec::new();
        let err = codec
            .decode(&payload, data.len(), &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn a_mismatched_kind_is_rejected() {
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        let (_, dec) = contexts(PixelFormat::rgb8(), 32);
        let codec = HybridCodec::new(HybridKind::Lz77ThenEntropy);
        let mut payload = Vec::new();
        codec
            .encode(&data, &contexts(PixelFormat::rgb8(), 32).0, &mut payload)
            .unwrap();
        payload[1] = HybridKind::PredictorThenLz77.code();
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
        let codec = HybridCodec::new(HybridKind::PredictorThenLz77ThenEntropy);
        let mut payload = Vec::new();
        codec
            .encode(&data, &contexts(PixelFormat::rgb8(), 32).0, &mut payload)
            .unwrap();
        let truncated = &payload[..payload.len() - 5];
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
        let codec = HybridCodec::new(HybridKind::PredictorThenLz77);
        let mut payload = Vec::new();
        codec
            .encode(&data, &contexts(PixelFormat::rgb8(), 32).0, &mut payload)
            .unwrap();
        let mut out = Vec::new();
        let err = codec
            .decode(&payload, data.len() - 3, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn the_residual_fold_is_exact() {
        for sample_bytes in [1usize, 2] {
            let half = 1i64 << (sample_bytes * 8 - 1);
            let mask = (1u32 << (sample_bytes * 8)) - 1;
            let extremes = [-half, -half + 1, -1, 0, 1, half - 1];
            let residuals: Vec<i32> = extremes.iter().map(|&v| v as i32).collect();
            let bytes = residuals_to_bytes(&residuals, sample_bytes);
            assert_eq!(bytes.len(), residuals.len() * sample_bytes);
            let restored = bytes_to_residuals(&bytes, sample_bytes);
            assert_eq!(restored, residuals);
            assert!(bytes.iter().all(|&b| u32::from(b) <= mask));
        }
    }

    #[test]
    fn golden_vector_is_stable() {
        // A fixed input with a fixed pipeline must always produce the same
        // bytes. The expected value is pinned so a change to the bitstream is
        // caught by the test suite.
        let data = sample_tile(32, 8, PixelFormat::rgb8());
        let payload = roundtrip(
            HybridKind::PredictorThenLz77ThenEntropy,
            PixelFormat::rgb8(),
            32,
            &data,
        );
        let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, GOLDEN_PAYLOAD_HEX, "the golden hybrid payload changed");
    }
}
