//! The main header: 64 bytes, or 72 from format v0.5.
//!
//! Layout (all integers little-endian, no implicit padding):
//!
//! ```text
//! offset  size  field
//!      0     8  magic  (56 50 4E 47 0D 0A 1A 0A)
//!      8     2  major version
//!     10     2  minor version
//!     12     4  header size (64, or 72 from v0.5)
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
//!     56     4  layer table offset (zero in v0.1 through v0.3)
//!     60     4  layer table length (zero in v0.1 through v0.3)
//!     64     4  GPU table offset (only in the 72-byte header of v0.5)
//!     68     4  GPU table length (only in the 72-byte header of v0.5)
//! ```
//!
//! The checksum covers bytes `0..52` and `56..header_size`; the checksum
//! field itself is excluded. This is ADR-0001.
//!
//! The last eight bytes were reserved in v0.1. v0.4 gives them to the layer
//! table's location: the table sits between the directory and the payloads,
//! and the header says exactly where. A v0.1, v0.2 or v0.3 file leaves both
//! fields zero, which is why the bytes did not have to move.
//!
//! v0.5 needs eight more bytes and the header has none left, so the header
//! **grows** — through the `header size` field, which is what that field is
//! for. A v0.5 file's header is 72 bytes and its last eight point at the GPU
//! payload table. The checksum rule is unchanged in shape: it covers
//! `0..52` and `56..header_size`, so for a 64-byte header it covers exactly
//! what it always did and no existing file's checksum changes.

use vpng_core::{BitsPerChannel, ColorModel, Dimensions, Error, PixelFormat, Result};

use crate::checksum;
use crate::leb;

/// File magic: `VPNG\r\n\x1a\n`.
pub const MAGIC: [u8; 8] = [0x56, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Size of the main header in v0.1 through v0.4.
pub const HEADER_SIZE: usize = 64;

/// Size of the main header in v0.5, which points at a GPU payload table.
pub const HEADER_SIZE_V0_5: usize = 72;

/// Major format version implemented by this build.
pub const FORMAT_MAJOR: u16 = 0;

/// The minor version of the first released format.
pub const MINOR_V0_1: u16 = 1;

/// The minor version that adds the preview pyramid and adaptive tiling.
///
/// v0.2 keeps the 64-byte header and the 40-byte directory entry exactly as
/// v0.1 defines them. It changes two *rules*: the directory entry's `flags`
/// field carries the pyramid level, and a level's tiles may have different
/// sizes rather than forming one uniform grid.
pub const MINOR_V0_2: u16 = 2;

/// The minor version that promotes the dictionary and context-adaptive codecs.
///
/// v0.3 keeps the 64-byte header and the 40-byte directory entry exactly as
/// v0.1 and v0.2 define them. It changes one *rule*: a directory entry may
/// name codec id 4 (LZ77 dictionary), 5 (predictor over a dictionary) or 6
/// (context-adaptive entropy). A v0.1 or v0.2 file that names one of them is
/// malformed.
pub const MINOR_V0_3: u16 = 3;

/// The minor version that adds named layers and masks.
///
/// v0.4 keeps the 64-byte header and the 40-byte directory entry exactly as
/// v0.1 defines them. It changes two *rules* and gives the header's last
/// eight bytes a job:
///
/// * the directory entry's `flags` field carries the **layer index** in bits
///   8..12, next to the pyramid level in bits 0..8;
/// * the header's last eight bytes point at the **layer table**, a list of
///   named grids that sits between the directory and the payloads.
///
/// A file that has no layer table is not a v0.4 file: the minor version
/// states what the file *contains*.
pub const MINOR_V0_4: u16 = 4;

/// The minor version that adds the optional GPU payload table.
///
/// v0.5 grows the header to 72 bytes and gives the last eight of the new
/// bytes a job: they point at the **GPU payload table**, which gives every
/// tile an optional *lossy* GPU sibling of its lossless payload.
///
/// The GPU payload is never a substitute for the lossless payload, and a
/// directory entry may never name a GPU block codec.
pub const MINOR_V0_5: u16 = 5;

/// Highest minor format version this build accepts.
pub const FORMAT_MINOR: u16 = MINOR_V0_5;

pub const HEADER_CHECKSUM_OFFSET: usize = 52;
pub const HEADER_CHECKSUM_LEN: usize = 4;
pub const RESERVED_OFFSET: usize = 56;

/// The header's layer-table offset field (bytes 56..60).
pub const LAYER_TABLE_OFFSET_OFFSET: usize = 56;

/// The header's layer-table length field (bytes 60..64).
pub const LAYER_TABLE_LENGTH_OFFSET: usize = 60;

/// The header's GPU-table offset field (bytes 64..68, v0.5 only).
pub const GPU_TABLE_OFFSET_OFFSET: usize = 64;

/// The header's GPU-table length field (bytes 68..72, v0.5 only).
pub const GPU_TABLE_LENGTH_OFFSET: usize = 68;

/// The file contains derived preview levels above the original image.
///
/// This is a v0.2 feature flag. A v0.1 file must have no feature flags set at
/// all, so a v0.1 reader rejects a pyramid file on two counts: the minor
/// version and the unknown flag.
pub const FEATURE_PYRAMID: u32 = 1 << 0;

/// The file contains more than one named layer.
///
/// This is a v0.4 feature flag. A v0.1, v0.2 or v0.3 file must have no
/// feature flags beyond [`FEATURE_PYRAMID`], so an older reader rejects a
/// layered file on two counts: the minor version and the unknown flag.
pub const FEATURE_LAYERS: u32 = 1 << 1;

/// The file carries a GPU payload table.
///
/// This is a v0.5 feature flag. It is **orthogonal** to the other two: a file
/// may have a GPU payload table, a layer table, a preview pyramid and
/// promoted codecs at the same time, because the GPU payload is a sibling of
/// the lossless payload and not a replacement for it.
pub const FEATURE_GPU: u32 = 1 << 2;

/// Feature-flag bits understood by this build.
///
/// v0.1 defines none, so a v0.1 file with any bit set is rejected. v0.2
/// defines [`FEATURE_PYRAMID`]; v0.4 defines [`FEATURE_LAYERS`]; v0.5 defines
/// [`FEATURE_GPU`].
pub const KNOWN_FEATURE_FLAGS: u32 = FEATURE_PYRAMID | FEATURE_LAYERS | FEATURE_GPU;

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
    /// Where the layer table starts, or zero when the file has none.
    pub layer_table_offset: u32,
    /// The layer table's length in bytes, or zero when the file has none.
    pub layer_table_length: u32,
    /// Where the GPU payload table starts, or zero when the file has none.
    pub gpu_table_offset: u32,
    /// The GPU payload table's length in bytes, or zero when the file has none.
    pub gpu_table_length: u32,
}

impl Header {
    /// Build a header for a new file. `declared_file_size` must already include
    /// the header, directory, and every payload.
    ///
    /// The result is a **v0.1** header. v0.1 is the format every file this
    /// project has written so far uses, and the golden fixtures pin its exact
    /// bytes, so the default writer does not move the version on its own.
    /// Use [`Header::new_v2`] for a file that carries a preview pyramid or an
    /// adaptive (variable-size) partition, [`Header::new_v3`] for a file
    /// whose directory names codec id 4, 5 or 6, [`Header::new_v4`] for a
    /// file that carries a layer table, and [`Header::new_v5`] for a file that
    /// carries a GPU payload table.
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
            minor: MINOR_V0_1,
            width,
            height,
            color_model: format.color_model,
            bits: format.bits,
            default_codec,
            tile_count,
            directory_offset,
            declared_file_size,
            feature_flags: 0,
            layer_table_offset: 0,
            layer_table_length: 0,
            gpu_table_offset: 0,
            gpu_table_length: 0,
        })
    }

    /// Build a **v0.2** header for a new file.
    ///
    /// The layout is identical to v0.1 — the same 64 bytes, the same field
    /// offsets — and only the minor version differs. The caller sets the
    /// feature flags with [`Header::with_feature_flags`].
    #[allow(clippy::too_many_arguments)]
    pub fn new_v2(
        width: u32,
        height: u32,
        format: PixelFormat,
        default_codec: u8,
        tile_count: u32,
        directory_offset: u64,
        declared_file_size: u64,
    ) -> Result<Self> {
        let mut header = Header::new(
            width,
            height,
            format,
            default_codec,
            tile_count,
            directory_offset,
            declared_file_size,
        )?;
        header.minor = MINOR_V0_2;
        Ok(header)
    }

    /// Set the feature flags.
    ///
    /// The flags are part of the header checksum, so a file that claims a
    /// pyramid cannot be edited into one without rewriting the header.
    pub fn with_feature_flags(mut self, flags: u32) -> Self {
        self.feature_flags = flags;
        self
    }

    /// Build a **v0.3** header for a new file.
    ///
    /// The layout is identical to v0.1 and v0.2 — the same 64 bytes, the same
    /// field offsets — and only the minor version differs. v0.3 is the
    /// version that permits codec ids 4, 5 and 6 in the directory.
    #[allow(clippy::too_many_arguments)]
    pub fn new_v3(
        width: u32,
        height: u32,
        format: PixelFormat,
        default_codec: u8,
        tile_count: u32,
        directory_offset: u64,
        declared_file_size: u64,
    ) -> Result<Self> {
        let mut header = Header::new(
            width,
            height,
            format,
            default_codec,
            tile_count,
            directory_offset,
            declared_file_size,
        )?;
        header.minor = MINOR_V0_3;
        Ok(header)
    }

    /// Whether the file declares derived preview levels.
    pub fn has_pyramid(&self) -> bool {
        self.feature_flags & FEATURE_PYRAMID != 0
    }

    /// Whether this header describes a v0.2 file.
    pub fn is_v2(&self) -> bool {
        self.minor >= MINOR_V0_2
    }

    /// Build a **v0.4** header for a new file.
    ///
    /// The layout is identical to v0.1 — the same 64 bytes, the same field
    /// offsets — and only the minor version differs. v0.4 is the version that
    /// permits a layer table and a layer index in the directory entry's
    /// `flags` field.
    #[allow(clippy::too_many_arguments)]
    pub fn new_v4(
        width: u32,
        height: u32,
        format: PixelFormat,
        default_codec: u8,
        tile_count: u32,
        directory_offset: u64,
        declared_file_size: u64,
    ) -> Result<Self> {
        let mut header = Header::new(
            width,
            height,
            format,
            default_codec,
            tile_count,
            directory_offset,
            declared_file_size,
        )?;
        header.minor = MINOR_V0_4;
        Ok(header)
    }

    /// Record where the layer table is.
    ///
    /// The table sits between the directory and the payloads. The length may
    /// be zero, which means "this file has no layer table"; the offset is
    /// then ignored.
    pub fn with_layer_table(mut self, offset: u32, length: u32) -> Self {
        self.layer_table_offset = offset;
        self.layer_table_length = length;
        self
    }

    /// Whether the file declares a layer table.
    pub fn has_layers(&self) -> bool {
        self.layer_table_length != 0
    }

    /// Build a **v0.5** header for a new file.
    ///
    /// The header is **72 bytes** from v0.5, and the last eight point at the
    /// GPU payload table. The caller records the table's location with
    /// [`Header::with_gpu_table`].
    #[allow(clippy::too_many_arguments)]
    pub fn new_v5(
        width: u32,
        height: u32,
        format: PixelFormat,
        default_codec: u8,
        tile_count: u32,
        directory_offset: u64,
        declared_file_size: u64,
    ) -> Result<Self> {
        let mut header = Header::new(
            width,
            height,
            format,
            default_codec,
            tile_count,
            directory_offset,
            declared_file_size,
        )?;
        header.minor = MINOR_V0_5;
        Ok(header)
    }

    /// Record where the GPU payload table is.
    ///
    /// The table sits with the other tables: after the directory, and after
    /// the layer table when there is one.
    pub fn with_gpu_table(mut self, offset: u32, length: u32) -> Self {
        self.gpu_table_offset = offset;
        self.gpu_table_length = length;
        self
    }

    /// Whether the file declares a GPU payload table.
    pub fn has_gpu(&self) -> bool {
        self.gpu_table_length != 0
    }

    /// How many bytes this header occupies.
    ///
    /// A v0.5 file's header is 72 bytes; every earlier file's is 64. The
    /// header size field says so on disk, and this says the same thing from
    /// the parsed fields.
    pub fn header_size(&self) -> usize {
        if self.minor >= MINOR_V0_5 {
            HEADER_SIZE_V0_5
        } else {
            HEADER_SIZE
        }
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

    /// The header size a prefix of a file declares.
    ///
    /// The `header size` field is at bytes 12..16, so a reader that has read
    /// the first sixteen bytes can ask how long the header is before reading
    /// the rest of it. This is what lets a range reader fetch 64 bytes for an
    /// old file and 72 for a v0.5 one.
    pub fn header_size_from_prefix(bytes: &[u8]) -> Result<usize> {
        if bytes.len() < 16 {
            return Err(Error::corrupt(format!(
                "file is {} bytes, too short to hold a header size",
                bytes.len()
            )));
        }
        let size = leb::read_u32(bytes, 12) as usize;
        match size {
            HEADER_SIZE => Ok(HEADER_SIZE),
            HEADER_SIZE_V0_5 => Ok(HEADER_SIZE_V0_5),
            other => Err(Error::corrupt(format!(
                "header size field is {other}, expected {HEADER_SIZE} or {HEADER_SIZE_V0_5}"
            ))),
        }
    }

    /// Serialize to exactly [`Self::header_size`] bytes, computing the
    /// checksum.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Dimensions::new(self.width, self.height)?;
        let size = self.header_size();
        let mut buf = vec![0u8; size];
        buf[0..8].copy_from_slice(&MAGIC);
        buf[8..10].copy_from_slice(&self.major.to_le_bytes());
        buf[10..12].copy_from_slice(&self.minor.to_le_bytes());
        buf[12..16].copy_from_slice(&(size as u32).to_le_bytes());
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
        buf[LAYER_TABLE_OFFSET_OFFSET..LAYER_TABLE_OFFSET_OFFSET + 4]
            .copy_from_slice(&self.layer_table_offset.to_le_bytes());
        buf[LAYER_TABLE_LENGTH_OFFSET..LAYER_TABLE_LENGTH_OFFSET + 4]
            .copy_from_slice(&self.layer_table_length.to_le_bytes());
        if size >= HEADER_SIZE_V0_5 {
            buf[GPU_TABLE_OFFSET_OFFSET..GPU_TABLE_OFFSET_OFFSET + 4]
                .copy_from_slice(&self.gpu_table_offset.to_le_bytes());
            buf[GPU_TABLE_LENGTH_OFFSET..GPU_TABLE_LENGTH_OFFSET + 4]
                .copy_from_slice(&self.gpu_table_length.to_le_bytes());
        }
        // buf[52..56] stays zero until the checksum is written below.
        let sum =
            checksum::crc32c_parts(&[&buf[0..HEADER_CHECKSUM_OFFSET], &buf[RESERVED_OFFSET..size]]);
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

        // The header size is known before the rest of the header is read, so
        // a reader can ask for exactly the bytes the file needs.
        let size = Header::header_size_from_prefix(bytes)?;
        if bytes.len() < size {
            return Err(Error::corrupt(format!(
                "file is {} bytes, shorter than the {size}-byte header",
                bytes.len()
            )));
        }
        let buf = &bytes[..size];

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

        // The header size and the minor version must agree. A v0.5 file has
        // a 72-byte header because it points at a GPU payload table, and
        // every earlier file has a 64-byte header.
        if (minor >= MINOR_V0_5) != (size == HEADER_SIZE_V0_5) {
            return Err(Error::corrupt(format!(
                "a v0.{minor} file must have a {}-byte header, not {size}",
                if minor >= MINOR_V0_5 {
                    HEADER_SIZE_V0_5
                } else {
                    HEADER_SIZE
                }
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
        let layer_table_offset = leb::read_u32(buf, LAYER_TABLE_OFFSET_OFFSET);
        let layer_table_length = leb::read_u32(buf, LAYER_TABLE_LENGTH_OFFSET);
        // The GPU table's location only exists in the 72-byte header. A
        // 64-byte header has no room for it, which is itself the rule that
        // an older file cannot carry a GPU table.
        let (gpu_table_offset, gpu_table_length) = if size >= HEADER_SIZE_V0_5 {
            (
                leb::read_u32(buf, GPU_TABLE_OFFSET_OFFSET),
                leb::read_u32(buf, GPU_TABLE_LENGTH_OFFSET),
            )
        } else {
            (0, 0)
        };

        if channels != color_model.channel_count() {
            return Err(Error::corrupt(format!(
                "header channel count {channels} disagrees with color model {} ({} channels)",
                color_model.name(),
                color_model.channel_count()
            )));
        }

        let stored = leb::read_u32(buf, HEADER_CHECKSUM_OFFSET);
        let computed =
            checksum::crc32c_parts(&[&buf[0..HEADER_CHECKSUM_OFFSET], &buf[RESERVED_OFFSET..size]]);
        if stored != computed {
            return Err(Error::corrupt(format!(
                "header checksum mismatch: stored {stored:#010x}, computed {computed:#010x}"
            )));
        }

        if buf[RESERVED_OFFSET..size].iter().any(|&b| b != 0) && minor < MINOR_V0_4 {
            return Err(Error::corrupt(
                "reserved header bytes must be zero before v0.4",
            ));
        }

        if feature_flags & !KNOWN_FEATURE_FLAGS != 0 {
            return Err(Error::unsupported(format!(
                "feature flags {feature_flags:#010x} include bits unknown to this build"
            )));
        }

        // v0.1 defines no feature flags at all, so a file that claims one is
        // not a v0.1 file even if its minor version says it is. The check is
        // separate from the "unknown bits" check above because the two reject
        // different mistakes: this one rejects a *known* flag in a version
        // that did not define it.
        if minor < MINOR_V0_2 && feature_flags != 0 {
            return Err(Error::unsupported(format!(
                "feature flags {feature_flags:#010x} are not defined in v0.1"
            )));
        }

        // The layer table arrived in v0.4. An older file must not point at
        // one, and a file that points at one must say so with the flag.
        if minor < MINOR_V0_4 && (layer_table_offset != 0 || layer_table_length != 0) {
            return Err(Error::corrupt(format!(
                "a v0.{minor} file must not carry a layer table"
            )));
        }
        if minor < MINOR_V0_4 && feature_flags & FEATURE_LAYERS != 0 {
            return Err(Error::unsupported(format!(
                "the layer feature flag is not defined in v0.{minor}"
            )));
        }
        let has_layer_table = layer_table_length != 0;
        if has_layer_table && layer_table_offset < size as u32 {
            return Err(Error::corrupt(
                "layer table offset points inside the fixed header",
            ));
        }
        if has_layer_table != (feature_flags & FEATURE_LAYERS != 0) {
            return Err(Error::corrupt(
                "the header's layer flag disagrees with the layer table it points at",
            ));
        }
        if feature_flags & FEATURE_LAYERS != 0 && feature_flags & FEATURE_PYRAMID != 0 {
            return Err(Error::corrupt(
                "a file with layers may not also have a preview pyramid in v0.4",
            ));
        }

        // The GPU payload table arrived in v0.5. The same two rules as the
        // layer table apply: an older file must not point at one, and a file
        // that points at one must say so with the flag.
        if minor < MINOR_V0_5 && (gpu_table_offset != 0 || gpu_table_length != 0) {
            return Err(Error::corrupt(format!(
                "a v0.{minor} file must not carry a GPU payload table"
            )));
        }
        if minor < MINOR_V0_5 && feature_flags & FEATURE_GPU != 0 {
            return Err(Error::unsupported(format!(
                "the GPU feature flag is not defined in v0.{minor}"
            )));
        }
        let has_gpu_table = gpu_table_length != 0;
        if has_gpu_table && gpu_table_offset < size as u32 {
            return Err(Error::corrupt(
                "GPU table offset points inside the fixed header",
            ));
        }
        if has_gpu_table != (feature_flags & FEATURE_GPU != 0) {
            return Err(Error::corrupt(
                "the header's GPU flag disagrees with the GPU table it points at",
            ));
        }
        if minor >= MINOR_V0_5 && !has_gpu_table {
            return Err(Error::corrupt("a v0.5 file must carry a GPU payload table"));
        }

        // The dimensions must be non-zero, exactly as `Header::new` and
        // `Header::to_bytes` require. A parser that accepted a zero
        // dimension would hand back a header that cannot be serialized
        // again, so `parse` and `to_bytes` would disagree about the same
        // bytes: one implementation would accept a file the other refuses
        // to write. The check comes after every other header rule so that
        // the error a caller sees for a malformed file does not change.
        Dimensions::new(width, height)?;

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
            layer_table_offset,
            layer_table_length,
            gpu_table_offset,
            gpu_table_length,
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

    fn sample_v5() -> Header {
        Header::new_v5(
            640,
            480,
            PixelFormat::rgba8(),
            0,
            12,
            HEADER_SIZE_V0_5 as u64,
            4096,
        )
        .unwrap()
        .with_feature_flags(FEATURE_GPU)
        .with_gpu_table(72, 272)
    }

    #[test]
    fn header_is_exactly_64_bytes() {
        assert_eq!(HEADER_SIZE, 64);
        let bytes = sample().to_bytes().unwrap();
        assert_eq!(bytes.len(), 64);
    }

    #[test]
    fn a_v5_header_is_exactly_72_bytes() {
        assert_eq!(HEADER_SIZE_V0_5, 72);
        let bytes = sample_v5().to_bytes().unwrap();
        assert_eq!(bytes.len(), 72);
        assert_eq!(bytes[12..16], 72u32.to_le_bytes());
    }

    #[test]
    fn roundtrip_exact() {
        let header = sample();
        let bytes = header.to_bytes().unwrap();
        assert_eq!(Header::parse(&bytes).unwrap(), header);
    }

    #[test]
    fn a_v5_header_round_trips_exactly() {
        let header = sample_v5();
        let bytes = header.to_bytes().unwrap();
        assert_eq!(Header::parse(&bytes).unwrap(), header);
    }

    #[test]
    fn the_64_byte_checksum_rule_did_not_move() {
        // The checksum of a 64-byte header covers bytes 0..52 and 56..64, the
        // rule v0.1 through v0.4 use. The v0.5 header only changes the rule
        // for a header that is 72 bytes long.
        let bytes = sample().to_bytes().unwrap();
        let expected = checksum::crc32c_parts(&[&bytes[0..52], &bytes[56..64]]);
        assert_eq!(leb::read_u32(&bytes, 52), expected);
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

    #[test]
    fn a_v4_file_may_not_use_the_72_byte_header() {
        let mut header = sample();
        header.minor = MINOR_V0_4;
        let mut bytes = header.to_bytes().unwrap();
        bytes.resize(HEADER_SIZE_V0_5, 0);
        bytes[12..16].copy_from_slice(&(HEADER_SIZE_V0_5 as u32).to_le_bytes());
        let sum = checksum::crc32c_parts(&[&bytes[0..52], &bytes[56..72]]);
        bytes[52..56].copy_from_slice(&sum.to_le_bytes());
        assert!(Header::parse(&bytes).is_err());
    }

    #[test]
    fn an_older_file_may_not_claim_the_gpu_feature() {
        let mut header = sample();
        header.feature_flags = FEATURE_GPU;
        let bytes = header.to_bytes().unwrap();
        let error = Header::parse(&bytes).unwrap_err();
        assert_eq!(error.kind(), vpng_core::ErrorKind::UnsupportedFeature);
    }

    #[test]
    fn a_v5_file_must_carry_a_gpu_table() {
        let header = Header::new_v5(
            640,
            480,
            PixelFormat::rgba8(),
            0,
            12,
            HEADER_SIZE_V0_5 as u64,
            4096,
        )
        .unwrap();
        let bytes = header.to_bytes().unwrap();
        assert!(Header::parse(&bytes).is_err());
    }

    #[test]
    fn an_older_file_has_no_room_for_a_gpu_table() {
        // A 64-byte header has no GPU table fields at all, so the header
        // size rule is what keeps an older file from carrying one.
        let header = sample();
        let bytes = header.to_bytes().unwrap();
        assert_eq!(bytes.len(), HEADER_SIZE);
        assert_eq!(Header::parse(&bytes).unwrap().gpu_table_length, 0);
    }

    #[test]
    fn a_truncated_v5_header_is_refused() {
        let bytes = sample_v5().to_bytes().unwrap();
        assert!(Header::parse(&bytes[..64]).is_err());
    }

    #[test]
    fn the_header_size_can_be_read_from_a_prefix() {
        let bytes = sample_v5().to_bytes().unwrap();
        assert_eq!(Header::header_size_from_prefix(&bytes[..16]).unwrap(), 72);
        let bytes = sample().to_bytes().unwrap();
        assert_eq!(Header::header_size_from_prefix(&bytes[..16]).unwrap(), 64);
    }

    /// Build a header's bytes with one dimension forced to zero and the
    /// checksum recomputed, so that only the dimension rule can fire.
    fn zero_dimension_bytes(offset: usize) -> Vec<u8> {
        let mut bytes = sample().to_bytes().unwrap();
        bytes[offset..offset + 4].copy_from_slice(&0u32.to_le_bytes());
        let sum = checksum::crc32c_parts(&[
            &bytes[0..HEADER_CHECKSUM_OFFSET],
            &bytes[RESERVED_OFFSET..HEADER_SIZE],
        ]);
        bytes[HEADER_CHECKSUM_OFFSET..HEADER_CHECKSUM_OFFSET + 4]
            .copy_from_slice(&sum.to_le_bytes());
        bytes
    }

    #[test]
    fn rejects_a_zero_dimension() {
        // Found by the coverage-guided fuzzer in CI (2026-11-18): `parse`
        // used to accept a header with a zero width or height, while
        // `to_bytes` refused to serialize one. The two disagreed about the
        // same bytes.
        for (offset, name) in [(16, "width"), (20, "height")] {
            let bytes = zero_dimension_bytes(offset);
            let err = Header::parse(&bytes).unwrap_err();
            assert_eq!(
                err.kind(),
                vpng_core::ErrorKind::InvalidArgument,
                "a zero {name} must be rejected"
            );
        }
    }

    #[test]
    fn an_accepted_header_can_always_be_re_encoded() {
        // The invariant the `header` fuzz target asserts: whatever `parse`
        // accepts, `to_bytes` must be able to serialize back to the same
        // bytes. It is checked here over a small grid of dimensions rather
        // than only over random bytes.
        for &(width, height) in &[(1u32, 1u32), (2, 3), (640, 480), (16_384, 1)] {
            let header = Header::new(
                width,
                height,
                PixelFormat::rgba8(),
                0,
                1,
                HEADER_SIZE as u64,
                4096,
            )
            .unwrap();
            let bytes = header.to_bytes().unwrap();
            let parsed = Header::parse(&bytes).unwrap();
            assert_eq!(parsed.to_bytes().unwrap(), bytes, "{width}x{height}");
        }
    }
}
