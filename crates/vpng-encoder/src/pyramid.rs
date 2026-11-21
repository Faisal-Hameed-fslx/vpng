//! The preview pyramid (P6).
//!
//! A preview pyramid is a stack of **derived** levels above the original
//! image. Level 0 is the original and holds original pixels; level 1 is half
//! as wide and half as tall, level 2 a quarter, and so on. A viewer that only
//! wants to show the picture can decode the smallest level it can get, which
//! is a quarter of the pixels of the level below it, instead of decoding the
//! original.
//!
//! A derived level is **never** used to reconstruct the original. The original
//! is always level 0, and it is always exact: the pyramid adds previews, it
//! does not replace the source of truth.
//!
//! # How a level is made
//!
//! Each level is made from the level below it by a **2x2 box average**: the
//! preview pixel at `(x, y)` is the mean of the source pixels at
//! `(2x, 2y)`, `(2x+1, 2y)`, `(2x, 2y+1)` and `(2x+1, 2y+1)`. At the right
//! and bottom edges the block is smaller, and the mean is taken over the
//! pixels that exist.
//!
//! The mean is computed in integers: the sum of the samples is divided by
//! the number of samples, rounded to the nearest integer (a half rounds up).
//! There is no floating point anywhere in the pyramid, so the same image
//! always produces the same previews, on every machine.
//!
//! The alpha channel is averaged like any other channel. A preview is a
//! picture to look at, not a mask to composite, and averaging it keeps the
//! rule simple enough to state exactly.

use vpng_core::{CanonicalImage, Dimensions, Result};

/// One derived preview level.
#[derive(Debug, Clone)]
pub struct Preview {
    /// The level's dimensions.
    pub dimensions: Dimensions,
    /// The level's pixels.
    pub image: CanonicalImage,
}

/// Build the derived preview levels of an image.
///
/// The levels are built from the one below them, so the first is half the
/// image, the second a quarter of the first, and so on. The chain stops when
/// the requested number of levels has been built or when the next level
/// would not be **strictly smaller** than the one below it in both
/// dimensions — which is what the format requires of a derived level.
///
/// A one-pixel image therefore has no preview at all: halving it would not
/// make it smaller.
pub fn build_previews(image: &CanonicalImage, count: u32) -> Result<Vec<Preview>> {
    let mut previews: Vec<Preview> = Vec::new();
    let mut below = image.dimensions();

    for _ in 0..count {
        let next = downsample_half(match previews.last() {
            Some(last) => &last.image,
            None => image,
        })?;
        let dimensions = next.dimensions();

        // A derived level must be strictly smaller than the level below it.
        if dimensions.width >= below.width || dimensions.height >= below.height {
            break;
        }

        previews.push(Preview {
            dimensions,
            image: next,
        });
        below = dimensions;
    }

    Ok(previews)
}

/// Halve an image with a 2x2 box average.
///
/// The result is `(width + 1) / 2` by `(height + 1) / 2` pixels: every
/// output pixel covers a 2x2 block of the source, and the blocks at the
/// right and bottom edges are clipped to the source.
pub fn downsample_half(image: &CanonicalImage) -> Result<CanonicalImage> {
    let format = image.format();
    let width = image.width();
    let height = image.height();
    let new_width = width.div_ceil(2);
    let new_height = height.div_ceil(2);

    let channels = format.channels() as usize;
    let sample_bytes = format.bytes_per_channel() as usize;
    let bpp = format.bytes_per_pixel() as usize;
    let row = image.row_bytes()? as usize;
    let data = image.data();

    let mut out = vec![0u8; (new_width as usize) * (new_height as usize) * bpp];
    let out_row = new_width as usize * bpp;

    for py in 0..new_height {
        let y0 = py * 2;
        let y1 = (y0 + 1).min(height - 1);

        // The distinct source rows of this block: one at the bottom edge,
        // two everywhere else.
        let rows = if y1 == y0 { 1 } else { 2 };

        for px in 0..new_width {
            let x0 = px * 2;
            let x1 = (x0 + 1).min(width - 1);
            let columns = if x1 == x0 { 1 } else { 2 };
            let count = rows * columns;

            let base = (py as usize) * out_row + (px as usize) * bpp;
            for channel in 0..channels {
                let mut sum: u32 = 0;
                for dy in 0..rows {
                    let y = y0 + dy;
                    for dx in 0..columns {
                        let x = x0 + dx;
                        let offset =
                            (y as usize) * row + (x as usize) * bpp + channel * sample_bytes;
                        sum += read_sample(data, offset, sample_bytes);
                    }
                }
                // Round to the nearest integer, a half rounding up.
                let value = (sum + count / 2) / count;
                write_sample(&mut out, base + channel * sample_bytes, sample_bytes, value);
            }
        }
    }

    CanonicalImage::from_vec(new_width, new_height, format, out)
}

/// Read one channel sample as an integer.
fn read_sample(data: &[u8], offset: usize, sample_bytes: usize) -> u32 {
    match sample_bytes {
        1 => *data.get(offset).unwrap_or(&0) as u32,
        2 => {
            let low = *data.get(offset).unwrap_or(&0) as u32;
            let high = *data.get(offset + 1).unwrap_or(&0) as u32;
            (high << 8) | low
        }
        _ => 0,
    }
}

/// Write one channel sample.
fn write_sample(data: &mut [u8], offset: usize, sample_bytes: usize, value: u32) {
    match sample_bytes {
        1 => {
            if let Some(slot) = data.get_mut(offset) {
                *slot = value as u8;
            }
        }
        2 => {
            if let Some(slot) = data.get_mut(offset) {
                *slot = (value & 0xff) as u8;
            }
            if let Some(slot) = data.get_mut(offset + 1) {
                *slot = ((value >> 8) & 0xff) as u8;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_core::PixelFormat;

    fn image(width: u32, height: u32, format: PixelFormat) -> CanonicalImage {
        let bpp = format.bytes_per_pixel() as usize;
        let mut data = vec![0u8; (width * height) as usize * bpp];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let base = (y * width as usize + x) * bpp;
                for c in 0..bpp {
                    data[base + c] = ((x * 7 + y * 13 + c * 29) & 0xff) as u8;
                }
            }
        }
        CanonicalImage::from_vec(width, height, format, data).unwrap()
    }

    #[test]
    fn a_flat_image_stays_flat() {
        let flat =
            CanonicalImage::from_vec(8, 8, PixelFormat::rgba8(), vec![0x7F; 8 * 8 * 4]).unwrap();
        let half = downsample_half(&flat).unwrap();
        assert_eq!(half.width(), 4);
        assert_eq!(half.height(), 4);
        assert!(half.data().iter().all(|&byte| byte == 0x7F));
    }

    #[test]
    fn the_average_is_the_mean_of_the_block() {
        // Four pixels 0, 100, 200, 44 average to 86.
        let pixels = vec![0u8, 100u8, 200u8, 44u8];
        let source = CanonicalImage::from_vec(2, 2, PixelFormat::gray8(), pixels).unwrap();
        let half = downsample_half(&source).unwrap();
        assert_eq!(half.data(), &[86]);
    }

    #[test]
    fn a_half_rounds_up() {
        let pixels = vec![0u8, 1u8];
        let source = CanonicalImage::from_vec(2, 1, PixelFormat::gray8(), pixels).unwrap();
        let half = downsample_half(&source).unwrap();
        assert_eq!(half.data(), &[1]);
    }

    #[test]
    fn odd_sizes_keep_the_last_pixel() {
        let source = image(5, 3, PixelFormat::rgba8());
        let half = downsample_half(&source).unwrap();
        assert_eq!(half.width(), 3);
        assert_eq!(half.height(), 2);
    }

    #[test]
    fn the_levels_get_strictly_smaller() {
        let source = image(64, 32, PixelFormat::rgba8());
        let previews = build_previews(&source, 4).unwrap();
        assert_eq!(previews.len(), 4);
        assert_eq!(previews[0].dimensions.width, 32);
        assert_eq!(previews[0].dimensions.height, 16);
        assert_eq!(previews[3].dimensions.width, 4);
        assert_eq!(previews[3].dimensions.height, 2);
    }

    #[test]
    fn a_one_pixel_image_has_no_preview() {
        let source = image(1, 1, PixelFormat::rgba8());
        let previews = build_previews(&source, 3).unwrap();
        assert!(previews.is_empty());
    }

    #[test]
    fn the_pyramid_is_deterministic() {
        let source = image(37, 19, PixelFormat::rgb8());
        let a = build_previews(&source, 3).unwrap();
        let b = build_previews(&source, 3).unwrap();
        for (left, right) in a.iter().zip(b.iter()) {
            assert_eq!(left.image, right.image);
        }
    }

    #[test]
    fn sixteen_bit_samples_survive() {
        let mut data = Vec::new();
        for _ in 0..4 {
            data.extend_from_slice(&0x1234u16.to_le_bytes());
        }
        let source = CanonicalImage::from_vec(2, 2, PixelFormat::gray16(), data).unwrap();
        let half = downsample_half(&source).unwrap();
        assert_eq!(half.data(), &[0x34, 0x12]);
    }
}
