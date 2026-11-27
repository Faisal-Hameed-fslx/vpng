//! The tile directory.
//!
//! The directory is a flat array of 40-byte entries in canonical order,
//! followed by a 4-byte CRC32C over the entry bytes. ADR-0002 fixes this layout;
//! the main header's `directory_offset` and `tile_count` are the only
//! authoritative source of the directory location and length. There is no
//! separate `TIDX` chunk in v0.1 (ADR-0003).
//!
//! Entry layout (little-endian):
//!
//! ```text
//! offset  size  field
//!      0     4  tile x (pixel origin, not a tile index)
//!      4     4  tile y (pixel origin)
//!      8     2  tile width
//!     10     2  tile height
//!     12     1  codec id
//!     13     1  predictor id
//!     14     2  flags
//!     16     8  payload offset
//!     24     8  compressed length
//!     32     4  uncompressed length
//!     36     4  payload checksum (CRC32C)
//! ```

use vpng_core::{Error, Result};

use crate::checksum;
use crate::leb;

/// Fixed size of one directory entry.
pub const ENTRY_SIZE: usize = 40;

/// CRC32C trailer that follows the entry array.
pub const DIRECTORY_TRAILER_SIZE: usize = 4;

/// The directory entry's `flags` field holds the pyramid level index.
///
/// v0.1 requires the field to be zero. v0.2 gives the low byte to the level:
/// level 0 is the original image, and every higher level holds derived
/// preview pixels. The upper byte is reserved and must be zero.
pub const TILE_FLAG_LEVEL_MASK: u16 = 0x00FF;

/// The bits of the `flags` field that v0.2 and v0.3 reserve and require to
/// be zero.
pub const TILE_FLAG_RESERVED_MASK: u16 = 0xFF00;

/// The bits of the `flags` field that v0.4 gives to the **layer index**.
///
/// The layer index sits immediately above the pyramid level: bits 8..12.
/// Four bits give sixteen layers, which is the most a container may hold.
pub const TILE_FLAG_LAYER_MASK: u16 = 0x0F00;

/// Where the layer index starts inside the `flags` field.
pub const TILE_FLAG_LAYER_SHIFT: u32 = 8;

/// The bits of the `flags` field that v0.4 still reserves and requires to be
/// zero.
pub const TILE_FLAG_RESERVED_V0_4_MASK: u16 = 0xF000;

/// The largest number of layers a container may hold.
pub const MAX_LAYERS: u16 = 16;

/// One independently decodable tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub x: u32,
    pub y: u32,
    pub width: u16,
    pub height: u16,
    pub codec_id: u8,
    pub predictor_id: u8,
    pub flags: u16,
    pub payload_offset: u64,
    pub compressed_length: u64,
    pub uncompressed_length: u32,
    pub payload_checksum: u32,
}

impl DirectoryEntry {
    /// The pyramid level this tile belongs to.
    ///
    /// Level 0 is the original image. Every other level holds derived preview
    /// pixels and is never used to reconstruct the original.
    pub fn level(&self) -> u16 {
        self.flags & TILE_FLAG_LEVEL_MASK
    }

    /// The layer this tile belongs to.
    ///
    /// Layer 0 is the base layer: the image itself. Every other layer is one
    /// of the named grids a v0.4 container holds beside it.
    ///
    /// A v0.1, v0.2 or v0.3 file has no layer index, and its entries all
    /// belong to layer 0 — which is exactly what the reserved bits being zero
    /// says.
    pub fn layer(&self) -> u16 {
        (self.flags & TILE_FLAG_LAYER_MASK) >> TILE_FLAG_LAYER_SHIFT
    }

    /// Whether the tile holds derived preview pixels rather than original ones.
    pub fn is_derived(&self) -> bool {
        self.level() != 0
    }

    /// Build the `flags` field for a level index.
    pub fn flags_for_level(level: u16) -> Result<u16> {
        if level > TILE_FLAG_LEVEL_MASK {
            return Err(Error::invalid(format!(
                "pyramid level {level} does not fit the directory entry's flag field"
            )));
        }
        Ok(level & TILE_FLAG_LEVEL_MASK)
    }

    /// Build the `flags` field for a level and a layer index.
    pub fn flags_for_level_and_layer(level: u16, layer: u16) -> Result<u16> {
        if level > TILE_FLAG_LEVEL_MASK {
            return Err(Error::invalid(format!(
                "pyramid level {level} does not fit the directory entry's flag field"
            )));
        }
        if layer >= MAX_LAYERS {
            return Err(Error::invalid(format!(
                "layer {layer} does not fit the directory entry's flag field"
            )));
        }
        Ok((level & TILE_FLAG_LEVEL_MASK)
            | ((layer << TILE_FLAG_LAYER_SHIFT) & TILE_FLAG_LAYER_MASK))
    }

    /// Exclusive right edge of the tile in pixels.
    pub fn right(&self) -> Result<u32> {
        self.x
            .checked_add(self.width as u32)
            .ok_or_else(|| Error::corrupt("tile x + width overflows"))
    }

    /// Exclusive bottom edge of the tile in pixels.
    pub fn bottom(&self) -> Result<u32> {
        self.y
            .checked_add(self.height as u32)
            .ok_or_else(|| Error::corrupt("tile y + height overflows"))
    }

    pub fn write_to(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.push(self.codec_id);
        out.push(self.predictor_id);
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&self.payload_offset.to_le_bytes());
        out.extend_from_slice(&self.compressed_length.to_le_bytes());
        out.extend_from_slice(&self.uncompressed_length.to_le_bytes());
        out.extend_from_slice(&self.payload_checksum.to_le_bytes());
    }

    pub fn parse(buf: &[u8]) -> Result<Self> {
        if buf.len() < ENTRY_SIZE {
            return Err(Error::corrupt("directory entry is truncated"));
        }
        Ok(DirectoryEntry {
            x: leb::read_u32(buf, 0),
            y: leb::read_u32(buf, 4),
            width: leb::read_u16(buf, 8),
            height: leb::read_u16(buf, 10),
            codec_id: buf[12],
            predictor_id: buf[13],
            flags: leb::read_u16(buf, 14),
            payload_offset: leb::read_u64(buf, 16),
            compressed_length: leb::read_u64(buf, 24),
            uncompressed_length: leb::read_u32(buf, 32),
            payload_checksum: leb::read_u32(buf, 36),
        })
    }
}

/// The complete tile directory.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Directory {
    pub entries: Vec<DirectoryEntry>,
}

impl Directory {
    pub fn new(entries: Vec<DirectoryEntry>) -> Self {
        Directory { entries }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total encoded size of a directory with `tile_count` entries.
    pub fn total_size(tile_count: u32) -> Result<usize> {
        let body = vpng_core::checked_mul(tile_count as u64, ENTRY_SIZE as u64)?;
        let total = vpng_core::checked_add(body, DIRECTORY_TRAILER_SIZE as u64)?;
        vpng_core::checked_usize(total)
    }

    /// Serialize entries plus the CRC32C trailer.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let size = Self::total_size(self.entries.len() as u32)?;
        let mut out = Vec::with_capacity(size);
        for entry in &self.entries {
            entry.write_to(&mut out);
        }
        let crc = checksum::crc32c(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        Ok(out)
    }

    /// Parse and checksum-verify a directory slice of exactly the expected size.
    pub fn decode(bytes: &[u8], tile_count: u32) -> Result<Directory> {
        let expected = Self::total_size(tile_count)?;
        if bytes.len() != expected {
            return Err(Error::corrupt(format!(
                "directory slice is {} bytes, expected {expected}",
                bytes.len()
            )));
        }
        let body_len = expected - DIRECTORY_TRAILER_SIZE;
        let stored = leb::read_u32(bytes, body_len);
        let computed = checksum::crc32c(&bytes[..body_len]);
        if stored != computed {
            return Err(Error::corrupt(format!(
                "directory checksum mismatch: stored {stored:#010x}, computed {computed:#010x}"
            )));
        }
        let mut entries = Vec::with_capacity(tile_count as usize);
        for i in 0..tile_count as usize {
            let start = i * ENTRY_SIZE;
            entries.push(DirectoryEntry::parse(&bytes[start..start + ENTRY_SIZE])?);
        }
        Ok(Directory { entries })
    }

    /// Sort into the canonical `(layer, level, y, x)` order used before
    /// encoding.
    ///
    /// v0.1 files have one layer and one level, so this is the `(y, x)` order
    /// v0.1 defines. v0.2 groups the levels, each in `(y, x)` order. v0.4
    /// groups the layers first, and each layer's levels inside it.
    pub fn sort_canonical(&mut self) {
        self.entries
            .sort_by_key(|e| (e.layer(), e.level(), e.y, e.x));
    }

    /// Sort a second copy into canonical order and return it, leaving `self` alone.
    pub fn sorted(&self) -> Directory {
        let mut clone = self.clone();
        clone.sort_canonical();
        clone
    }

    /// Check every entry's `flags` field against the format version.
    ///
    /// v0.1 requires the field to be zero. v0.2 gives the low byte to the
    /// pyramid level and requires the rest to be zero. v0.4 gives four more
    /// bits to the layer index and reserves the top four.
    pub fn validate_flags(&self, minor: u16) -> Result<()> {
        for (index, entry) in self.entries.iter().enumerate() {
            if minor < crate::header::MINOR_V0_2 {
                if entry.flags != 0 {
                    return Err(Error::corrupt(format!(
                        "tile {index} sets flags {:#06x}, which v0.1 does not define",
                        entry.flags
                    )));
                }
            } else if minor < crate::header::MINOR_V0_4 {
                if entry.flags & TILE_FLAG_RESERVED_MASK != 0 {
                    return Err(Error::corrupt(format!(
                        "tile {index} sets reserved flag bits {:#06x}",
                        entry.flags & TILE_FLAG_RESERVED_MASK
                    )));
                }
            } else if entry.flags & TILE_FLAG_RESERVED_V0_4_MASK != 0 {
                return Err(Error::corrupt(format!(
                    "tile {index} sets reserved flag bits {:#06x}",
                    entry.flags & TILE_FLAG_RESERVED_V0_4_MASK
                )));
            }
        }
        Ok(())
    }

    /// Verify every payload range lies inside the file and that no two ranges
    /// overlap.
    pub fn validate_payload_ranges(&self, file_size: u64) -> Result<()> {
        for (index, entry) in self.entries.iter().enumerate() {
            vpng_core::checked_range(entry.payload_offset, entry.compressed_length, file_size)
                .map_err(|e| Error::corrupt(format!("tile {index} payload range invalid: {e}")))?;
        }

        let mut ranges: Vec<(u64, u64)> = self
            .entries
            .iter()
            .map(|e| {
                (
                    e.payload_offset,
                    e.payload_offset.saturating_add(e.compressed_length),
                )
            })
            .collect();
        ranges.sort_unstable();
        for window in ranges.windows(2) {
            let (_, prev_end) = window[0];
            let (next_start, _) = window[1];
            if next_start < prev_end {
                return Err(Error::corrupt(
                    "tile payload ranges overlap; the specification forbids shared ranges",
                ));
            }
        }
        Ok(())
    }

    /// Verify that each entry's declared uncompressed length matches its tile
    /// geometry for the given bytes-per-pixel.
    pub fn validate_tile_sizes(&self, bytes_per_pixel: u64) -> Result<()> {
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.width == 0 || entry.height == 0 {
                return Err(Error::corrupt(format!(
                    "tile {index} has zero width or height"
                )));
            }
            let pixels = vpng_core::checked_mul(entry.width as u64, entry.height as u64)?;
            let expected = vpng_core::checked_mul(pixels, bytes_per_pixel)?;
            if expected != entry.uncompressed_length as u64 {
                return Err(Error::corrupt(format!(
                    "tile {index} declares {} uncompressed bytes, expected {expected}",
                    entry.uncompressed_length
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(x: u32, y: u32) -> DirectoryEntry {
        DirectoryEntry {
            x,
            y,
            width: 256,
            height: 256,
            codec_id: 0,
            predictor_id: 0,
            flags: 0,
            payload_offset: 0,
            compressed_length: 262_144,
            uncompressed_length: 262_144,
            payload_checksum: 0,
        }
    }

    #[test]
    fn entry_is_exactly_40_bytes() {
        assert_eq!(ENTRY_SIZE, 40);
        let mut out = Vec::new();
        entry(0, 0).write_to(&mut out);
        assert_eq!(out.len(), 40);
        assert_eq!(DirectoryEntry::parse(&out).unwrap(), entry(0, 0));
    }

    #[test]
    fn directory_size_math() {
        assert_eq!(Directory::total_size(0).unwrap(), 4);
        assert_eq!(Directory::total_size(2).unwrap(), 84);
    }

    #[test]
    fn roundtrip_with_checksum() {
        let dir = Directory::new(vec![entry(0, 0), entry(256, 0)]);
        let bytes = dir.encode().unwrap();
        assert_eq!(bytes.len(), Directory::total_size(2).unwrap());
        assert_eq!(Directory::decode(&bytes, 2).unwrap(), dir);
    }

    #[test]
    fn detects_corruption() {
        let dir = Directory::new(vec![entry(0, 0)]);
        let mut bytes = dir.encode().unwrap();
        bytes[0] ^= 0xff;
        assert!(Directory::decode(&bytes, 1).is_err());
    }

    #[test]
    fn detects_overlapping_payloads() {
        let mut a = entry(0, 0);
        a.payload_offset = 64;
        a.compressed_length = 100;
        let mut b = entry(256, 0);
        b.payload_offset = 100;
        b.compressed_length = 50;
        let dir = Directory::new(vec![a, b]);
        assert!(dir.validate_payload_ranges(1024).is_err());
    }

    #[test]
    fn accepts_adjacent_payloads() {
        let mut a = entry(0, 0);
        a.payload_offset = 64;
        a.compressed_length = 100;
        let mut b = entry(256, 0);
        b.payload_offset = 164;
        b.compressed_length = 50;
        let dir = Directory::new(vec![a, b]);
        assert!(dir.validate_payload_ranges(1024).is_ok());
    }

    #[test]
    fn tile_size_mismatch_is_corrupt() {
        let mut e = entry(0, 0);
        e.width = 10;
        e.height = 10;
        e.uncompressed_length = 999;
        let dir = Directory::new(vec![e]);
        assert!(dir.validate_tile_sizes(4).is_err());
    }
}
