//! GPU block payloads for VPNG.
//!
//! A VPNG file's **lossless** payload is what reconstructs the image exactly.
//! This crate builds the other thing: a **GPU payload**, an optional sibling
//! that holds the same picture in the block format a GPU samples as a texture
//! without decompressing anything.
//!
//! Two block codecs are implemented:
//!
//! * [`bc1`] — BC1 (DXT1), eight bytes per 4×4 block, three colour channels;
//! * [`bc4`] — BC4 (ATI1N), eight bytes per 4×4 block, one channel.
//!
//! **Both are lossy.** Nothing here may be used to reconstruct an image
//! exactly, and nothing here is ever read by an exact operation. A file that
//! carries a GPU payload still reconstructs exactly through its lossless
//! payload, and the GPU payload is read only through the explicitly named
//! lossy API.
//!
//! The crate exists apart from `vpng-codecs` because that crate's contract is
//! that a codec is *exactly reversible*, and neither BC1 nor BC4 is. The
//! codec **ids** live in `vpng-codecs`, which owns the codec table.

#![deny(missing_docs)]

use vpng_core::{Error, PixelFormat, Result};

pub mod bc1;
pub mod bc4;

/// The width and height of one GPU block.
pub const BLOCK_EDGE: u32 = 4;

/// The GPU block codecs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuCodec {
    /// BC1 (DXT1): three colour channels, eight bytes per block.
    Bc1,
    /// BC4 (ATI1N): one channel, eight bytes per block.
    Bc4,
}

impl GpuCodec {
    /// The codec's id in the codec table.
    pub const fn id(self) -> u8 {
        match self {
            GpuCodec::Bc1 => vpng_codecs::CODEC_BC1,
            GpuCodec::Bc4 => vpng_codecs::CODEC_BC4,
        }
    }

    /// The codec with the given id, if there is one.
    pub fn from_id(id: u8) -> Option<Self> {
        match id {
            vpng_codecs::CODEC_BC1 => Some(GpuCodec::Bc1),
            vpng_codecs::CODEC_BC4 => Some(GpuCodec::Bc4),
            _ => None,
        }
    }

    /// The codec's name, as the codec table spells it.
    pub const fn name(self) -> &'static str {
        match self {
            GpuCodec::Bc1 => "bc1",
            GpuCodec::Bc4 => "bc4",
        }
    }

    /// How many channels the codec carries.
    pub const fn channels(self) -> u8 {
        match self {
            GpuCodec::Bc1 => 3,
            GpuCodec::Bc4 => 1,
        }
    }

    /// How many bytes one block of the codec occupies.
    pub const fn bytes_per_block(self) -> u64 {
        8
    }
}

/// The number of 4×4 blocks a tile of this size holds.
///
/// A tile whose width or height is not a multiple of four still has a whole
/// block at its edge, and the block's unused pixels are not part of the
/// image.
pub fn block_count(width: u32, height: u32) -> u64 {
    let across = u64::from(width.div_ceil(BLOCK_EDGE));
    let down = u64::from(height.div_ceil(BLOCK_EDGE));
    across * down
}

/// The number of bytes a GPU payload for a tile of this size occupies.
///
/// `colour` and `alpha` are the codecs of the payload's two parts; a part
/// that is not there contributes nothing.
pub fn payload_length(
    width: u32,
    height: u32,
    colour: Option<GpuCodec>,
    alpha: Option<GpuCodec>,
) -> u64 {
    let blocks = block_count(width, height);
    let mut length = 0u64;
    if let Some(codec) = colour {
        length += blocks * codec.bytes_per_block();
    }
    if let Some(codec) = alpha {
        length += blocks * codec.bytes_per_block();
    }
    length
}

/// One tile's GPU payload: the colour blocks and the alpha blocks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GpuPayload {
    /// The BC1 colour blocks, when the tile has them.
    pub colour: Option<Vec<u8>>,
    /// The BC4 alpha (or coverage) blocks, when the tile has them.
    pub alpha: Option<Vec<u8>>,
}

impl GpuPayload {
    /// Whether the payload holds nothing at all.
    pub fn is_empty(&self) -> bool {
        self.colour.is_none() && self.alpha.is_none()
    }

    /// The codec of the payload's colour blocks, if it has them.
    pub fn colour_codec(&self) -> Option<GpuCodec> {
        self.colour.as_ref().map(|_| GpuCodec::Bc1)
    }

    /// The codec of the payload's alpha blocks, if it has them.
    pub fn alpha_codec(&self) -> Option<GpuCodec> {
        self.alpha.as_ref().map(|_| GpuCodec::Bc4)
    }

    /// The payload's bytes: the colour blocks, then the alpha blocks.
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(colour) = &self.colour {
            out.extend_from_slice(colour);
        }
        if let Some(alpha) = &self.alpha {
            out.extend_from_slice(alpha);
        }
        out
    }

    /// The payload's length in bytes.
    pub fn len(&self) -> u64 {
        self.colour.as_ref().map_or(0, |bytes| bytes.len() as u64)
            + self.alpha.as_ref().map_or(0, |bytes| bytes.len() as u64)
    }

    /// Whether the payload holds nothing at all.
    pub fn is_empty_bytes(&self) -> bool {
        self.colour.as_ref().map_or(true, |bytes| bytes.is_empty())
            && self.alpha.as_ref().map_or(true, |bytes| bytes.is_empty())
    }
}

/// Encode a tile's pixels into a GPU payload.
///
/// `pixels` holds the tile's canonical pixels in the file's own format: one
/// byte per channel, interleaved, row by row.
///
/// `coverage_only` says that the tile is a **mask**: only the alpha channel
/// is encoded, as BC4 coverage, and the colour channels are left out
/// entirely. A mask's colour channels carry no meaning.
///
/// A 16-bit format has no GPU payload: the block codecs hold eight bits per
/// channel, and the encoder refuses rather than quietly dropping the
/// precision.
pub fn encode_tile(
    format: PixelFormat,
    width: u32,
    height: u32,
    pixels: &[u8],
    coverage_only: bool,
) -> Result<GpuPayload> {
    if format.bits != vpng_core::BitsPerChannel::Eight {
        return Err(Error::invalid(
            "GPU payloads are eight-bit block formats; a 16-bit file has no GPU payload",
        ));
    }

    let channels = u32::from(format.color_model.channel_count());
    let expected = u64::from(width) * u64::from(height) * u64::from(channels);
    if pixels.len() as u64 != expected {
        return Err(Error::invalid(format!(
            "a {width}x{height} tile in {} channels holds {expected} bytes, not {}",
            format.color_model.name(),
            pixels.len()
        )));
    }

    let has_alpha = format.color_model.has_alpha();
    let blocks_x = width.div_ceil(BLOCK_EDGE);
    let blocks_y = height.div_ceil(BLOCK_EDGE);

    let mut colour = Vec::new();
    let mut alpha = Vec::new();

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let mut block_colour = [[0u8; 3]; 16];
            let mut block_alpha = [0u8; 16];

            for row in 0..BLOCK_EDGE {
                for column in 0..BLOCK_EDGE {
                    let index = (row * BLOCK_EDGE + column) as usize;
                    let x = block_x * BLOCK_EDGE + column;
                    let y = block_y * BLOCK_EDGE + row;
                    let inside = x < width && y < height;
                    let offset = (y as usize * width as usize + x as usize) * channels as usize;

                    let (r, g, b, a) = if inside {
                        match format.color_model {
                            vpng_core::ColorModel::Gray => {
                                let value = pixels[offset];
                                (value, value, value, 255)
                            }
                            vpng_core::ColorModel::GrayAlpha => {
                                let value = pixels[offset];
                                (value, value, value, pixels[offset + 1])
                            }
                            vpng_core::ColorModel::Rgb => {
                                (pixels[offset], pixels[offset + 1], pixels[offset + 2], 255)
                            }
                            vpng_core::ColorModel::Rgba => (
                                pixels[offset],
                                pixels[offset + 1],
                                pixels[offset + 2],
                                pixels[offset + 3],
                            ),
                            other => {
                                return Err(Error::unsupported(format!(
                                    "the {} colour model has no GPU payload",
                                    other.name()
                                )))
                            }
                        }
                    } else {
                        (0, 0, 0, 0)
                    };

                    block_colour[index] = [r, g, b];
                    block_alpha[index] = a;
                }
            }

            if !coverage_only {
                colour.extend_from_slice(&bc1::encode_block(&block_colour));
            }
            if has_alpha {
                alpha.extend_from_slice(&bc4::encode_block(&block_alpha));
            }
        }
    }

    Ok(GpuPayload {
        colour: if colour.is_empty() {
            None
        } else {
            Some(colour)
        },
        alpha: if alpha.is_empty() { None } else { Some(alpha) },
    })
}

/// Decode a GPU payload into **RGBA8** pixels.
///
/// The result is always four channels, because a GPU payload is always
/// eight-bit and a texture is always RGBA. A payload with no alpha blocks is
/// fully opaque. A mask's payload is black with its coverage in the alpha.
///
/// The result is **not** the file's own format, and it is **not** the
/// original pixels: BC1 and BC4 are lossy. What it is, is what the GPU would
/// show.
pub fn decode_tile(
    width: u32,
    height: u32,
    colour: Option<&[u8]>,
    alpha: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let blocks_x = width.div_ceil(BLOCK_EDGE);
    let blocks_y = height.div_ceil(BLOCK_EDGE);
    let blocks = u64::from(blocks_x) * u64::from(blocks_y);

    if let Some(bytes) = colour {
        let expected = blocks * GpuCodec::Bc1.bytes_per_block();
        if bytes.len() as u64 != expected {
            return Err(Error::corrupt(format!(
                "a BC1 payload for a {width}x{height} tile is {expected} bytes, not {}",
                bytes.len()
            )));
        }
    }
    if let Some(bytes) = alpha {
        let expected = blocks * GpuCodec::Bc4.bytes_per_block();
        if bytes.len() as u64 != expected {
            return Err(Error::corrupt(format!(
                "a BC4 payload for a {width}x{height} tile is {expected} bytes, not {}",
                bytes.len()
            )));
        }
    }

    let mut out = vec![0u8; width as usize * height as usize * 4];

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let block_index = (block_y * blocks_x + block_x) as usize;

            let colours = colour.map(|bytes| {
                let start = block_index * bc1::BLOCK_SIZE;
                let mut block = [0u8; bc1::BLOCK_SIZE];
                block.copy_from_slice(&bytes[start..start + bc1::BLOCK_SIZE]);
                bc1::decode_block(&block)
            });

            let alphas = alpha.map(|bytes| {
                let start = block_index * bc4::BLOCK_SIZE;
                let mut block = [0u8; bc4::BLOCK_SIZE];
                block.copy_from_slice(&bytes[start..start + bc4::BLOCK_SIZE]);
                bc4::decode_block(&block)
            });

            for row in 0..BLOCK_EDGE {
                for column in 0..BLOCK_EDGE {
                    let x = block_x * BLOCK_EDGE + column;
                    let y = block_y * BLOCK_EDGE + row;
                    if x >= width || y >= height {
                        continue;
                    }
                    let index = (row * BLOCK_EDGE + column) as usize;
                    let offset = (y as usize * width as usize + x as usize) * 4;

                    let rgb = colours.map_or([0u8, 0u8, 0u8], |block| block[index]);
                    let a = alphas.map_or(255u8, |block| block[index]);

                    out[offset] = rgb[0];
                    out[offset + 1] = rgb[1];
                    out[offset + 2] = rgb[2];
                    out[offset + 3] = a;
                }
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_core::BitsPerChannel;

    fn rgba(width: u32, height: u32) -> (PixelFormat, Vec<u8>) {
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                pixels.push((x * 8) as u8);
                pixels.push((y * 8) as u8);
                pixels.push(128);
                pixels.push(255);
            }
        }
        (
            PixelFormat::new(vpng_core::ColorModel::Rgba, BitsPerChannel::Eight).unwrap(),
            pixels,
        )
    }

    #[test]
    fn a_payload_has_the_length_the_specification_says() {
        let (format, pixels) = rgba(10, 6);
        let payload = encode_tile(format, 10, 6, &pixels, false).unwrap();
        let expected = payload_length(10, 6, Some(GpuCodec::Bc1), Some(GpuCodec::Bc4));
        assert_eq!(payload.len(), expected);
        // Three blocks across, two down, sixteen bytes each.
        assert_eq!(expected, 3 * 2 * 16);
    }

    #[test]
    fn a_payload_round_trips_through_its_own_decoder() {
        let (format, pixels) = rgba(8, 8);
        let payload = encode_tile(format, 8, 8, &pixels, false).unwrap();
        let decoded =
            decode_tile(8, 8, payload.colour.as_deref(), payload.alpha.as_deref()).unwrap();
        assert_eq!(decoded.len(), 8 * 8 * 4);
        // A **uniform** alpha channel survives exactly: BC4's endpoints are
        // the block's own extremes, and a block of one value has one value in
        // its palette. A block with a *ramp* in it does not: BC4 holds eight
        // palette entries, not 256, so a varying channel is quantised. The
        // crate's own `bc4` tests say so.
        for index in 0..64 {
            assert_eq!(decoded[index * 4 + 3], 255);
        }
    }

    #[test]
    fn a_mask_payload_carries_only_coverage() {
        let (format, pixels) = rgba(8, 8);
        let payload = encode_tile(format, 8, 8, &pixels, true).unwrap();
        assert!(payload.colour.is_none());
        assert!(payload.alpha.is_some());
    }

    #[test]
    fn a_sixteen_bit_format_is_refused() {
        let format =
            PixelFormat::new(vpng_core::ColorModel::Rgba, BitsPerChannel::Sixteen).unwrap();
        let error = encode_tile(format, 2, 2, &[0u8; 32], false).unwrap_err();
        assert_eq!(error.kind(), vpng_core::ErrorKind::InvalidArgument);
    }

    #[test]
    fn a_wrong_length_payload_is_refused() {
        let error = decode_tile(8, 8, Some(&[0u8; 7]), None).unwrap_err();
        assert_eq!(error.kind(), vpng_core::ErrorKind::CorruptFile);
    }

    #[test]
    fn a_grey_picture_is_replicated_into_the_colour_channels() {
        let format = PixelFormat::new(vpng_core::ColorModel::Gray, BitsPerChannel::Eight).unwrap();
        let pixels = vec![128u8; 16];
        let payload = encode_tile(format, 4, 4, &pixels, false).unwrap();
        assert!(payload.alpha.is_none());
        let decoded = decode_tile(4, 4, payload.colour.as_deref(), None).unwrap();
        for pixel in decoded.chunks(4) {
            // The three channels stay close to one another. They are not
            // *equal*, because BC1 holds five bits of red and blue and six of
            // green, and a grey replicated into all three is quantised
            // differently in each.
            let spread =
                pixel[0].max(pixel[1]).max(pixel[2]) - pixel[0].min(pixel[1]).min(pixel[2]);
            assert!(spread <= 4, "the grey spread by {spread} levels");
            for &channel in &pixel[..3] {
                let error = i32::from(channel) - 128;
                assert!(error.abs() <= 8, "the grey moved by {error} levels");
            }
            assert_eq!(pixel[3], 255);
        }
    }
}
