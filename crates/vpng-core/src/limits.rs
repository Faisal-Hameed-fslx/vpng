//! Configurable resource limits.
//!
//! Every entry point that touches untrusted input takes a [`Limits`]. Nothing is
//! allocated from a declared length before the corresponding limit is checked.

use crate::error::{Error, Result};
use crate::geom::Dimensions;

/// Hard bounds applied to parsing, decoding and encoding.
///
/// The defaults are deliberately generous but finite; a decompression bomb must
/// fail before it can allocate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_tile_count: u32,
    /// Maximum compressed bytes for a single tile payload.
    pub max_tile_payload_bytes: u64,
    /// Maximum total decompressed bytes produced while decoding one file.
    pub max_decompressed_bytes: u64,
    /// Maximum size of a `.vpng` file that will be opened.
    pub max_file_bytes: u64,
    /// Maximum ratio of uncompressed to compressed bytes per tile.
    pub max_expansion_ratio: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_width: 1 << 16,
            max_height: 1 << 16,
            max_pixels: 1 << 30,
            max_tile_count: 1 << 24,
            max_tile_payload_bytes: 1 << 28,
            max_decompressed_bytes: 1 << 31,
            max_file_bytes: 1 << 34,
            max_expansion_ratio: 4096,
        }
    }
}

impl Limits {
    pub fn validate_dimensions(&self, dims: Dimensions, context: &str) -> Result<()> {
        if dims.width > self.max_width || dims.height > self.max_height {
            return Err(Error::resource_limit(format!(
                "{context}: dimensions {}x{} exceed limit {}x{}",
                dims.width, dims.height, self.max_width, self.max_height
            )));
        }
        let pixels = dims.pixel_count()?;
        if pixels > self.max_pixels {
            return Err(Error::resource_limit(format!(
                "{context}: {pixels} pixels exceed limit {}",
                self.max_pixels
            )));
        }
        Ok(())
    }

    pub fn validate_tile_count(&self, count: u32) -> Result<()> {
        if count > self.max_tile_count {
            return Err(Error::resource_limit(format!(
                "tile count {count} exceeds limit {}",
                self.max_tile_count
            )));
        }
        Ok(())
    }

    pub fn validate_file_size(&self, size: u64) -> Result<()> {
        if size > self.max_file_bytes {
            return Err(Error::resource_limit(format!(
                "file size {size} exceeds limit {}",
                self.max_file_bytes
            )));
        }
        Ok(())
    }

    /// Guard a single tile before any buffer of `uncompressed` bytes is created.
    pub fn validate_tile_payload(&self, compressed: u64, uncompressed: u64) -> Result<()> {
        if compressed > self.max_tile_payload_bytes {
            return Err(Error::resource_limit(format!(
                "compressed tile payload {compressed} exceeds limit {}",
                self.max_tile_payload_bytes
            )));
        }
        if uncompressed > self.max_decompressed_bytes {
            return Err(Error::resource_limit(format!(
                "decompressed tile {uncompressed} exceeds limit {}",
                self.max_decompressed_bytes
            )));
        }
        if compressed == 0 {
            if uncompressed != 0 {
                return Err(Error::corrupt(
                    "tile declares zero compressed bytes but non-zero uncompressed bytes",
                ));
            }
        } else {
            let budget = compressed.saturating_mul(self.max_expansion_ratio);
            if uncompressed > budget {
                return Err(Error::resource_limit(format!(
                    "tile expansion ratio too large: {uncompressed} uncompressed from {compressed} compressed"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversize_dimensions() {
        let limits = Limits {
            max_width: 100,
            max_height: 100,
            ..Limits::default()
        };
        let dims = Dimensions::new(200, 10).unwrap();
        assert!(limits.validate_dimensions(dims, "test").is_err());
        let ok = Dimensions::new(50, 50).unwrap();
        assert!(limits.validate_dimensions(ok, "test").is_ok());
    }

    #[test]
    fn expansion_bomb_is_rejected() {
        let limits = Limits {
            max_expansion_ratio: 4,
            ..Limits::default()
        };
        assert!(limits.validate_tile_payload(10, 40).is_ok());
        assert!(limits.validate_tile_payload(10, 41).is_err());
    }

    #[test]
    fn zero_compressed_nonzero_uncompressed_is_corrupt() {
        let limits = Limits::default();
        let err = limits.validate_tile_payload(0, 5).unwrap_err();
        assert_eq!(err.kind(), crate::ErrorKind::CorruptFile);
    }
}
