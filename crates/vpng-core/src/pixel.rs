//! Pixel formats: the canonical description of channel layout.
//!
//! Canonical pixel buffers are defined by these rules (see
//! `docs/VPNG_SPEC_0_1.md`):
//!
//! * Samples are stored row-major, top-to-bottom, left-to-right.
//! * Channels are interleaved in the order documented per color model.
//! * 16-bit samples are stored **little-endian**.
//! * Rows carry no padding. A row is exactly `width * bytes_per_pixel` bytes.
//! * Values under a fully transparent pixel are preserved verbatim.

use std::fmt;

use crate::error::{Error, Result};

/// How the channels of a pixel are interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ColorModel {
    /// One luminance channel.
    Gray = 0,
    /// Luminance then alpha.
    GrayAlpha = 1,
    /// Red, green, blue.
    Rgb = 2,
    /// Red, green, blue, alpha.
    Rgba = 3,
    /// A single index into a palette. Reserved; not implemented in v0.1.
    Indexed = 4,
}

impl ColorModel {
    pub fn from_u8(value: u8) -> Result<Self> {
        match value {
            0 => Ok(ColorModel::Gray),
            1 => Ok(ColorModel::GrayAlpha),
            2 => Ok(ColorModel::Rgb),
            3 => Ok(ColorModel::Rgba),
            4 => Ok(ColorModel::Indexed),
            other => Err(Error::unsupported(format!(
                "unknown color model id {other}"
            ))),
        }
    }

    /// Number of interleaved channels produced by this model.
    pub const fn channel_count(self) -> u8 {
        match self {
            ColorModel::Gray | ColorModel::Indexed => 1,
            ColorModel::GrayAlpha => 2,
            ColorModel::Rgb => 3,
            ColorModel::Rgba => 4,
        }
    }

    /// Whether this model carries an alpha channel.
    pub const fn has_alpha(self) -> bool {
        matches!(self, ColorModel::GrayAlpha | ColorModel::Rgba)
    }

    pub const fn name(self) -> &'static str {
        match self {
            ColorModel::Gray => "Gray",
            ColorModel::GrayAlpha => "GrayAlpha",
            ColorModel::Rgb => "RGB",
            ColorModel::Rgba => "RGBA",
            ColorModel::Indexed => "Indexed",
        }
    }
}

/// Bit width of a single channel sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BitsPerChannel {
    Eight = 8,
    Sixteen = 16,
}

impl BitsPerChannel {
    pub fn from_u8(value: u8) -> Result<Self> {
        match value {
            8 => Ok(BitsPerChannel::Eight),
            16 => Ok(BitsPerChannel::Sixteen),
            other => Err(Error::unsupported(format!(
                "unsupported bits per channel {other} (expected 8 or 16)"
            ))),
        }
    }

    pub const fn bits(self) -> u8 {
        self as u8
    }

    pub const fn bytes(self) -> u8 {
        (self as u8) / 8
    }
}

/// A complete, unambiguous pixel layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelFormat {
    pub color_model: ColorModel,
    pub bits: BitsPerChannel,
}

impl PixelFormat {
    /// Build a format, rejecting combinations that v0.1 does not implement.
    pub fn new(color_model: ColorModel, bits: BitsPerChannel) -> Result<Self> {
        if color_model == ColorModel::Indexed {
            return Err(Error::unsupported(
                "indexed palette images are not implemented in v0.1",
            ));
        }
        Ok(PixelFormat { color_model, bits })
    }

    pub const fn gray8() -> Self {
        PixelFormat {
            color_model: ColorModel::Gray,
            bits: BitsPerChannel::Eight,
        }
    }

    pub const fn gray16() -> Self {
        PixelFormat {
            color_model: ColorModel::Gray,
            bits: BitsPerChannel::Sixteen,
        }
    }

    pub const fn gray_alpha8() -> Self {
        PixelFormat {
            color_model: ColorModel::GrayAlpha,
            bits: BitsPerChannel::Eight,
        }
    }

    pub const fn gray_alpha16() -> Self {
        PixelFormat {
            color_model: ColorModel::GrayAlpha,
            bits: BitsPerChannel::Sixteen,
        }
    }

    pub const fn rgb8() -> Self {
        PixelFormat {
            color_model: ColorModel::Rgb,
            bits: BitsPerChannel::Eight,
        }
    }

    pub const fn rgb16() -> Self {
        PixelFormat {
            color_model: ColorModel::Rgb,
            bits: BitsPerChannel::Sixteen,
        }
    }

    pub const fn rgba8() -> Self {
        PixelFormat {
            color_model: ColorModel::Rgba,
            bits: BitsPerChannel::Eight,
        }
    }

    pub const fn rgba16() -> Self {
        PixelFormat {
            color_model: ColorModel::Rgba,
            bits: BitsPerChannel::Sixteen,
        }
    }

    pub const fn channels(&self) -> u8 {
        self.color_model.channel_count()
    }

    pub const fn bytes_per_channel(&self) -> u8 {
        self.bits.bytes()
    }

    pub const fn bytes_per_pixel(&self) -> u64 {
        self.channels() as u64 * self.bits.bytes() as u64
    }

    pub fn bytes_per_row(&self, width: u32) -> Result<u64> {
        crate::checked_mul(width as u64, self.bytes_per_pixel())
    }

    pub fn frame_bytes(&self, width: u32, height: u32) -> Result<u64> {
        let row = self.bytes_per_row(width)?;
        crate::checked_mul(row, height as u64)
    }

    /// All formats enabled in the v0.1 stable set, in a fixed order.
    pub const ALL_V0_1: [PixelFormat; 8] = [
        PixelFormat {
            color_model: ColorModel::Gray,
            bits: BitsPerChannel::Eight,
        },
        PixelFormat {
            color_model: ColorModel::Gray,
            bits: BitsPerChannel::Sixteen,
        },
        PixelFormat {
            color_model: ColorModel::GrayAlpha,
            bits: BitsPerChannel::Eight,
        },
        PixelFormat {
            color_model: ColorModel::GrayAlpha,
            bits: BitsPerChannel::Sixteen,
        },
        PixelFormat {
            color_model: ColorModel::Rgb,
            bits: BitsPerChannel::Eight,
        },
        PixelFormat {
            color_model: ColorModel::Rgb,
            bits: BitsPerChannel::Sixteen,
        },
        PixelFormat {
            color_model: ColorModel::Rgba,
            bits: BitsPerChannel::Eight,
        },
        PixelFormat {
            color_model: ColorModel::Rgba,
            bits: BitsPerChannel::Sixteen,
        },
    ];
}

impl fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.color_model.name(), self.bits.bits())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_counts() {
        assert_eq!(ColorModel::Gray.channel_count(), 1);
        assert_eq!(ColorModel::GrayAlpha.channel_count(), 2);
        assert_eq!(ColorModel::Rgb.channel_count(), 3);
        assert_eq!(ColorModel::Rgba.channel_count(), 4);
    }

    #[test]
    fn bytes_per_pixel() {
        assert_eq!(PixelFormat::rgba8().bytes_per_pixel(), 4);
        assert_eq!(PixelFormat::rgb16().bytes_per_pixel(), 6);
        assert_eq!(PixelFormat::gray16().bytes_per_pixel(), 2);
        assert_eq!(PixelFormat::rgba16().bytes_per_pixel(), 8);
    }

    #[test]
    fn frame_bytes_checks_overflow() {
        // 65536 * 65536 * 8 overflows u32 but must fit the checked u64 path.
        let v = PixelFormat::rgba16().frame_bytes(65_536, 65_536).unwrap();
        assert_eq!(v, 65_536u64 * 65_536 * 8);
    }

    #[test]
    fn indexed_is_rejected() {
        assert!(PixelFormat::new(ColorModel::Indexed, BitsPerChannel::Eight).is_err());
    }

    #[test]
    fn display_is_stable() {
        assert_eq!(PixelFormat::rgba8().to_string(), "RGBA8");
        assert_eq!(PixelFormat::rgb16().to_string(), "RGB16");
    }
}
