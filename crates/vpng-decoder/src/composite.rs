//! Compositing layers over one another.
//!
//! The blend is **straight alpha** "source over": the source pixel is laid
//! over the destination pixel in proportion to the source's alpha, the
//! layer's opacity and the mask's coverage.
//!
//! Two properties matter and both are pinned by the test suite:
//!
//! * A container with one layer composites to **exactly** that layer's
//!   pixels. The blend of a single layer is the identity, not an
//!   approximation, and no rounding can creep in.
//! * The arithmetic is integer and deterministic: the same file always
//!   composites to the same bytes, on every platform.
//!
//! The weights are carried as a fraction rather than a floating-point
//! number, so the result does not depend on the host's floating-point
//! behaviour.

use vpng_core::{Error, PixelFormat, Rect, Result};

/// The byte offset of the alpha channel inside a pixel, if the format has
/// one.
///
/// The alpha channel is always the last one, so its offset is the number of
/// colour channels times the channel width.
pub fn alpha_offset(format: PixelFormat) -> Option<usize> {
    if format.color_model.has_alpha() {
        Some((format.channels() as usize - 1) * format.bytes_per_channel() as usize)
    } else {
        None
    }
}

/// The format's largest sample value: 255 for eight bits, 65535 for sixteen.
pub fn sample_max(format: PixelFormat) -> u64 {
    if format.bits.bits() == 8 {
        255
    } else {
        65535
    }
}

/// Blend one layer over the canvas.
///
/// `destination` is the canvas image, `canvas` its rectangle. `source` is the
/// layer's own image and `source_rect` where it sits on the canvas.
/// `coverage` is the mask's coverage over the layer's rectangle — one byte
/// per pixel — or `None` when the layer has no mask.
///
/// The blend runs over the intersection of the canvas and the layer's
/// rectangle, so a layer that hangs over the canvas's edge is clipped rather
/// than refused. The encoder refuses such a layer when it writes the file;
/// the decoder is tolerant of a file it did not write.
pub fn blend(
    destination: &mut vpng_core::CanonicalImage,
    canvas: Rect,
    source: &vpng_core::CanonicalImage,
    source_rect: Rect,
    coverage: Option<&[u8]>,
    opacity: u8,
    format: PixelFormat,
) -> Result<()> {
    let left = canvas.x.max(source_rect.x);
    let top = canvas.y.max(source_rect.y);
    let right = canvas.right()?.min(source_rect.right()?);
    let bottom = canvas.bottom()?.min(source_rect.bottom()?);
    if left >= right || top >= bottom {
        return Ok(());
    }

    let bpp = format.bytes_per_pixel() as usize;
    let sample_bytes = format.bytes_per_channel() as usize;
    let max = sample_max(format);
    let alpha_at = alpha_offset(format);

    let destination_row = destination.row_bytes()? as usize;
    let source_row = source.row_bytes()? as usize;

    // The weight of a pixel is `source_alpha * opacity * coverage` over
    // `max * 255 * max`. Keeping it as a fraction keeps the arithmetic
    // exact and integer.
    let denominator = max.saturating_mul(255).saturating_mul(max);

    for y in top..bottom {
        for x in left..right {
            let source_pixel =
                (y - source_rect.y) as usize * source_row + (x - source_rect.x) as usize * bpp;
            let destination_pixel =
                (y - canvas.y) as usize * destination_row + (x - canvas.x) as usize * bpp;

            let source_alpha = match alpha_at {
                Some(offset) => read_sample(&source.data()[source_pixel..], offset, sample_bytes),
                None => max,
            };
            let mask_value = match coverage {
                Some(bytes) => {
                    let index = (y - source_rect.y) as usize * source_rect.width as usize
                        + (x - source_rect.x) as usize;
                    let value = bytes.get(index).copied().unwrap_or(255) as u64;
                    if sample_bytes == 1 {
                        value
                    } else {
                        value.saturating_mul(257)
                    }
                }
                None => max,
            };

            let numerator = source_alpha
                .saturating_mul(opacity as u64)
                .saturating_mul(mask_value);
            let rest = denominator.saturating_sub(numerator);

            // The colour channels blend with the source's own weight.
            for channel in 0..format.channels() as usize {
                let source_value = read_sample(
                    &source.data()[source_pixel..],
                    channel * sample_bytes,
                    sample_bytes,
                );
                let destination_value = read_sample(
                    &destination.data()[destination_pixel..],
                    channel * sample_bytes,
                    sample_bytes,
                );
                let blended = (source_value
                    .saturating_mul(numerator)
                    .saturating_add(destination_value.saturating_mul(rest)))
                    / denominator;
                write_sample(
                    &mut destination.data_mut()[destination_pixel..],
                    channel * sample_bytes,
                    sample_bytes,
                    blended.min(max),
                );
            }

            // The alpha channel is the weight itself, scaled back to the
            // format's range: `max * w + destination_alpha * (1 - w)`.
            if let Some(offset) = alpha_at {
                let destination_alpha = read_sample(
                    &destination.data()[destination_pixel..],
                    offset,
                    sample_bytes,
                );
                let blended = (max
                    .saturating_mul(numerator)
                    .saturating_add(destination_alpha.saturating_mul(rest)))
                    / denominator;
                write_sample(
                    &mut destination.data_mut()[destination_pixel..],
                    offset,
                    sample_bytes,
                    blended.min(max),
                );
            }
        }
    }

    Ok(())
}

/// Read one channel sample out of a pixel.
fn read_sample(pixel: &[u8], offset: usize, bytes: usize) -> u64 {
    if bytes == 1 {
        pixel[offset] as u64
    } else {
        u16::from_le_bytes([pixel[offset], pixel[offset + 1]]) as u64
    }
}

/// Write one channel sample into a pixel.
fn write_sample(pixel: &mut [u8], offset: usize, bytes: usize, value: u64) {
    if bytes == 1 {
        pixel[offset] = value.min(255) as u8;
    } else {
        let value = value.min(65535) as u16;
        pixel[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
}

/// Copy a decoded tile's pixels into a layer image at its geometry.
///
/// The layer's tiles carry canvas coordinates, so the origin is subtracted:
/// the destination image is sized to the layer's rectangle, not the canvas.
pub(crate) fn blit_at(
    image: &mut vpng_core::CanonicalImage,
    geom: &vpng_tiling::TileRect,
    origin_x: u32,
    origin_y: u32,
    pixels: &[u8],
) -> Result<()> {
    let bpp = image.bytes_per_pixel() as usize;
    let destination_row = image.row_bytes()? as usize;
    let tile_row = geom.width as usize * bpp;
    let expected = tile_row * geom.height as usize;

    if pixels.len() != expected {
        return Err(Error::corrupt(format!(
            "tile pixel buffer is {} bytes, expected {expected}",
            pixels.len()
        )));
    }

    let data = image.data_mut();
    for row in 0..geom.height as usize {
        let source = row * tile_row;
        let destination = ((geom.y - origin_y) as usize + row) * destination_row
            + (geom.x - origin_x) as usize * bpp;
        data[destination..destination + tile_row]
            .copy_from_slice(&pixels[source..source + tile_row]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_core::CanonicalImage;

    /// A two-by-two RGBA8 image from four explicit pixels.
    fn image(pixels: &[[u8; 4]]) -> CanonicalImage {
        let mut data = Vec::with_capacity(pixels.len() * 4);
        for pixel in pixels {
            data.extend_from_slice(pixel);
        }
        CanonicalImage::from_vec(2, 2, PixelFormat::rgba8(), data).unwrap()
    }

    /// The canvas the tests composite on.
    fn canvas() -> Rect {
        Rect::new(0, 0, 2, 2).unwrap()
    }

    /// One pixel repeated over the whole canvas.
    fn filled(pixel: [u8; 4]) -> Vec<u8> {
        pixel.repeat(4)
    }

    #[test]
    fn an_opaque_source_replaces_the_destination() {
        let mut destination = image(&[[255, 0, 0, 255]; 4]);
        let source = image(&[[0, 0, 255, 255]; 4]);

        blend(
            &mut destination,
            canvas(),
            &source,
            canvas(),
            None,
            255,
            PixelFormat::rgba8(),
        )
        .unwrap();

        assert_eq!(destination.data(), filled([0, 0, 255, 255]).as_slice());
    }

    #[test]
    fn a_transparent_source_changes_nothing() {
        let mut destination = image(&[[255, 0, 0, 255]; 4]);
        let source = image(&[[0, 0, 255, 0]; 4]);

        blend(
            &mut destination,
            canvas(),
            &source,
            canvas(),
            None,
            255,
            PixelFormat::rgba8(),
        )
        .unwrap();

        assert_eq!(destination.data(), filled([255, 0, 0, 255]).as_slice());
    }

    #[test]
    fn half_opacity_blends_halfway() {
        let mut destination = image(&[[255, 255, 255, 255]; 4]);
        let source = image(&[[0, 0, 0, 255]; 4]);

        blend(
            &mut destination,
            canvas(),
            &source,
            canvas(),
            None,
            128,
            PixelFormat::rgba8(),
        )
        .unwrap();

        // 255 * (1 - 128/255) = 127 exactly: the arithmetic is integer and
        // the rounding is pinned, not accidental.
        assert_eq!(destination.data(), filled([127, 127, 127, 255]).as_slice());
    }

    #[test]
    fn a_fully_covered_mask_does_not_change_the_blend() {
        let mut plain = image(&[[255, 255, 255, 255]; 4]);
        let mut masked = image(&[[255, 255, 255, 255]; 4]);
        let source = image(&[[0, 0, 0, 255]; 4]);
        let coverage = [255u8; 4];

        blend(
            &mut plain,
            canvas(),
            &source,
            canvas(),
            None,
            255,
            PixelFormat::rgba8(),
        )
        .unwrap();
        blend(
            &mut masked,
            canvas(),
            &source,
            canvas(),
            Some(&coverage),
            255,
            PixelFormat::rgba8(),
        )
        .unwrap();

        assert_eq!(plain.data(), masked.data());
    }

    #[test]
    fn a_mask_that_says_nothing_hides_the_source() {
        let mut destination = image(&[[255, 0, 0, 255]; 4]);
        let source = image(&[[0, 0, 255, 255]; 4]);
        let coverage = [0u8; 4];

        blend(
            &mut destination,
            canvas(),
            &source,
            canvas(),
            Some(&coverage),
            255,
            PixelFormat::rgba8(),
        )
        .unwrap();

        assert_eq!(destination.data(), filled([255, 0, 0, 255]).as_slice());
    }

    #[test]
    fn the_alpha_channel_blends_on_its_own() {
        // A transparent destination under an opaque source becomes opaque;
        // an opaque destination under a transparent source stays opaque.
        let mut destination = image(&[[10, 20, 30, 0]; 4]);
        let source = image(&[[40, 50, 60, 255]; 4]);
        blend(
            &mut destination,
            canvas(),
            &source,
            canvas(),
            None,
            255,
            PixelFormat::rgba8(),
        )
        .unwrap();
        assert_eq!(destination.data()[3], 255);
        assert_eq!(destination.data()[0], 40);

        let mut destination = image(&[[10, 20, 30, 255]; 4]);
        let source = image(&[[40, 50, 60, 0]; 4]);
        blend(
            &mut destination,
            canvas(),
            &source,
            canvas(),
            None,
            255,
            PixelFormat::rgba8(),
        )
        .unwrap();
        assert_eq!(destination.data()[3], 255);
        assert_eq!(destination.data()[0], 10);
    }

    #[test]
    fn a_layer_that_hangs_over_the_canvas_is_clipped() {
        let mut destination = image(&[[255, 0, 0, 255]; 4]);
        let source = image(&[[0, 0, 255, 255]; 4]);
        let source_rect = Rect::new(1, 1, 2, 2).unwrap();

        blend(
            &mut destination,
            canvas(),
            &source,
            source_rect,
            None,
            255,
            PixelFormat::rgba8(),
        )
        .unwrap();

        // Only the bottom-right pixel of the canvas is inside the layer.
        assert_eq!(&destination.data()[0..4], &[255, 0, 0, 255]);
        assert_eq!(&destination.data()[12..16], &[0, 0, 255, 255]);
    }

    #[test]
    fn sixteen_bit_samples_blend_too() {
        let mut destination = CanonicalImage::from_vec(
            1,
            1,
            PixelFormat::rgba16(),
            vec![0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        )
        .unwrap();
        let source = CanonicalImage::from_vec(
            1,
            1,
            PixelFormat::rgba16(),
            vec![0, 0, 0, 0, 0, 0, 0xFF, 0xFF],
        )
        .unwrap();

        blend(
            &mut destination,
            Rect::new(0, 0, 1, 1).unwrap(),
            &source,
            Rect::new(0, 0, 1, 1).unwrap(),
            None,
            255,
            PixelFormat::rgba16(),
        )
        .unwrap();

        assert_eq!(&destination.data()[0..2], &[0, 0]);
        assert_eq!(&destination.data()[6..8], &[0xFF, 0xFF]);
    }

    #[test]
    fn the_alpha_offset_is_the_last_channel() {
        assert_eq!(alpha_offset(PixelFormat::rgba8()), Some(3));
        assert_eq!(alpha_offset(PixelFormat::gray_alpha8()), Some(1));
        assert_eq!(alpha_offset(PixelFormat::rgb8()), None);
        assert_eq!(sample_max(PixelFormat::rgb8()), 255);
        assert_eq!(sample_max(PixelFormat::rgb16()), 65535);
    }
}
