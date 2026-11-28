//! BC1 (DXT1) colour blocks: eight bytes per 4×4 block.
//!
//! A BC1 block holds two RGB565 endpoints and a two-bit index per pixel. The
//! decoder expands the endpoints to eight bits per channel and builds four
//! palette entries from them; the encoder picks the endpoint pair and the
//! indices that reproduce the block's pixels most closely.
//!
//! The arithmetic here is the **reference** arithmetic, taken from the
//! published BC1 decoder so that a payload written by this crate is read
//! identically by a GPU and by any other BC1 decoder:
//!
//! * a 5-bit channel expands as `(v * 527 + 23) >> 6`;
//! * a 6-bit channel expands as `(v * 259 + 33) >> 6`;
//! * the two interpolated colours are computed from the *quantised*
//!   components, not from the expanded ones.
//!
//! BC1 is **lossy**. Nothing in this crate is ever used to reconstruct an
//! image exactly; the lossless payload is the one that does that.

/// The bytes one BC1 block occupies.
pub const BLOCK_SIZE: usize = 8;

/// A colour, as three eight-bit channels.
pub type Rgb = [u8; 3];

/// Expand a five-bit channel to eight bits.
fn expand5(value: u16) -> u8 {
    ((u32::from(value) * 527 + 23) >> 6) as u8
}

/// Expand a six-bit channel to eight bits.
fn expand6(value: u16) -> u8 {
    ((u32::from(value) * 259 + 33) >> 6) as u8
}

/// Pack five- and six-bit channels into an RGB565 value.
fn pack565(r: u16, g: u16, b: u16) -> u16 {
    (r << 11) | (g << 5) | b
}

/// The four palette entries a BC1 block's endpoints define.
///
/// When `c0 > c1` the block is in the four-colour mode: two interpolated
/// colours join the endpoints. When `c0 <= c1` the block is in the three-colour
/// mode, and the fourth entry is black.
///
/// The black entry is **opaque** black here, not the transparent black that a
/// DXT1 *texture* would use for its alpha channel. A BC1 payload in a VPNG
/// file carries colour only, so there is no alpha to make transparent, and a
/// decoder that returned transparent black would be inventing an alpha the
/// payload never had.
pub fn palette(c0: u16, c1: u16) -> [Rgb; 4] {
    let r0 = u32::from((c0 >> 11) & 0x1F);
    let g0 = u32::from((c0 >> 5) & 0x3F);
    let b0 = u32::from(c0 & 0x1F);
    let r1 = u32::from((c1 >> 11) & 0x1F);
    let g1 = u32::from((c1 >> 5) & 0x3F);
    let b1 = u32::from(c1 & 0x1F);

    let first = [expand5(r0 as u16), expand6(g0 as u16), expand5(b0 as u16)];
    let second = [expand5(r1 as u16), expand6(g1 as u16), expand5(b1 as u16)];

    if c0 > c1 {
        let third = [
            (((2 * r0 + r1) * 351 + 61) >> 7) as u8,
            (((2 * g0 + g1) * 2763 + 1039) >> 11) as u8,
            (((2 * b0 + b1) * 351 + 61) >> 7) as u8,
        ];
        let fourth = [
            (((r0 + 2 * r1) * 351 + 61) >> 7) as u8,
            (((g0 + 2 * g1) * 2763 + 1039) >> 11) as u8,
            (((b0 + 2 * b1) * 351 + 61) >> 7) as u8,
        ];
        [first, second, third, fourth]
    } else {
        let third = [
            (((r0 + r1) * 1053 + 125) >> 8) as u8,
            (((g0 + g1) * 4145 + 1019) >> 11) as u8,
            (((b0 + b1) * 1053 + 125) >> 8) as u8,
        ];
        [first, second, third, [0, 0, 0]]
    }
}

/// Decode one BC1 block into its sixteen colours, row by row.
pub fn decode_block(block: &[u8; BLOCK_SIZE]) -> [Rgb; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let colours = palette(c0, c1);

    let mut out = [[0u8; 3]; 16];
    for (index, pixel) in out.iter_mut().enumerate() {
        let choice = ((indices >> (2 * index)) & 0x03) as usize;
        *pixel = colours[choice];
    }
    out
}

/// The squared error between two colours.
fn distance(a: &Rgb, b: &Rgb) -> u64 {
    let dr = i32::from(a[0]) - i32::from(b[0]);
    let dg = i32::from(a[1]) - i32::from(b[1]);
    let db = i32::from(a[2]) - i32::from(b[2]);
    (dr * dr + dg * dg + db * db) as u64
}

/// Fit one endpoint pair to the block's pixels.
///
/// Returns the total squared error and the packed two-bit indices. The third
/// palette entry is black in the three-colour mode, so index 3 is never used
/// there: it would be a colour the payload never meant to carry.
fn fit(c0: u16, c1: u16, pixels: &[Rgb; 16]) -> (u64, u32) {
    let colours = palette(c0, c1);
    let allowed: &[usize] = if c0 > c1 { &[0, 1, 2, 3] } else { &[0, 1, 2] };

    let mut total = 0u64;
    let mut indices = 0u32;
    for (position, pixel) in pixels.iter().enumerate() {
        let mut best = allowed[0];
        let mut best_error = u64::MAX;
        for &choice in allowed {
            let error = distance(pixel, &colours[choice]);
            if error < best_error {
                best_error = error;
                best = choice;
            }
        }
        total += best_error;
        indices |= (best as u32) << (2 * position);
    }
    (total, indices)
}

/// Build a block from an endpoint pair and packed indices.
fn block(c0: u16, c1: u16, indices: u32) -> [u8; BLOCK_SIZE] {
    let mut out = [0u8; BLOCK_SIZE];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..8].copy_from_slice(&indices.to_le_bytes());
    out
}

/// Quantise an eight-bit channel to five or six bits.
///
/// The value is rounded to the nearest representable level rather than
/// truncated, so a channel that is *exactly* a representable level comes
/// back unchanged. Truncating would push a value up to seven levels away
/// from the nearest one, which is visible in a flat area of a picture.
fn quantise(value: u8, bits: u32) -> u16 {
    let levels = (1u32 << bits) - 1;
    ((u32::from(value) * levels + 127) / 255) as u16
}

/// Pull a quantised range in towards itself by a sixteenth.
///
/// The bounding box of a block's colours usually overshoots: the extremes are
/// one pixel each, and the two interpolated colours then sit away from the
/// bulk of the block. Shrinking the range brings them closer. The result is
/// only used when it measures better, so the choice never makes a block
/// worse.
fn inset(minimum: u16, maximum: u16) -> (u16, u16) {
    if maximum <= minimum {
        return (minimum, maximum);
    }
    let step = (maximum - minimum) / 16;
    (minimum + step, maximum - step)
}

/// Encode one 4×4 block of colours.
///
/// The encoder tries the block's bounding box and the bounding box pulled in
/// by a sixteenth, and keeps whichever reproduces the pixels more closely. The
/// bounding box wins a tie, so the result is a deterministic function of the
/// pixels.
pub fn encode_block(pixels: &[Rgb; 16]) -> [u8; BLOCK_SIZE] {
    let mut minimum = [255u8; 3];
    let mut maximum = [0u8; 3];
    for pixel in pixels {
        for channel in 0..3 {
            if pixel[channel] < minimum[channel] {
                minimum[channel] = pixel[channel];
            }
            if pixel[channel] > maximum[channel] {
                maximum[channel] = pixel[channel];
            }
        }
    }

    let r_min = quantise(minimum[0], 5);
    let r_max = quantise(maximum[0], 5);
    let g_min = quantise(minimum[1], 6);
    let g_max = quantise(maximum[1], 6);
    let b_min = quantise(minimum[2], 5);
    let b_max = quantise(maximum[2], 5);

    let box_endpoints = (pack565(r_max, g_max, b_max), pack565(r_min, g_min, b_min));
    let (r_low, r_high) = inset(r_min, r_max);
    let (g_low, g_high) = inset(g_min, g_max);
    let (b_low, b_high) = inset(b_min, b_max);
    let inset_endpoints = (
        pack565(r_high, g_high, b_high),
        pack565(r_low, g_low, b_low),
    );

    let (box_error, box_indices) = fit(box_endpoints.0, box_endpoints.1, pixels);
    let (inset_error, inset_indices) = fit(inset_endpoints.0, inset_endpoints.1, pixels);

    if inset_error < box_error {
        block(inset_endpoints.0, inset_endpoints.1, inset_indices)
    } else {
        block(box_endpoints.0, box_endpoints.1, box_indices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(colour: Rgb) -> [Rgb; 16] {
        [colour; 16]
    }

    #[test]
    fn a_uniform_block_survives_within_the_quantisation() {
        // BC1 holds five bits of red and blue and six of green, so a flat
        // colour comes back at the nearest level the format can hold — close,
        // but not necessarily equal.
        let pixels = uniform([200, 100, 50]);
        let encoded = encode_block(&pixels);
        let decoded = decode_block(&encoded);
        for (original, restored) in pixels.iter().zip(decoded.iter()) {
            for channel in 0..3 {
                let error = i32::from(original[channel]) - i32::from(restored[channel]);
                assert!(
                    error.abs() <= 8,
                    "channel {channel} moved by {error} levels"
                );
            }
        }
        // Every pixel of a flat block is the same colour.
        assert!(decoded.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn a_black_block_survives_exactly() {
        let pixels = uniform([0, 0, 0]);
        let encoded = encode_block(&pixels);
        assert_eq!(decode_block(&encoded), pixels);
    }

    #[test]
    fn a_white_block_survives_exactly() {
        let pixels = uniform([255, 255, 255]);
        let encoded = encode_block(&pixels);
        assert_eq!(decode_block(&encoded), pixels);
    }

    #[test]
    fn a_two_colour_pattern_survives_exactly() {
        // Black and white are both exactly representable, so a block that
        // holds only those two colours comes back unchanged.
        let mut pixels = [[0u8; 3]; 16];
        for pixel in pixels.iter_mut().step_by(3) {
            *pixel = [255, 255, 255];
        }
        let encoded = encode_block(&pixels);
        assert_eq!(decode_block(&encoded), pixels);
    }

    #[test]
    fn a_gradient_is_close() {
        let mut pixels = [[0u8; 3]; 16];
        for (index, pixel) in pixels.iter_mut().enumerate() {
            let value = (index * 16) as u8;
            *pixel = [value, value, value];
        }
        let encoded = encode_block(&pixels);
        let decoded = decode_block(&encoded);
        for (original, restored) in pixels.iter().zip(decoded.iter()) {
            for channel in 0..3 {
                let error = i32::from(original[channel]) - i32::from(restored[channel]);
                assert!(
                    error.abs() <= 40,
                    "channel {channel} moved by {error} levels"
                );
            }
        }
    }

    #[test]
    fn the_palette_expands_the_way_the_reference_does() {
        // c0 = 0xF800 (pure red), c1 = 0x001F (pure blue).
        let colours = palette(0xF800, 0x001F);
        assert_eq!(colours[0], [255, 0, 0]);
        assert_eq!(colours[1], [0, 0, 255]);
        // The two interpolated colours sit a third and two thirds of the way.
        assert_eq!(colours[2], [170, 0, 85]);
        assert_eq!(colours[3], [85, 0, 170]);
    }

    #[test]
    fn the_three_colour_mode_has_opaque_black() {
        let colours = palette(0x1234, 0x1234);
        assert_eq!(colours[3], [0, 0, 0]);
    }

    #[test]
    fn the_indices_are_row_major() {
        // The first row is white and the rest is black. A decoder that read
        // the indices in any other order would put the white somewhere else.
        let mut pixels = [[0u8; 3]; 16];
        for pixel in pixels.iter_mut().take(4) {
            *pixel = [255, 255, 255];
        }
        let encoded = encode_block(&pixels);
        let decoded = decode_block(&encoded);
        for (index, pixel) in decoded.iter().enumerate() {
            let expected = if index < 4 {
                [255, 255, 255]
            } else {
                [0, 0, 0]
            };
            assert_eq!(*pixel, expected, "pixel {index}");
        }
    }
}
