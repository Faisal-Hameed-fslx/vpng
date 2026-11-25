//! Image and container conversion for VPNG.
//!
//! Three external representations are understood:
//!
//! * `.vpng` — the native format, read through the decoder.
//! * `.png`  — via the `png` crate. 16-bit samples are byte-swapped to the
//!   canonical little-endian order on read and back on write.
//! * `.raw`  — headerless canonical pixel bytes; requires explicit `--width`,
//!   `--height`, and `--format`. Useful for tests without image dependencies.
//!
//! This module is the **only** place in the workspace that knows how a PNG
//! maps onto canonical pixels. The command line and the Python extension both
//! use it, so they cannot disagree about what a PNG means.
//!
//! # The rules
//!
//! * PNG is **big-endian** for 16-bit samples; canonical pixels are
//!   **little-endian**. The two are swapped on the way in and on the way out.
//! * An explicit alpha channel is kept as it is. PNG's `tRNS` transparency is
//!   **not** expanded into an alpha channel: a grayscale or RGB PNG with
//!   `tRNS` stays a grayscale or RGB image.
//! * An indexed PNG is **refused**, not expanded. Expanding it would invent
//!   pixels the file does not state.
//! * A bit depth below 8 is **refused**. The canonical formats are 8 and 16
//!   bits per channel.
//! * A PNG is never written with fewer channels than it was read with: alpha
//!   is preserved.

use vpng_core::{BitsPerChannel, CanonicalImage, ColorModel, Error, Limits, PixelFormat, Result};

/// External file kinds VPNG recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// The native `.vpng` container.
    Vpng,
    /// A PNG image.
    Png,
    /// Headerless canonical pixel bytes.
    Raw,
}

/// Guess a kind from the path extension.
pub fn kind_from_path(path: &str) -> Option<FileKind> {
    let lowered = path.to_ascii_lowercase();
    if lowered.ends_with(".vpng") {
        Some(FileKind::Vpng)
    } else if lowered.ends_with(".png") {
        Some(FileKind::Png)
    } else if lowered.ends_with(".raw") {
        Some(FileKind::Raw)
    } else {
        None
    }
}

/// Parse a kind name as accepted by `--input-format`.
pub fn parse_kind(text: &str) -> Result<FileKind> {
    match text.to_ascii_lowercase().as_str() {
        "vpng" => Ok(FileKind::Vpng),
        "png" => Ok(FileKind::Png),
        "raw" => Ok(FileKind::Raw),
        other => Err(Error::invalid(format!(
            "unknown format `{other}` (expected vpng, png, or raw)"
        ))),
    }
}

/// Parse a pixel-format name such as `rgba8` or `rgb16`.
pub fn parse_format(text: &str) -> Result<PixelFormat> {
    let lowered = text.to_ascii_lowercase();
    let format = match lowered.as_str() {
        "gray8" | "gray" | "g8" | "l8" => PixelFormat::gray8(),
        "gray16" | "g16" | "l16" => PixelFormat::gray16(),
        "graya8" | "gray-alpha8" | "ga8" | "la8" => PixelFormat::gray_alpha8(),
        "graya16" | "gray-alpha16" | "ga16" | "la16" => PixelFormat::gray_alpha16(),
        "rgb8" | "rgb" => PixelFormat::rgb8(),
        "rgb16" => PixelFormat::rgb16(),
        "rgba8" | "rgba" => PixelFormat::rgba8(),
        "rgba16" => PixelFormat::rgba16(),
        other => {
            return Err(Error::invalid(format!(
                "unknown pixel format `{other}` (try rgba8, rgb16, gray8, graya16, ...)"
            )))
        }
    };
    Ok(format)
}

/// Read a PNG into canonical pixels.
pub fn read_png(path: &str, limits: &Limits) -> Result<CanonicalImage> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let decoder = png::Decoder::new(reader);
    let mut png_reader = decoder
        .read_info()
        .map_err(|e| Error::corrupt(format!("png header: {e}")))?;

    let mut buffer = vec![0u8; png_reader.output_buffer_size()];
    let info = png_reader
        .next_frame(&mut buffer)
        .map_err(|e| Error::corrupt(format!("png pixel data: {e}")))?;

    let format = png_format(info.color_type, info.bit_depth)?;
    let expected = vpng_core::checked_usize(format.frame_bytes(info.width, info.height)?)?;
    if buffer.len() < expected {
        return Err(Error::corrupt(format!(
            "png produced {} bytes but {expected} were expected for {}x{} {format}",
            buffer.len(),
            info.width,
            info.height
        )));
    }
    buffer.truncate(expected);

    if format.bits == BitsPerChannel::Sixteen {
        swap_bytes_16(&mut buffer);
    }

    CanonicalImage::from_vec_checked(info.width, info.height, format, buffer, limits)
}

/// Write canonical pixels to a PNG.
pub fn write_png(path: &str, image: &CanonicalImage) -> Result<()> {
    let format = image.format();
    let color = match format.color_model {
        ColorModel::Gray => png::ColorType::Grayscale,
        ColorModel::GrayAlpha => png::ColorType::GrayscaleAlpha,
        ColorModel::Rgb => png::ColorType::Rgb,
        ColorModel::Rgba => png::ColorType::Rgba,
        ColorModel::Indexed => return Err(Error::unsupported("indexed output is not supported")),
    };
    let depth = match format.bits {
        BitsPerChannel::Eight => png::BitDepth::Eight,
        BitsPerChannel::Sixteen => png::BitDepth::Sixteen,
    };

    let file = std::fs::File::create(path)?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, image.width(), image.height());
    encoder.set_color(color);
    encoder.set_depth(depth);
    let mut png_writer = encoder
        .write_header()
        .map_err(|e| Error::internal(format!("png write header: {e}")))?;

    if format.bits == BitsPerChannel::Sixteen {
        let mut swapped = image.data().to_vec();
        swap_bytes_16(&mut swapped);
        png_writer
            .write_image_data(&swapped)
            .map_err(|e| Error::internal(format!("png write data: {e}")))?;
    } else {
        png_writer
            .write_image_data(image.data())
            .map_err(|e| Error::internal(format!("png write data: {e}")))?;
    }
    Ok(())
}

/// Read headerless canonical pixel bytes.
pub fn read_raw(
    path: &str,
    width: u32,
    height: u32,
    format: PixelFormat,
    limits: &Limits,
) -> Result<CanonicalImage> {
    let data = std::fs::read(path)?;
    let expected = format.frame_bytes(width, height)?;
    if data.len() as u64 != expected {
        return Err(Error::invalid(format!(
            "raw file is {} bytes but {expected} were expected for {width}x{height} {format}",
            data.len()
        )));
    }
    CanonicalImage::from_vec_checked(width, height, format, data, limits)
}

/// Write headerless canonical pixel bytes.
pub fn write_raw(path: &str, image: &CanonicalImage) -> Result<()> {
    std::fs::write(path, image.data())?;
    Ok(())
}

fn png_format(color: png::ColorType, depth: png::BitDepth) -> Result<PixelFormat> {
    let bits = match depth {
        png::BitDepth::Eight => BitsPerChannel::Eight,
        png::BitDepth::Sixteen => BitsPerChannel::Sixteen,
        other => {
            return Err(Error::unsupported(format!(
                "png bit depth {other:?} is not supported (only 8 and 16)"
            )))
        }
    };
    let model = match color {
        png::ColorType::Grayscale => ColorModel::Gray,
        png::ColorType::GrayscaleAlpha => ColorModel::GrayAlpha,
        png::ColorType::Rgb => ColorModel::Rgb,
        png::ColorType::Rgba => ColorModel::Rgba,
        png::ColorType::Indexed => {
            return Err(Error::unsupported(
                "indexed PNG is not supported; expand the palette to RGB(A) first",
            ))
        }
    };
    PixelFormat::new(model, bits)
}

/// Swap the two bytes of every 16-bit sample (PNG is big-endian, canonical is
/// little-endian).
fn swap_bytes_16(data: &mut [u8]) {
    for chunk in data.chunks_exact_mut(2) {
        chunk.swap(0, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_aliases() {
        assert_eq!(parse_format("rgba8").unwrap(), PixelFormat::rgba8());
        assert_eq!(parse_format("RGBA").unwrap(), PixelFormat::rgba8());
        assert_eq!(parse_format("gray16").unwrap(), PixelFormat::gray16());
        assert!(parse_format("cmyk").is_err());
    }

    #[test]
    fn kind_detection() {
        assert_eq!(kind_from_path("a.VPNG"), Some(FileKind::Vpng));
        assert_eq!(kind_from_path("a.png"), Some(FileKind::Png));
        assert_eq!(kind_from_path("a.raw"), Some(FileKind::Raw));
        assert_eq!(kind_from_path("a.jpg"), None);
    }

    #[test]
    fn sixteen_bit_swap_is_an_involution() {
        let mut data = vec![0x12u8, 0x34, 0x56, 0x78];
        let original = data.clone();
        swap_bytes_16(&mut data);
        assert_eq!(data, vec![0x34, 0x12, 0x78, 0x56]);
        swap_bytes_16(&mut data);
        assert_eq!(data, original);
    }
}
