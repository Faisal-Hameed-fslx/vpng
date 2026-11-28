//! BC4 (ATI1N) single-channel blocks: eight bytes per 4×4 block.
//!
//! A BC4 block holds two eight-bit endpoints and a three-bit index per
//! pixel. The decoder builds an eight-entry palette from the endpoints, and
//! the encoder picks the endpoint pair and the indices that reproduce the
//! block's values most closely.
//!
//! The palette arithmetic is the **reference** arithmetic, taken from the
//! published BC4 decoder, so that a payload written here is read identically
//! by a GPU and by any other BC4 decoder. The two modes are:
//!
//! * **eight-value mode** (`red0 > red1`): six values interpolated between
//!   the endpoints in sevenths;
//! * **six-value mode** (`red0 <= red1`): four values interpolated between
//!   the endpoints in fifths, plus a hard zero and a hard 255.
//!
//! The encoder tries both modes and keeps whichever reproduces the block's
//! values more closely, so a block of mostly-transparent pixels gets the mode
//! that reaches zero and 255 exactly.

/// The bytes one BC4 block occupies.
pub const BLOCK_SIZE: usize = 8;

/// The eight palette entries a BC4 block's endpoints define.
pub fn palette(red0: u8, red1: u8) -> [u8; 8] {
    let a0 = u32::from(red0);
    let a1 = u32::from(red1);
    if red0 > red1 {
        [
            red0,
            ((56173 * a0 + 9363 * a1 + 32768) >> 16) as u8,
            ((46812 * a0 + 18724 * a1 + 32768) >> 16) as u8,
            ((37450 * a0 + 28086 * a1 + 32768) >> 16) as u8,
            ((28086 * a0 + 37450 * a1 + 32768) >> 16) as u8,
            ((18724 * a0 + 46812 * a1 + 32768) >> 16) as u8,
            ((9363 * a0 + 56173 * a1 + 32768) >> 16) as u8,
            red1,
        ]
    } else {
        [
            red0,
            red1,
            ((52429 * a0 + 13107 * a1 + 32768) >> 16) as u8,
            ((39321 * a0 + 26215 * a1 + 32768) >> 16) as u8,
            ((26215 * a0 + 39321 * a1 + 32768) >> 16) as u8,
            ((13107 * a0 + 52429 * a1 + 32768) >> 16) as u8,
            0,
            255,
        ]
    }
}

/// Decode one BC4 block into its sixteen values, row by row.
pub fn decode_block(block: &[u8; BLOCK_SIZE]) -> [u8; 16] {
    let values = palette(block[0], block[1]);

    let mut out = [0u8; 16];
    let mut bits = 0u64;
    for (offset, byte) in block[2..8].iter().enumerate() {
        bits |= u64::from(*byte) << (8 * offset);
    }

    for (index, value) in out.iter_mut().enumerate() {
        let choice = ((bits >> (3 * index)) & 0x07) as usize;
        *value = values[choice];
    }
    out
}

/// Fit one endpoint pair to the block's values.
fn fit(red0: u8, red1: u8, values: &[u8; 16]) -> (u64, u64) {
    let palette = palette(red0, red1);

    let mut total = 0u64;
    let mut indices = 0u64;
    for (position, value) in values.iter().enumerate() {
        let mut best = 0usize;
        let mut best_error = u64::MAX;
        for (choice, candidate) in palette.iter().enumerate() {
            let error = (i32::from(*value) - i32::from(*candidate)).unsigned_abs() as u64;
            if error < best_error {
                best_error = error;
                best = choice;
            }
        }
        total += best_error;
        indices |= (best as u64) << (3 * position);
    }
    (total, indices)
}

/// Build a block from an endpoint pair and packed indices.
fn block(red0: u8, red1: u8, indices: u64) -> [u8; BLOCK_SIZE] {
    let mut out = [0u8; BLOCK_SIZE];
    out[0] = red0;
    out[1] = red1;
    for (offset, byte) in out[2..8].iter_mut().enumerate() {
        *byte = ((indices >> (8 * offset)) & 0xFF) as u8;
    }
    out
}

/// Encode one 4×4 block of single-channel values.
///
/// The encoder tries the eight-value mode and the six-value mode, and keeps
/// whichever reproduces the values more closely. The eight-value mode wins a
/// tie, so the result is a deterministic function of the values.
pub fn encode_block(values: &[u8; 16]) -> [u8; BLOCK_SIZE] {
    let mut minimum = 255u8;
    let mut maximum = 0u8;
    for value in values {
        if *value < minimum {
            minimum = *value;
        }
        if *value > maximum {
            maximum = *value;
        }
    }

    // The eight-value mode: the endpoints are the block's own extremes.
    let (eight_error, eight_indices) = fit(maximum, minimum, values);

    // The six-value mode: the endpoints are reversed, so the decoder's
    // interpolation runs the other way and the hard zero and 255 are
    // reachable.
    let (six_error, six_indices) = fit(minimum, maximum, values);

    if six_error < eight_error {
        block(minimum, maximum, six_indices)
    } else {
        block(maximum, minimum, eight_indices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uniform_block_survives_exactly() {
        let values = [77u8; 16];
        let encoded = encode_block(&values);
        assert_eq!(decode_block(&encoded), values);
    }

    #[test]
    fn a_zero_block_survives_exactly() {
        let values = [0u8; 16];
        let encoded = encode_block(&values);
        assert_eq!(decode_block(&encoded), values);
    }

    #[test]
    fn a_full_block_survives_exactly() {
        let values = [255u8; 16];
        let encoded = encode_block(&values);
        assert_eq!(decode_block(&encoded), values);
    }

    #[test]
    fn a_ramp_is_close() {
        let mut values = [0u8; 16];
        for (index, value) in values.iter_mut().enumerate() {
            *value = (index * 17) as u8;
        }
        let encoded = encode_block(&values);
        let decoded = decode_block(&encoded);
        for (original, restored) in values.iter().zip(decoded.iter()) {
            let error = i32::from(*original) - i32::from(*restored);
            assert!(error.abs() <= 20, "a value moved by {error} levels");
        }
    }

    #[test]
    fn the_palette_reaches_zero_and_255_in_the_six_value_mode() {
        let values = palette(10, 200);
        assert_eq!(values[0], 10);
        assert_eq!(values[1], 200);
        assert_eq!(values[6], 0);
        assert_eq!(values[7], 255);
    }

    #[test]
    fn the_palette_interpolates_in_the_eight_value_mode() {
        let values = palette(200, 10);
        assert_eq!(values[0], 200);
        assert_eq!(values[7], 10);
        // The eight-value mode steps in sevenths between the endpoints.
        assert_eq!(values[1], 173);
        assert_eq!(values[3], 119);
        assert_eq!(values[4], 91);
    }

    #[test]
    fn the_indices_are_row_major() {
        let mut values = [0u8; 16];
        for value in values.iter_mut().take(4) {
            *value = 255;
        }
        let encoded = encode_block(&values);
        let decoded = decode_block(&encoded);
        for (index, value) in decoded.iter().enumerate() {
            let expected = if index < 4 { 255 } else { 0 };
            assert_eq!(*value, expected, "value {index}");
        }
    }
}
