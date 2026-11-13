//! The canonical pixel buffer.
//!
//! A [`CanonicalImage`] is the single authoritative representation of pixels.
//! Encoders accept it, decoders produce it, and the SHA-256 canonical digest is
//! defined over it. Its invariants are:
//!
//! * `data.len() == format.frame_bytes(width, height)`.
//! * Rows are tightly packed with no padding.
//! * 16-bit samples are little-endian.

use crate::error::{Error, Result};
use crate::geom::{Dimensions, Rect};
use crate::limits::Limits;
use crate::pixel::PixelFormat;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalImage {
    width: u32,
    height: u32,
    format: PixelFormat,
    data: Vec<u8>,
}

impl CanonicalImage {
    /// Allocate a zero-filled image, validating against `limits`.
    pub fn new_zeroed(
        width: u32,
        height: u32,
        format: PixelFormat,
        limits: &Limits,
    ) -> Result<Self> {
        let dims = Dimensions::new(width, height)?;
        limits.validate_dimensions(dims, "image")?;
        let len = format.frame_bytes(width, height)?;
        let len = crate::checked_usize(len)?;
        Ok(CanonicalImage {
            width,
            height,
            format,
            data: vec![0u8; len],
        })
    }

    /// Wrap an existing buffer, checking that its length matches the format.
    pub fn from_vec(width: u32, height: u32, format: PixelFormat, data: Vec<u8>) -> Result<Self> {
        let dims = Dimensions::new(width, height)?;
        let expected = format.frame_bytes(width, height)?;
        if data.len() as u64 != expected {
            return Err(Error::invalid(format!(
                "pixel buffer length {} does not match {}x{} {} (expected {expected})",
                data.len(),
                width,
                height,
                format
            )));
        }
        let _ = dims;
        Ok(CanonicalImage {
            width,
            height,
            format,
            data,
        })
    }

    /// Like [`CanonicalImage::from_vec`] but also enforces resource limits.
    pub fn from_vec_checked(
        width: u32,
        height: u32,
        format: PixelFormat,
        data: Vec<u8>,
        limits: &Limits,
    ) -> Result<Self> {
        let dims = Dimensions::new(width, height)?;
        limits.validate_dimensions(dims, "image")?;
        Self::from_vec(width, height, format, data)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn dimensions(&self) -> Dimensions {
        // width/height are non-zero by construction.
        Dimensions {
            width: self.width,
            height: self.height,
        }
    }

    pub fn format(&self) -> PixelFormat {
        self.format
    }

    pub fn bytes_per_pixel(&self) -> u64 {
        self.format.bytes_per_pixel()
    }

    pub fn row_bytes(&self) -> Result<u64> {
        self.format.bytes_per_row(self.width)
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// The full-image rectangle.
    pub fn bounds(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        }
    }

    /// Byte range of one pixel row.
    pub fn row_range(&self, y: u32) -> Result<std::ops::Range<usize>> {
        if y >= self.height {
            return Err(Error::invalid(format!("row {y} out of range")));
        }
        let row = crate::checked_usize(self.row_bytes()?)?;
        let start = row
            .checked_mul(y as usize)
            .ok_or_else(|| Error::invalid("row offset overflow"))?;
        Ok(start..start + row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_wrong_buffer_length() {
        let err = CanonicalImage::from_vec(2, 2, PixelFormat::rgba8(), vec![0u8; 15]).unwrap_err();
        assert_eq!(err.kind(), crate::ErrorKind::InvalidArgument);
        assert!(CanonicalImage::from_vec(2, 2, PixelFormat::rgba8(), vec![0u8; 16]).is_ok());
    }

    #[test]
    fn zeroed_allocation_respects_limits() {
        let limits = Limits {
            max_pixels: 3,
            ..Limits::default()
        };
        assert!(CanonicalImage::new_zeroed(2, 2, PixelFormat::rgba8(), &limits).is_err());
        assert!(CanonicalImage::new_zeroed(1, 2, PixelFormat::rgba8(), &limits).is_ok());
    }

    #[test]
    fn row_ranges_are_contiguous() {
        let img =
            CanonicalImage::new_zeroed(4, 3, PixelFormat::rgb8(), &Limits::default()).unwrap();
        assert_eq!(img.row_range(0).unwrap(), 0..12);
        assert_eq!(img.row_range(2).unwrap(), 24..36);
        assert!(img.row_range(3).is_err());
    }
}
