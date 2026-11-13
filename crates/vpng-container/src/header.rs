//! The 64-byte VPNG main header.
//!
//! Layout (all integers little-endian, no implicit padding):
//!
//! ```text
//! offset  size  field
//!      0     8  magic  (56 50 4E 47 0D 0A 1A 0A)
//!      8     2  major version
//!     10     2  minor version
//!     12     4  header size (always 64)
//!     16     4  width
//!     20     4  height
//!     24     1  channel count (must equal the color model's channel count)
//!     25     1  bits per channel (8 or 16)
//!     26     1  color model id
//!     27     1  default codec id
//!     28     4  tile count
//!     32     8  directory offset
//!     40     8  declared file size
//!     48     4  feature flags
//!     52     4  header checksum (CRC32C)
//!     56     8  reserved (must be zero in v0.1)
//! ```
//!
//! The checksum covers bytes `0..52` and `56..64`; the checksum field itself is
//! excluded. This is ADR-0001.

use vpng_core::{BitsPerChannel, ColorModel, Dimensions, Error, PixelFormat, Result};

use crate::checksum;
use crate::leb;

/// File magic: `VPNG\r\n\x1a\n`.
pub const MAGIC: [u8; 8] = [0x56, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Fixed size of the main header.
pub const HEADER_SIZE: usize = 64;

/// Major format version implemented by this build.
pub const FORMAT_MAJOR: u16 = 0;

/// Highest minor format version this build accepts.
pub const FORMAT_MINOR: u16 = 1;

pub const HEADER_CHECKSUM_OFFSET: usize = 52;
pub const HEADER_CHECKSUM_LEN: usize = 4;
pub const RESERVED_OFFSET: usize = 56;

/// Feature-flag bits understood by this build. v0.1 defines none, so any set
/// bit makes the file an unsupported version.
pub const KNOWN_FEATURE_FLAGS: u32 = 0;

/// Parsed main header. Field widths mirror the on-disk layout exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub major: u16,
    pub minor: u16,
    pub width: u32,
    pub height: u32,
    pub color_model: ColorModel,
    pub bits: BitsPerChannel,
    pub default_codec: u8,
    pub tile_count: u32,
    pub directory_offset: u64,
    pub declared_file_size: u64,
    pub feature_flags: u32,
}

impl Header {
    /// Build a header for a new file. `declared_file_size` must already include
    /// the header, directory, and every payload.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        width: u32,
        height: u32,
        format: PixelFormat,
        default_codec: u8,
        tile_count: u32,
        directory_offset: u64,
        declared_file_size: u64,
    ) -> Result<Self> {
        Dimensions::new(width, height)?;
        Ok(Header {
            major: FORMAT_MAJOR,
            minor: FORMAT_MINOR,
            width,
            height,
            color_model: format.color_model,
            bits: format.bits,
            default_codec,
            tile_count,
            directory_offset,
            declared_file_size,
            feature_flags: 0,
        })
    }

    /// The pixel format described by this header (rejects unimplemented models).
    pub fn format(&self) -> Result<PixelFormat> {
        PixelFormat::new(self.color_model, self.bits)
    }

    pub fn dimensions(&self) -> Result<Dimensions> {
        Dimensions::new(self.width, self.height)
    }

    pub fn channels(&self) -> u8 {
        self.color_model.channel_count()
    }

    /// Serialize to exactly [`HEADER_SIZE`] bytes, computing the checksum.
    pub fn to_bytes(&self) -> Result<[u8; HEADER_SIZE]> {
        Dimensions::new(self.width, self.height)?;
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..8].copy_from_slice(&MAGIC);
        buf[8..10].copy_from_slice(&self.major.to_le_bytes());
        buf[10..12].copy_from_slice(&self.minor.to_le_bytes());
        buf[12..16].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        buf[16..20].copy_from_slice(&self.width.to_le_bytes());
        buf[20..24].copy_from_slice(&self.height.to_le_bytes());
        buf[24] = self.color_model.channel_count();
        buf[25] = self.bits.bits();
        buf[26] = self.color_model as u8;
        buf[27] = self.default_codec;
        buf[28..32].copy_from_slice(&self.tile_count.to_le_bytes());
        buf[32..40].copy_from_slice(&self.directory_offset.to_le_bytes());
        buf[40..48].copy_from_slice(&self.declared_file_size.to_le_bytes());
        buf[48..52].copy_from_slice(&self.feature_flags.to_le_bytes());
        // buf[52..56] stays zero until the checksum is written below.
        let sum = checksum::crc32c_parts(&[
            &buf[0..HEADER_CHECKSUM_OFFSET],
            &buf[RESERVED_OFFSET..HEADER_SIZE],
        ]);
        buf[HEADER_CHECKSUM_OFFSET..HEADER_CHECKSUM_OFFSET + HEADER_CHECKSUM_LEN]
            .copy_from_slice(&sum.to_le_bytes());
        Ok(buf)
    }

    pub fn write_to(&self, out: &mut Vec<u8>) -> Result<()> {
        out.extend_from_slice(&self.to_bytes()?);
        Ok(())
    }

    /// Parse and fully validate a header from the start of a file.
    pub fn parse(bytes: &[u8]) -> Result<Header> {
        if bytes.len() < HEADER_SIZE {
            return Err(Error::corrupt(format!(
                "file is {} bytes, shorter than the {HEADER_SIZE}-byte header",
                bytes.len()
            )));
        }
        let buf = &bytes[..HEADER_SIZE];
        if buf[0..8] != MAGIC {
            return Err(Error::corrupt("bad magic bytes; not a VPNG file"));
        }

        let major = leb::read_u16(buf, 8);
        let minor = leb::read_u16(buf, 10);
        if major != FORMAT_MAJOR {
            return Err(Error::unsupported(format!(
                "major version {major} is not supported (this build reads {FORMAT_MAJOR})"
            )));
        }
        if minor > FORMAT_MINOR {
            return Err(Error::unsupported(format!(
                "minor version {minor} is newer than supported {FORMAT_MINOR}"
            )));
        }

        let header_size = leb::read_u32(buf, 12);
        if header_size as usize != HEADER_SIZE {
            return Err(Error::corrupt(format!(
                "header size field is {header_size}, expected {HEADER_SIZE}"
            )));
        }

        let width = leb::read_u32(buf, 16);
        let height = leb::read_u32(buf, 20);
        let channels = buf[24];
        let bits = BitsPerChannel::from_u8(buf[25])?;
        let color_model = ColorModel::from_u8(buf[26])?;
        let default_codec = buf[27];
        let tile_count = leb::read_u32(buf, 28);
        let directory_offset = leb::read_u64(buf, 32);
        let declared_file_size = leb::read_u64(buf, 40);
        let feature_flags = leb::read_u32(buf, 48);

        if channels != color_model.channel_count() {
            return Err(Error::corrupt(format!(
                "header channel count {channels} disagrees with color model {} ({} channels)",
                color_model.name(),
                color_model.channel_count()
            )));
        }

        let stored = leb::read_u32(buf, HEADER_CHECKSUM_OFFSET);
        let computed = checksum::crc32c_parts(&[
            &buf[0..HEADER_CHECKSUM_OFFSET],
            &buf[RESERVED_OFFSET..HEADER_SIZE],
        ]);
        if stored != computed {
            return Err(Error::corrupt(format!(
                "header checksum mismatch: stored {stored:#010x}, computed {computed:#010x}"
            )));
        }

        if buf[RESERVED_OFFSET..HEADER_SIZE].iter().any(|&b| b != 0) {
            return Err(Error::corrupt("reserved header bytes must be zero in v0.1"));
        }

        if feature_flags & !KNOWN_FEATURE_FLAGS != 0 {
            return Err(Error::unsupported(format!(
                "feature flags {feature_flags:#010x} include bits unknown to this build"
            )));
        }

        Ok(Header {
            major,
            minor,
            width,
            height,
            color_model,
            bits,
            default_codec,
            tile_count,
            directory_offset,
            declared_file_size,
            feature_flags,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Header {
        Header::new(
            640,
            480,
            PixelFormat::rgba8(),
            0,
            12,
            HEADER_SIZE as u64,
            4096,
        )
        .unwrap()
    }

    #[test]
    fn header_is_exactly_64_bytes() {
        assert_eq!(HEADER_SIZE, 64);
        let bytes = sample().to_bytes().unwrap();
        assert_eq!(bytes.len(), 64);
    }

    #[test]
    fn roundtrip_exact() {
        let header = sample();
        let bytes = header.to_bytes().unwrap();
        assert_eq!(Header::parse(&bytes).unwrap(), header);
    }

    #[test]
    fn magic_is_first() {
        let bytes = sample().to_bytes().unwrap();
        assert_eq!(&bytes[0..8], &MAGIC);
    }

    #[test]
    fn detects_flipped_byte() {
        let mut bytes = sample().to_bytes().unwrap();
        bytes[16] ^= 0x01; // flip a width bit
        let err = Header::parse(&bytes).unwrap_err();
        assert_eq!(err.kind(), vpng_core::ErrorKind::CorruptFile);
    }

    #[test]
    fn rejects_newer_minor() {
        let mut header = sample();
        header.minor = FORMAT_MINOR + 1;
        let bytes = header.to_bytes().unwrap();
        assert_eq!(
            Header::parse(&bytes).unwrap_err().kind(),
            vpng_core::ErrorKind::UnsupportedFeature
        );
    }

    #[test]
    fn rejects_nonzero_reserved() {
        let mut bytes = sample().to_bytes().unwrap();
        bytes[RESERVED_OFFSET] = 1;
        // Recompute the checksum so only the reserved-field rule can fire.
        let sum = checksum::crc32c_parts(&[
            &bytes[0..HEADER_CHECKSUM_OFFSET],
            &bytes[RESERVED_OFFSET..HEADER_SIZE],
        ]);
        bytes[HEADER_CHECKSUM_OFFSET..HEADER_CHECKSUM_OFFSET + 4]
            .copy_from_slice(&sum.to_le_bytes());
        assert!(Header::parse(&bytes).is_err());
    }

    #[test]
    fn rejects_unknown_codec_default_only_at_use_site() {
        // The header itself stores any codec id; rejection happens when a tile
        // actually claims it. This test pins that the header does not police it.
        let mut header = sample();
        header.default_codec = 200;
        let bytes = header.to_bytes().unwrap();
        assert_eq!(Header::parse(&bytes).unwrap().default_codec, 200);
    }
}
