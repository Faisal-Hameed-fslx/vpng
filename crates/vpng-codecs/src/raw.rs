//! Codec A — Raw. The correctness baseline and the always-available fallback.

use vpng_core::{Error, Result};

use crate::{DecodeContext, EncodeContext, TileCodec, CODEC_RAW};

/// Stateless raw codec. Its payload is the tile's canonical bytes verbatim.
#[derive(Debug, Clone, Copy, Default)]
pub struct RawCodec;

pub static RAW_CODEC: RawCodec = RawCodec;

impl TileCodec for RawCodec {
    fn codec_id(&self) -> u8 {
        CODEC_RAW
    }

    fn name(&self) -> &'static str {
        "raw"
    }

    fn encode(&self, input: &[u8], ctx: &EncodeContext, out: &mut Vec<u8>) -> Result<()> {
        if input.len() as u64 > ctx.limits.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "raw tile of {} bytes exceeds limit {}",
                input.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }
        out.clear();
        out.extend_from_slice(input);
        Ok(())
    }

    fn decode(
        &self,
        input: &[u8],
        expected_len: usize,
        ctx: &DecodeContext,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        if expected_len as u64 > ctx.limits.max_decompressed_bytes {
            return Err(Error::resource_limit(format!(
                "raw tile decode of {expected_len} bytes exceeds limit {}",
                ctx.limits.max_decompressed_bytes
            )));
        }
        if input.len() != expected_len {
            return Err(Error::corrupt(format!(
                "raw tile length mismatch: payload is {} bytes, expected {expected_len}",
                input.len()
            )));
        }
        out.clear();
        out.extend_from_slice(input);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_core::{Limits, PixelFormat};

    fn ctx() -> (EncodeContext, DecodeContext) {
        let limits = Limits::default();
        let format = PixelFormat::rgba8();
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

    #[test]
    fn identity_roundtrip() {
        let (enc_ctx, dec_ctx) = ctx();
        let input: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        let mut payload = Vec::new();
        RAW_CODEC.encode(&input, &enc_ctx, &mut payload).unwrap();
        assert_eq!(payload, input);

        let mut decoded = Vec::new();
        RAW_CODEC
            .decode(&payload, input.len(), &dec_ctx, &mut decoded)
            .unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn length_mismatch_is_corrupt() {
        let (_, dec_ctx) = ctx();
        let mut out = Vec::new();
        let err = RAW_CODEC
            .decode(&[0u8; 3], 4, &dec_ctx, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), vpng_core::ErrorKind::CorruptFile);
    }

    #[test]
    fn respects_decompressed_limit() {
        let limits = Limits {
            max_decompressed_bytes: 8,
            ..Limits::default()
        };
        let dec_ctx = DecodeContext {
            limits,
            format: PixelFormat::rgba8(),
        };
        let mut out = Vec::new();
        let err = RAW_CODEC
            .decode(&[0u8; 16], 16, &dec_ctx, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), vpng_core::ErrorKind::ResourceLimit);
    }
}
