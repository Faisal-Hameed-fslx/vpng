//! Codec C — run-length.
//!
//! Runs are measured in whole pixels (symbols), never in bytes, so a run always
//! repeats one exact pixel tuple and no channel or alpha byte can drift. The
//! stream is self-delimiting: every token begins with a control byte, so symbols
//! are never confused with markers and no escaping is needed.
//!
//! Payload layout (see `docs/VPNG_SPEC_0_1.md` §5.3 and ADR-0010):
//!
//! ```text
//! control 0x00..=0x7F   literal run: count = control + 1 (1..=128)
//!                       followed by count symbols, verbatim
//! control 0x81..=0xFF   repeat run:  count = (control & 0x7F) + 1 (2..=128)
//!                       followed by one symbol, repeated count times
//! control 0x80          malformed (a repeat run of length 1)
//! ```
//!
//! Symbols are `bytes_per_pixel` wide. Both token kinds cap at [`MAX_RUN`] = 128
//! symbols, so a longer run is split across several tokens.
//!
//! The encoder is deterministic: it emits one token per maximal run of length
//! ≥ 2, and groups adjacent single-symbol runs (up to [`MAX_RUN`]) into literal
//! tokens. The decoder accepts any well-formed stream, not only that canonical
//! one; malformed streams (a length-1 repeat, truncation, an overrun past the
//! expected pixel count, or a total that does not match the tile) are rejected.

use vpng_core::{Error, Result};

use crate::{DecodeContext, EncodeContext, TileCodec, CODEC_RLE};

/// Maximum symbols a single token may represent.
pub const MAX_RUN: usize = 128;

/// Set in a control byte to mark a repeat run.
const REPEAT_FLAG: u8 = 0x80;

/// Stateless per-tile run-length codec.
#[derive(Debug, Clone, Copy, Default)]
pub struct RleCodec;

pub static RLE_CODEC: RleCodec = RleCodec;

/// Control byte for a literal run of `count` symbols (1..=128).
fn literal_control(count: usize) -> u8 {
    debug_assert!((1..=MAX_RUN).contains(&count));
    (count - 1) as u8
}

/// Control byte for a repeat run of `count` symbols (2..=128).
fn repeat_control(count: usize) -> u8 {
    debug_assert!((2..=MAX_RUN).contains(&count));
    REPEAT_FLAG | (count - 1) as u8
}

/// Length of the maximal run of equal symbols starting at symbol `start`.
fn run_length(input: &[u8], bpp: usize, start: usize, total: usize) -> usize {
    let base = start * bpp;
    let mut run = 1;
    while start + run < total {
        let other = (start + run) * bpp;
        if input[other..other + bpp] != input[base..base + bpp] {
            break;
        }
        run += 1;
    }
    run
}

/// Append a literal token covering symbols `[start, end)`.
fn emit_literal(input: &[u8], bpp: usize, start: usize, end: usize, out: &mut Vec<u8>) {
    let count = end - start;
    debug_assert!((1..=MAX_RUN).contains(&count));
    out.push(literal_control(count));
    out.extend_from_slice(&input[start * bpp..end * bpp]);
}

/// Append repeat tokens covering `run_length` copies of the symbol at `start`,
/// split into chunks of at most [`MAX_RUN`] and never leaving a chunk of one.
fn emit_repeats(input: &[u8], bpp: usize, start: usize, run_length: usize, out: &mut Vec<u8>) {
    let base = start * bpp;
    let symbol = &input[base..base + bpp];
    let mut remaining = run_length;
    while remaining > 0 {
        let mut chunk = remaining.min(MAX_RUN);
        if remaining - chunk == 1 {
            // Never emit a trailing chunk of a single symbol; fold it into this
            // one so every repeat token has length >= 2.
            chunk -= 1;
        }
        out.push(repeat_control(chunk));
        out.extend_from_slice(symbol);
        remaining -= chunk;
    }
}

impl RleCodec {
    /// Build a run-length payload for `input`.
    ///
    /// Returns `Ok(true)` when a payload was written to `out`, and `Ok(false)`
    /// when run-length coding is not applicable: an empty tile, or a payload
    /// that would be larger than the configured per-tile payload limit. A
    /// selector then falls back to another codec; [`TileCodec::encode`] turns
    /// `false` into [`vpng_core::ErrorKind::UnsupportedFeature`].
    pub fn try_encode(&self, input: &[u8], ctx: &EncodeContext, out: &mut Vec<u8>) -> Result<bool> {
        let bpp = ctx.format.bytes_per_pixel() as usize;
        if bpp == 0 || input.len() % bpp != 0 {
            return Err(Error::internal(
                "run-length input is not a whole number of pixels",
            ));
        }
        if input.len() as u64 > ctx.limits.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "run-length tile of {} bytes exceeds limit {}",
                input.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }
        let total = input.len() / bpp;
        out.clear();
        if total == 0 {
            return Ok(false);
        }
        // A tight upper bound on the payload; avoids repeated reallocation. The
        // final size is checked against the limit before returning.
        out.reserve(2 * bpp + total.min(1 << 16));

        let mut index = 0usize;
        let mut literal_start: Option<usize> = None;
        while index < total {
            let run = run_length(input, bpp, index, total);
            if run >= 2 {
                if let Some(start) = literal_start.take() {
                    emit_literal(input, bpp, start, index, out);
                }
                emit_repeats(input, bpp, index, run, out);
                index += run;
            } else {
                if literal_start.is_none() {
                    literal_start = Some(index);
                }
                index += 1;
                if index - literal_start.expect("literal run started") == MAX_RUN {
                    let start = literal_start.take().expect("literal run started");
                    emit_literal(input, bpp, start, index, out);
                }
            }
        }
        if let Some(start) = literal_start.take() {
            emit_literal(input, bpp, start, total, out);
        }

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
        if expected_len as u64 > ctx.limits.max_decompressed_bytes {
            return Err(Error::resource_limit(format!(
                "run-length tile decode of {expected_len} bytes exceeds limit {}",
                ctx.limits.max_decompressed_bytes
            )));
        }
        if input.len() as u64 > ctx.limits.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "run-length payload of {} bytes exceeds limit {}",
                input.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }
        if bpp == 0 || expected_len % bpp != 0 {
            return Err(Error::corrupt(
                "run-length tile expected length is not a whole number of pixels",
            ));
        }
        let total = expected_len / bpp;

        out.clear();
        out.reserve(expected_len);

        let mut cursor = 0usize;
        let mut produced = 0usize;
        while cursor < input.len() {
            let control = input[cursor];
            cursor += 1;
            if control & REPEAT_FLAG == 0 {
                let count = usize::from(control) + 1;
                let bytes = count * bpp;
                if cursor + bytes > input.len() {
                    return Err(Error::corrupt("run-length literal run is truncated"));
                }
                if produced + count > total {
                    return Err(Error::corrupt(
                        "run-length literal run exceeds the expected pixel count",
                    ));
                }
                out.extend_from_slice(&input[cursor..cursor + bytes]);
                cursor += bytes;
                produced += count;
            } else {
                let count = usize::from(control & 0x7F) + 1;
                if count < 2 {
                    return Err(Error::corrupt("run-length repeat run has length 1"));
                }
                if cursor + bpp > input.len() {
                    return Err(Error::corrupt("run-length repeat run is truncated"));
                }
                if produced + count > total {
                    return Err(Error::corrupt(
                        "run-length repeat run exceeds the expected pixel count",
                    ));
                }
                let symbol = &input[cursor..cursor + bpp];
                cursor += bpp;
                for _ in 0..count {
                    out.extend_from_slice(symbol);
                }
                produced += count;
            }
        }

        if produced != total {
            return Err(Error::corrupt(format!(
                "run-length stream produced {produced} symbols but the tile needs {total}"
            )));
        }
        Ok(())
    }
}

impl TileCodec for RleCodec {
    fn codec_id(&self) -> u8 {
        CODEC_RLE
    }

    fn name(&self) -> &'static str {
        "rle"
    }

    fn encode(&self, input: &[u8], ctx: &EncodeContext, out: &mut Vec<u8>) -> Result<()> {
        if self.try_encode(input, ctx, out)? {
            Ok(())
        } else {
            Err(Error::unsupported(
                "run-length codec cannot encode this tile (an empty tile, or a payload above the \
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

    fn encode_ok(format: PixelFormat, pixels: &[u8]) -> Vec<u8> {
        let (enc, _) = contexts(format);
        let mut out = Vec::new();
        assert!(
            RLE_CODEC.try_encode(pixels, &enc, &mut out).unwrap(),
            "rle should apply"
        );
        out
    }

    fn decode_ok(format: PixelFormat, payload: &[u8], expected_len: usize) -> Vec<u8> {
        let (_, dec) = contexts(format);
        let mut out = Vec::new();
        RLE_CODEC
            .decode(payload, expected_len, &dec, &mut out)
            .unwrap();
        out
    }

    #[test]
    fn alternating_pixels_golden_vector() {
        // No two neighbours match, so this is one literal run of four symbols.
        let pixels = [0x01u8, 0x02, 0x01, 0x02];
        let payload = encode_ok(PixelFormat::gray8(), &pixels);
        assert_eq!(payload, vec![0x03, 0x01, 0x02, 0x01, 0x02]);
        assert_eq!(
            decode_ok(PixelFormat::gray8(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn long_run_splits_into_128_symbol_tokens() {
        // 300 = 128 + 128 + 44; each token is control + one symbol byte.
        let pixels = vec![0x9Cu8; 300];
        let payload = encode_ok(PixelFormat::gray8(), &pixels);
        assert_eq!(
            payload,
            vec![0xFF, 0x9C, 0xFF, 0x9C, 0xAB, 0x9C],
            "two full tokens then a 44-symbol token"
        );
        assert_eq!(
            decode_ok(PixelFormat::gray8(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn chunking_never_leaves_a_single_symbol() {
        // 129 = 127 + 2, not 128 + 1 (which would need a length-1 repeat).
        let pixels = vec![0x5Au8; 129];
        let payload = encode_ok(PixelFormat::gray8(), &pixels);
        assert_eq!(payload, vec![0xFE, 0x5A, 0x81, 0x5A]);
        assert_eq!(
            decode_ok(PixelFormat::gray8(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn short_runs_mix_repeats_and_a_literal() {
        // A A B C C  =>  repeat(2) A, literal(1) B, repeat(2) C
        let pixels = [0x0Au8, 0x0A, 0x0B, 0x0C, 0x0C];
        let payload = encode_ok(PixelFormat::gray8(), &pixels);
        assert_eq!(payload, vec![0x81, 0x0A, 0x00, 0x0B, 0x81, 0x0C]);
        assert_eq!(
            decode_ok(PixelFormat::gray8(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn multi_byte_symbols_repeat_as_whole_pixels() {
        let p = [0x10u8, 0x20, 0x30, 0x40];
        let q = [0x50u8, 0x60, 0x70, 0x80];
        let pixels: Vec<u8> = [p, p, q].concat();
        let payload = encode_ok(PixelFormat::rgba8(), &pixels);
        let mut expected = vec![0x81];
        expected.extend_from_slice(&p);
        expected.push(0x00);
        expected.extend_from_slice(&q);
        assert_eq!(payload, expected);
        assert_eq!(
            decode_ok(PixelFormat::rgba8(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn rgba16_long_run_uses_eight_byte_symbols() {
        let pixel = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let pixels: Vec<u8> = pixel.iter().copied().cycle().take(8 * 200).collect();
        let payload = encode_ok(PixelFormat::rgba16(), &pixels);
        // 200 = 128 + 72; symbol is emitted once per token.
        let mut expected = vec![0xFF];
        expected.extend_from_slice(&pixel);
        expected.push(0xC7); // 72 - 1 = 71 = 0x47, plus the repeat flag = 0xC7
        expected.extend_from_slice(&pixel);
        assert_eq!(payload, expected);
        assert_eq!(
            decode_ok(PixelFormat::rgba16(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn roundtrips_every_v0_1_format_and_pattern() {
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
            for pattern in 0..4 {
                let pixels: Vec<u8> = (0..600usize)
                    .flat_map(|i| {
                        let value = match pattern {
                            0 => 0x11,           // one long run
                            1 => (i % 2) as u8,  // alternating (literals)
                            2 => (i / 7) as u8,  // runs of 7
                            _ => (i * 31) as u8, // pseudo-random-ish
                        };
                        (0..bpp).map(move |c| value.wrapping_add(c as u8))
                    })
                    .collect();
                let payload = encode_ok(format, &pixels);
                assert_eq!(decode_ok(format, &payload, pixels.len()), pixels);
            }
        }
    }

    #[test]
    fn empty_tile_is_not_applicable() {
        let (enc, _) = contexts(PixelFormat::gray8());
        let mut out = vec![0xFFu8; 4];
        assert!(!RLE_CODEC.try_encode(&[], &enc, &mut out).unwrap());
        assert_eq!(out.len(), 0, "a rejected tile must not leave output behind");
    }

    #[test]
    fn rejects_repeat_of_length_one() {
        let (_, dec) = contexts(PixelFormat::gray8());
        let mut out = Vec::new();
        let err = RLE_CODEC
            .decode(&[0x80, 0x9C], 1, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn rejects_truncated_literal() {
        let (_, dec) = contexts(PixelFormat::gray8());
        let mut out = Vec::new();
        let err = RLE_CODEC
            .decode(&[0x03, 0x01], 4, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn rejects_truncated_repeat() {
        let (_, dec) = contexts(PixelFormat::rgba8());
        let mut out = Vec::new();
        let err = RLE_CODEC
            .decode(&[0x81, 0x00, 0x11, 0x22], 2, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn rejects_overrun_past_expected_count() {
        let (_, dec) = contexts(PixelFormat::gray8());
        let mut out = Vec::new();
        let err = RLE_CODEC
            .decode(&[0x81, 0xAA], 1, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn rejects_short_output() {
        let (_, dec) = contexts(PixelFormat::gray8());
        let mut out = Vec::new();
        let err = RLE_CODEC
            .decode(&[0x00, 0x01], 2, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn rejects_empty_payload_for_a_nonempty_tile() {
        let (_, dec) = contexts(PixelFormat::gray8());
        let mut out = Vec::new();
        let err = RLE_CODEC.decode(&[], 4, &dec, &mut out).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }
}
