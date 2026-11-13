//! Codec B — per-tile palette.
//!
//! The palette is built from the tile's own pixels only (ADR-0004: no codec may
//! reference bytes outside its tile). Palette entries are the exact canonical
//! pixel bytes, so alpha and every channel value survive verbatim; indices are
//! packed at the smallest bit width that can address the palette.
//!
//! Payload layout (little-endian; see `docs/VPNG_SPEC_0_1.md` §5.2 and ADR-0009):
//!
//! ```text
//! offset 0   u8   index_bits_code  (0=>1, 1=>2, 2=>4, 3=>8); bits 2..7 reserved, zero
//! offset 1   u8   entry_count - 1  (so 1..=256 entries)
//! offset 2   ..   entry_count * bytes_per_pixel  palette entries
//! ...        ..   ceil(pixel_count * index_bits / 8)  packed indices, MSB-first
//! ```
//!
//! The format is canonical: the index width must be the smallest width that can
//! address `entry_count`, every index must be `< entry_count`, the payload length
//! must be exact, and the unused padding bits of the final byte must be zero.
//! A decoder rejects anything else, so one image has exactly one palette payload.

use std::collections::HashMap;

use vpng_core::{Error, Result};

use crate::{DecodeContext, EncodeContext, TileCodec, CODEC_PALETTE};

/// Index widths the format permits, in increasing order. An encoder picks the
/// first one that can address every entry.
pub const INDEX_WIDTHS: [u8; 4] = [1, 2, 4, 8];

/// Maximum number of palette entries a single payload may carry.
pub const MAX_ENTRIES: usize = 256;

/// Stateless per-tile palette codec.
#[derive(Debug, Clone, Copy, Default)]
pub struct PaletteCodec;

pub static PALETTE_CODEC: PaletteCodec = PaletteCodec;

/// Width code stored in the first payload byte for each supported bit width.
fn width_code(bits: u8) -> u8 {
    match bits {
        1 => 0,
        2 => 1,
        4 => 2,
        8 => 3,
        other => unreachable!("unsupported palette index width {other}"),
    }
}

/// Inverse of [`width_code`]. `None` for a reserved code.
fn bits_for_code(code: u8) -> Option<u8> {
    match code {
        0 => Some(1),
        1 => Some(2),
        2 => Some(4),
        3 => Some(8),
        _ => None,
    }
}

/// The smallest permitted width that can address `entry_count` entries.
fn smallest_width(entry_count: usize) -> u8 {
    for &width in INDEX_WIDTHS.iter() {
        if entry_count <= (1usize << width) {
            return width;
        }
    }
    8
}

/// Map one pixel's bytes to a unique key. `bytes_per_pixel` is at most 8, so the
/// key is exact (no truncation) for every v0.1 format.
fn pixel_key(pixel: &[u8]) -> u64 {
    let mut key = 0u64;
    for &byte in pixel {
        key = (key << 8) | u64::from(byte);
    }
    key
}

fn pack_indices(indices: &[u16], bits: u8, out: &mut Vec<u8>) {
    let mut acc: u32 = 0;
    let mut pending: u32 = 0;
    for &index in indices {
        acc = (acc << bits) | u32::from(index);
        pending += u32::from(bits);
        while pending >= 8 {
            pending -= 8;
            out.push((acc >> pending) as u8);
        }
        acc &= (1u32 << pending) - 1;
    }
    if pending > 0 {
        // MSB-first: the remaining low bits of `acc` move to the top of the
        // final byte; the unused low bits stay zero.
        out.push(((acc << (8 - pending)) & 0xff) as u8);
    }
}

impl PaletteCodec {
    /// Build a palette payload for `input`.
    ///
    /// Returns `Ok(true)` when a payload was written to `out`, and `Ok(false)`
    /// when a palette is not applicable to this tile: more than 256 distinct
    /// pixel values, an empty tile, or a payload that would be larger than the
    /// configured per-tile payload limit. A selector must then fall back to
    /// another codec; [`TileCodec::encode`] turns `false` into
    /// [`vpng_core::ErrorKind::UnsupportedFeature`].
    pub fn try_encode(&self, input: &[u8], ctx: &EncodeContext, out: &mut Vec<u8>) -> Result<bool> {
        let bpp = ctx.format.bytes_per_pixel() as usize;
        if bpp == 0 || input.len() % bpp != 0 {
            return Err(Error::internal(
                "palette input is not a whole number of pixels",
            ));
        }
        if input.len() as u64 > ctx.limits.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "palette tile of {} bytes exceeds limit {}",
                input.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }
        let pixel_count = input.len() / bpp;
        if pixel_count == 0 {
            out.clear();
            return Ok(false);
        }
        // From here on every path leaves `out` in a defined state.
        out.clear();

        // Assign indices in first-appearance order so the payload is
        // deterministic regardless of hashing.
        let mut index_of: HashMap<u64, u16> = HashMap::new();
        let mut entries: Vec<u8> = Vec::new();
        let mut indices: Vec<u16> = Vec::with_capacity(pixel_count);
        for pixel in input.chunks_exact(bpp) {
            let key = pixel_key(pixel);
            let index = match index_of.get(&key) {
                Some(&existing) => existing,
                None => {
                    if entries.len() / bpp >= MAX_ENTRIES {
                        // A 257th distinct value: not a palette case.
                        out.clear();
                        return Ok(false);
                    }
                    let assigned = (entries.len() / bpp) as u16;
                    index_of.insert(key, assigned);
                    entries.extend_from_slice(pixel);
                    assigned
                }
            };
            indices.push(index);
        }

        let entry_count = entries.len() / bpp;
        let bits = smallest_width(entry_count);

        out.reserve(2 + entries.len() + pixel_count);
        out.push(width_code(bits));
        out.push((entry_count - 1) as u8);
        out.extend_from_slice(&entries);
        pack_indices(&indices, bits, out);

        if out.len() as u64 > ctx.limits.max_tile_payload_bytes {
            // A legal payload cannot be emitted; let the caller try another codec.
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
                "palette tile decode of {expected_len} bytes exceeds limit {}",
                ctx.limits.max_decompressed_bytes
            )));
        }
        if input.len() as u64 > ctx.limits.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "palette payload of {} bytes exceeds limit {}",
                input.len(),
                ctx.limits.max_tile_payload_bytes
            )));
        }
        if bpp == 0 || expected_len % bpp != 0 {
            return Err(Error::corrupt(
                "palette tile expected length is not a whole number of pixels",
            ));
        }
        let pixel_count = expected_len / bpp;

        if input.len() < 2 {
            return Err(Error::corrupt("palette payload is shorter than its header"));
        }
        let header = input[0];
        if (header & 0b1111_1100) != 0 {
            return Err(Error::corrupt(
                "palette payload has reserved header bits set",
            ));
        }
        let bits = bits_for_code(header & 0b11)
            .ok_or_else(|| Error::corrupt("palette payload has a reserved index width code"))?;
        let entry_count = usize::from(input[1]) + 1;

        // Canonical form: the declared width must be exactly the smallest width
        // that addresses every entry, and it must be able to address them.
        if entry_count > (1usize << bits) || bits != smallest_width(entry_count) {
            return Err(Error::corrupt(
                "palette index width is not the minimal width for its entry count",
            ));
        }

        let palette_len = entry_count * bpp;
        let packed_len = (pixel_count as u64 * u64::from(bits)).div_ceil(8) as usize;
        let expected_payload = 2 + palette_len + packed_len;
        if input.len() != expected_payload {
            return Err(Error::corrupt(format!(
                "palette payload length mismatch: {} bytes, expected {expected_payload}",
                input.len()
            )));
        }

        let palette = &input[2..2 + palette_len];
        let packed = &input[2 + palette_len..];

        out.clear();
        out.reserve(expected_len);

        let mask = (1u32 << bits) - 1;
        let mut acc: u32 = 0;
        let mut available: u32 = 0;
        let mut cursor = 0usize;
        for _ in 0..pixel_count {
            while available < u32::from(bits) {
                if cursor >= packed.len() {
                    return Err(Error::corrupt("palette index stream is truncated"));
                }
                acc = (acc << 8) | u32::from(packed[cursor]);
                cursor += 1;
                available += 8;
            }
            available -= u32::from(bits);
            let index = ((acc >> available) & mask) as usize;
            acc &= (1u32 << available) - 1;
            if index >= entry_count {
                return Err(Error::corrupt(format!(
                    "palette index {index} is out of range for {entry_count} entries"
                )));
            }
            let start = index * bpp;
            out.extend_from_slice(&palette[start..start + bpp]);
        }

        if cursor != packed.len() {
            return Err(Error::corrupt("palette index stream has trailing bytes"));
        }
        if acc != 0 {
            return Err(Error::corrupt("palette padding bits must be zero"));
        }
        Ok(())
    }
}

impl TileCodec for PaletteCodec {
    fn codec_id(&self) -> u8 {
        CODEC_PALETTE
    }

    fn name(&self) -> &'static str {
        "palette"
    }

    fn encode(&self, input: &[u8], ctx: &EncodeContext, out: &mut Vec<u8>) -> Result<()> {
        if self.try_encode(input, ctx, out)? {
            Ok(())
        } else {
            Err(Error::unsupported(
                "palette codec cannot encode this tile (more than 256 distinct pixel values, \
                 an empty tile, or a payload above the configured size limit)",
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
            PALETTE_CODEC.try_encode(pixels, &enc, &mut out).unwrap(),
            "palette should apply"
        );
        out
    }

    fn decode_ok(format: PixelFormat, payload: &[u8], expected_len: usize) -> Vec<u8> {
        let (_, dec) = contexts(format);
        let mut out = Vec::new();
        PALETTE_CODEC
            .decode(payload, expected_len, &dec, &mut out)
            .unwrap();
        out
    }

    #[test]
    fn gray8_two_colors_golden_vector() {
        // 8 gray pixels alternating 0xAA / 0x55 => two entries, 1-bit indices.
        // Entry order is first appearance: 0xAA => index 0, 0x55 => index 1.
        // Bits MSB-first for pixels 0..7: 0 1 0 1 0 1 0 1 = 0b0101_0101 = 0x55.
        let pixels = [0xAAu8, 0x55, 0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55];
        let payload = encode_ok(PixelFormat::gray8(), &pixels);
        assert_eq!(payload, vec![0x00, 0x01, 0xAA, 0x55, 0x55]);
        assert_eq!(
            decode_ok(PixelFormat::gray8(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn gray8_single_color_uses_one_entry() {
        // One entry => 1-bit indices, all zero. 3 pixels pack into one byte.
        let pixels = [0x7Fu8, 0x7F, 0x7F];
        let payload = encode_ok(PixelFormat::gray8(), &pixels);
        assert_eq!(payload, vec![0x00, 0x00, 0x7F, 0x00]);
        assert_eq!(
            decode_ok(PixelFormat::gray8(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn rgba8_three_colors_golden_vector() {
        // bpp = 4 => entry size 4. Three entries => 2-bit indices.
        let a = [0x01u8, 0x02, 0x03, 0x04];
        let b = [0x11, 0x12, 0x13, 0x14];
        let c = [0x21, 0x22, 0x23, 0x24];
        let pixels: Vec<u8> = [a, b, c, c].concat();
        let payload = encode_ok(PixelFormat::rgba8(), &pixels);

        // header: width code 1 (2 bits), entry count - 1 = 2
        // indices 0,1,2,2 => 00 01 10 10 => byte 0b0001_1010 = 0x1A
        let mut expected = vec![0x01, 0x02];
        expected.extend_from_slice(&a);
        expected.extend_from_slice(&b);
        expected.extend_from_slice(&c);
        expected.push(0x1A);
        assert_eq!(payload, expected);
        assert_eq!(
            decode_ok(PixelFormat::rgba8(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn rgba16_preserves_every_byte() {
        // bpp = 8, 16-bit channels stored little-endian. Two colors => 1 bit.
        let a: [u8; 8] = [0x34, 0x12, 0x78, 0x56, 0xBC, 0x9A, 0xF0, 0xDE];
        let b: [u8; 8] = [0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0xFF, 0x00];
        let pixels: Vec<u8> = [a, b, a, b].concat();
        let payload = encode_ok(PixelFormat::rgba16(), &pixels);
        // Indices 0,1,0,1 => 0b0101_0000 = 0x50.
        let mut expected = vec![0x00, 0x01];
        expected.extend_from_slice(&a);
        expected.extend_from_slice(&b);
        expected.push(0x50);
        assert_eq!(payload, expected);
        assert_eq!(
            decode_ok(PixelFormat::rgba16(), &payload, pixels.len()),
            pixels
        );
    }

    #[test]
    fn sixteen_and_256_color_widths() {
        // 16 distinct gray values => 4-bit indices; 256 => 8-bit indices.
        let sixteen: Vec<u8> = (0..16u8).collect();
        let payload = encode_ok(PixelFormat::gray8(), &sixteen);
        assert_eq!(payload[0], 2, "16 colors need 4-bit indices");
        assert_eq!(payload[1], 15, "16 entries => count - 1 = 15");
        assert_eq!(
            decode_ok(PixelFormat::gray8(), &payload, sixteen.len()),
            sixteen
        );

        let many: Vec<u8> = (0..=255u8).collect();
        let payload = encode_ok(PixelFormat::gray8(), &many);
        assert_eq!(payload[0], 3, "256 colors need 8-bit indices");
        assert_eq!(payload[1], 255, "256 entries => count - 1 = 255");
        assert_eq!(decode_ok(PixelFormat::gray8(), &payload, many.len()), many);
    }

    #[test]
    fn more_than_256_colors_is_not_applicable() {
        // 257 distinct rgba8 values cannot be palettised.
        let mut pixels = Vec::new();
        for i in 0..257u32 {
            pixels.extend_from_slice(&[i as u8, (i >> 8) as u8, 0, 0xFF]);
        }
        let (enc, _) = contexts(PixelFormat::rgba8());
        let mut out = vec![0xFFu8; 8];
        assert!(!PALETTE_CODEC.try_encode(&pixels, &enc, &mut out).unwrap());
        assert_eq!(out.len(), 0, "a rejected tile must not leave output behind");

        let err = PALETTE_CODEC.encode(&pixels, &enc, &mut out).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
    }

    #[test]
    fn roundtrips_every_v0_1_format() {
        // Kept local (no `vpng-testkit` dev-dependency, to avoid a dependency
        // cycle): the same eight formats the conformance suite covers.
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
            // A few distinct colors, so the palette path is exercised.
            let colors: Vec<Vec<u8>> = (0..3u8)
                .map(|i| vec![i.wrapping_mul(37).wrapping_add(5); bpp])
                .collect();
            let mut pixels = Vec::new();
            for i in 0..100usize {
                pixels.extend_from_slice(&colors[i % colors.len()]);
            }
            let payload = encode_ok(format, &pixels);
            assert_eq!(decode_ok(format, &payload, pixels.len()), pixels);
        }
    }

    #[test]
    fn rejects_non_minimal_width() {
        // Two entries encoded with a 4-bit width is non-canonical.
        let format = PixelFormat::gray8();
        let payload = vec![2u8, 1, 0xAA, 0x55, 0x00];
        let (_, dec) = contexts(format);
        let mut out = Vec::new();
        let err = PALETTE_CODEC
            .decode(&payload, 4, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn rejects_out_of_range_index() {
        let format = PixelFormat::gray8();
        let (_, dec) = contexts(format);

        // Sanity: two entries and 4 one-bit indices (all 1) is valid.
        let valid = vec![0u8, 1, 0xAA, 0x55, 0b1111_0000];
        let mut out = Vec::new();
        PALETTE_CODEC.decode(&valid, 4, &dec, &mut out).unwrap();
        assert_eq!(out.len(), 4);

        // One entry, but an 8-bit index of 5 is out of range.
        let bad = vec![3u8, 0, 0xAA, 0x05];
        let err = PALETTE_CODEC.decode(&bad, 1, &dec, &mut out).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn rejects_reserved_header_bits() {
        let format = PixelFormat::gray8();
        let payload = vec![0b0000_0100u8, 0, 0xAA, 0x00];
        let (_, dec) = contexts(format);
        let mut out = Vec::new();
        let err = PALETTE_CODEC
            .decode(&payload, 2, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn rejects_nonzero_padding() {
        let format = PixelFormat::gray8();
        // Two entries, 3 one-bit pixels: bits 0,0,0 then padding must be zero.
        // Setting a padding bit must be rejected.
        let payload = vec![0u8, 1, 0xAA, 0x55, 0b0001_0000];
        let (_, dec) = contexts(format);
        let mut out = Vec::new();
        let err = PALETTE_CODEC
            .decode(&payload, 3, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }

    #[test]
    fn rejects_length_mismatch() {
        let format = PixelFormat::gray8();
        let payload = vec![0u8, 1, 0xAA, 0x55, 0x00, 0x00];
        let (_, dec) = contexts(format);
        let mut out = Vec::new();
        let err = PALETTE_CODEC
            .decode(&payload, 4, &dec, &mut out)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::CorruptFile);
    }
}
