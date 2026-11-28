//! `vpng-decoder` — read a `.vpng` file and reproduce exact pixels.
//!
//! A [`Document`] borrows the file bytes and validates the whole container up
//! front: header, directory checksum, tile coverage, payload ranges, and codec
//! availability. After that, decoding individual tiles or regions touches only
//! the payloads it needs.
//!
//! v0.1 layout rule (see `docs/VPNG_SPEC_0_1.md`): the directory sits at the
//! header's `directory_offset` and every tile payload lies after the directory.
//!
//! # Levels
//!
//! A file may hold more than one grid of tiles. Level 0 is the original image;
//! every other level holds derived preview pixels. The levels are derived from
//! the directory by [`derive_levels`], which is the same function the
//! file-backed reader uses, so the two readers cannot disagree.
//!
//! # One validation, every reader
//!
//! [`parse_container_parts`] holds the rules that decide which containers are
//! acceptable. [`Document::parse`], the streaming reader in `vpng-source` and
//! the WebAssembly package all call it, so no two of them can drift apart
//! about which files are valid.

#![forbid(unsafe_code)]

pub mod composite;
pub mod gpu;
pub mod layer;
pub mod levels;

pub use composite::{alpha_offset, sample_max};
pub use layer::{derive_layers, Layer};
pub use levels::{derive_levels, Level};

use vpng_codecs::{get_codec, DecodeContext};
use vpng_container::{
    crc32c, Directory, DirectoryEntry, GpuTable, Header, LayerTable, HEADER_SIZE, MINOR_V0_3,
};
use vpng_core::{CanonicalImage, Dimensions, Error, Limits, PixelFormat, Rect, Result};
use vpng_tiling::{Partition, TileGeometry, TileGrid, TileRect};

use crate::composite::blit_at;

/// The pieces a validated container declares.
///
/// This is the metadata every reader needs and no reader may disagree about:
/// the header, the tile directory, the layers derived from the directory, and
/// the pixel format.
#[derive(Debug, Clone)]
pub struct ContainerParts {
    header: Header,
    directory: Directory,
    layers: Vec<Layer>,
    /// The GPU payload table, when the container carries one.
    gpu: Option<GpuTable>,
    format: PixelFormat,
}

impl ContainerParts {
    /// The container's main header.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The container's tile directory.
    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    /// The container's layers: the base layer first, then any others.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// The container's levels: the original image first, then any derived
    /// previews.
    ///
    /// The levels belong to the **base** layer. A v0.4 container has one
    /// level per layer and no pyramid, so this is a single level; a
    /// container written before v0.4 has one layer and may have a pyramid.
    pub fn levels(&self) -> &[Level] {
        self.layers[0].levels()
    }

    /// The container's pixel format.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// The container's GPU payload table, when it has one.
    ///
    /// The table holds one record per tile, in the directory's canonical
    /// order.
    pub fn gpu_table(&self) -> Option<&GpuTable> {
        self.gpu.as_ref()
    }
}

/// Validate a container's header, directory and layer table, and derive its
/// layers.
///
/// **This is the one implementation of the rules that decide which containers
/// are acceptable.** The in-memory decoder, the streaming reader over any byte
/// source — a file or an HTTP object — and the WebAssembly package all call
/// it, so a container one of them accepts is one all of them accept. The
/// checks are:
///
/// * the header's dimensions and the file's declared size against the bytes
///   actually present;
/// * the directory's length against the header's tile count, and its flags
///   against the minor version;
/// * the layer table's length against the header, when the header declares
///   one, and its own checksum;
/// * every payload's offset against the end of the directory **and** the end
///   of the layer table, and its length against the decompression limits;
/// * the three codecs promoted in v0.3 against the minor version, and every
///   codec id against the set this build implements;
/// * the payload ranges against the file's length, and the tile sizes
///   against the pixel format;
/// * the layers, derived from the directory by [`derive_layers`].
///
/// `file_size` is the number of bytes the object has. A source that knows the
/// object's length passes that length; a caller that holds only the header,
/// the directory and the layer table — a browser that fetched three byte
/// ranges — passes the header's declared size, so the payload *ranges* are
/// checked against the size the container claims for itself.
///
/// `layer_table_bytes` is the layer table, or `None` for a container that has
/// none. A container that *declares* a layer table and is given none is
/// refused: a reader cannot know what the container means without it.
pub fn parse_container_parts(
    header: Header,
    directory_bytes: &[u8],
    layer_table_bytes: Option<&[u8]>,
    gpu_table_bytes: Option<&[u8]>,
    file_size: u64,
    limits: &Limits,
) -> Result<ContainerParts> {
    let dims = header.dimensions()?;
    limits.validate_dimensions(dims, "file")?;
    let format = header.format()?;

    if header.declared_file_size != file_size {
        return Err(Error::corrupt(format!(
            "declared file size {} does not match the {file_size} bytes actually present",
            header.declared_file_size
        )));
    }

    limits.validate_tile_count(header.tile_count)?;

    if header.directory_offset < HEADER_SIZE as u64 {
        return Err(Error::corrupt(
            "directory offset points inside the fixed header",
        ));
    }

    let directory_size = Directory::total_size(header.tile_count)?;
    if directory_bytes.len() != directory_size {
        return Err(Error::corrupt(format!(
            "the tile directory is {} bytes, the header says {directory_size}",
            directory_bytes.len()
        )));
    }
    let directory = Directory::decode(directory_bytes, header.tile_count)?;

    if directory.len() != header.tile_count as usize {
        return Err(Error::corrupt(
            "directory entry count disagrees with the header",
        ));
    }

    directory.validate_flags(header.minor)?;

    let directory_end = vpng_core::checked_add(header.directory_offset, directory_size as u64)?;

    // The layer table, when the container has one.
    let layer_table = match (header.has_layers(), layer_table_bytes) {
        (true, Some(bytes)) => {
            if bytes.len() != header.layer_table_length as usize {
                return Err(Error::corrupt(format!(
                    "the layer table is {} bytes, the header says {}",
                    bytes.len(),
                    header.layer_table_length
                )));
            }
            let table = LayerTable::decode(bytes)?;
            let table_end = vpng_core::checked_add(
                header.layer_table_offset as u64,
                header.layer_table_length as u64,
            )?;
            if (header.layer_table_offset as u64) < directory_end || table_end > file_size {
                return Err(Error::corrupt(
                    "the layer table does not fit between the directory and the end of the file",
                ));
            }
            Some(table)
        }
        (true, None) => {
            return Err(Error::corrupt(
                "the container declares a layer table, and the reader did not supply it",
            ))
        }
        (false, Some(bytes)) if !bytes.is_empty() => {
            return Err(Error::corrupt(
                "the container has no layer table, but one was supplied",
            ))
        }
        _ => None,
    };

    // The payloads come after the directory and, when there is one, after the
    // layer table and the GPU table.
    let mut payloads_start = directory_end;
    if let Some(table) = layer_table.as_ref() {
        payloads_start = vpng_core::checked_add(payloads_start, header.layer_table_length as u64)?;
        let _ = table;
    }

    // The GPU payload table, when the container has one.
    let gpu_table = match (header.has_gpu(), gpu_table_bytes) {
        (true, Some(bytes)) => {
            if bytes.len() != header.gpu_table_length as usize {
                return Err(Error::corrupt(format!(
                    "the GPU table is {} bytes, the header says {}",
                    bytes.len(),
                    header.gpu_table_length
                )));
            }
            let table = GpuTable::decode(bytes)?;
            let table_end = vpng_core::checked_add(
                header.gpu_table_offset as u64,
                header.gpu_table_length as u64,
            )?;
            if (header.gpu_table_offset as u64) < payloads_start || table_end > file_size {
                return Err(Error::corrupt(
                    "the GPU table does not fit between the tables and the end of the file",
                ));
            }
            if table.len() != header.tile_count as usize {
                return Err(Error::corrupt(format!(
                    "the GPU table holds {} records, the file has {} tiles",
                    table.len(),
                    header.tile_count
                )));
            }
            payloads_start =
                vpng_core::checked_add(payloads_start, header.gpu_table_length as u64)?;
            Some(table)
        }
        (true, None) => {
            return Err(Error::corrupt(
                "the container declares a GPU payload table, and the reader did not supply it",
            ))
        }
        (false, Some(bytes)) if !bytes.is_empty() => {
            return Err(Error::corrupt(
                "the container has no GPU payload table, but one was supplied",
            ))
        }
        _ => None,
    };

    for (index, entry) in directory.entries.iter().enumerate() {
        if entry.payload_offset < payloads_start {
            return Err(Error::corrupt(format!(
                "tile {index} payload begins before the end of the container's tables"
            )));
        }
        limits.validate_tile_payload(entry.compressed_length, entry.uncompressed_length as u64)?;
        // A GPU block codec is lossy, and a tile's lossless payload is always
        // lossless, so a directory entry may never name one.
        if vpng_codecs::is_gpu_block_codec(entry.codec_id) {
            return Err(Error::unsupported(format!(
                "tile {index} names codec id {} ({}), which is a GPU block codec and may not be a tile codec",
                entry.codec_id,
                vpng_codecs::codec_name(entry.codec_id).unwrap_or("unknown")
            )));
        }
        // The three codecs promoted in v0.3 may only be named by a file
        // whose minor version says so.
        if vpng_codecs::is_v0_3_codec(entry.codec_id) && header.minor < MINOR_V0_3 {
            return Err(Error::unsupported(format!(
                "tile {index} uses codec id {} ({}), which format v0.{} does not define",
                entry.codec_id,
                vpng_codecs::codec_name(entry.codec_id).unwrap_or("unknown"),
                header.minor
            )));
        }
        if !vpng_codecs::is_implemented(entry.codec_id) {
            return Err(Error::unsupported(format!(
                "tile {index} uses codec id {} ({}), which this build does not implement",
                entry.codec_id,
                vpng_codecs::codec_name(entry.codec_id).unwrap_or("unknown")
            )));
        }
    }

    // The GPU table's records, against the geometry of the tile each one
    // describes.
    if let Some(table) = gpu_table.as_ref() {
        for (index, record) in table.records.iter().enumerate() {
            let entry = &directory.entries[index];
            gpu::validate_record(record, u32::from(entry.width), u32::from(entry.height))?;
            if record.is_present() {
                let end = vpng_core::checked_add(
                    record.payload_offset,
                    u64::from(record.payload_length),
                )?;
                if record.payload_offset < payloads_start || end > file_size {
                    return Err(Error::corrupt(format!(
                        "the GPU payload of tile {index} does not fit between the tables and the end of the file"
                    )));
                }
            }
        }
    }

    directory.validate_payload_ranges(file_size)?;
    directory.validate_tile_sizes(format.bytes_per_pixel())?;

    let layers = derive_layers(&header, &directory, layer_table.as_ref(), dims)?;

    Ok(ContainerParts {
        header,
        directory,
        layers,
        gpu: gpu_table,
        format,
    })
}

/// A validated, borrowable view over one VPNG file.
#[derive(Debug)]
pub struct Document<'a> {
    bytes: &'a [u8],
    header: Header,
    directory: Directory,
    layers: Vec<Layer>,
    /// The GPU payload table, when the file carries one.
    gpu: Option<GpuTable>,
    format: PixelFormat,
}

impl<'a> Document<'a> {
    /// Parse and fully validate a file. `bytes` must be the complete file.
    pub fn parse(bytes: &'a [u8], limits: &Limits) -> Result<Document<'a>> {
        limits.validate_file_size(bytes.len() as u64)?;

        let header = Header::parse(bytes)?;

        // The directory's own bytes are read out of the file, but every rule
        // that decides whether the container is acceptable is applied by
        // `parse_container_parts`, which every other reader shares.
        let directory_size = Directory::total_size(header.tile_count)?;
        vpng_core::checked_range(
            header.directory_offset,
            directory_size as u64,
            bytes.len() as u64,
        )?;
        let directory_start = header.directory_offset as usize;
        let directory_slice = &bytes[directory_start..directory_start + directory_size];

        // The layer table, when the file has one, sits between the directory
        // and the payloads.
        let layer_table_bytes = if header.has_layers() {
            let start = header.layer_table_offset as usize;
            let length = header.layer_table_length as usize;
            let end = vpng_core::checked_add(start as u64, length as u64)? as usize;
            if end > bytes.len() {
                return Err(Error::corrupt(
                    "the layer table extends past the end of the file",
                ));
            }
            Some(&bytes[start..end])
        } else {
            None
        };

        // The GPU payload table, when the file has one, sits with the other
        // tables.
        let gpu_table_bytes = if header.has_gpu() {
            let start = header.gpu_table_offset as usize;
            let length = header.gpu_table_length as usize;
            let end = vpng_core::checked_add(start as u64, length as u64)? as usize;
            if end > bytes.len() {
                return Err(Error::corrupt(
                    "the GPU table extends past the end of the file",
                ));
            }
            Some(&bytes[start..end])
        } else {
            None
        };

        let parts = parse_container_parts(
            header,
            directory_slice,
            layer_table_bytes,
            gpu_table_bytes,
            bytes.len() as u64,
            limits,
        )?;

        Ok(Document {
            bytes,
            header: parts.header,
            directory: parts.directory,
            layers: parts.layers,
            gpu: parts.gpu,
            format: parts.format,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    pub fn tile_entry(&self, index: usize) -> Option<&DirectoryEntry> {
        self.directory.entries.get(index)
    }

    /// The file's layers: the base layer first, then any others.
    ///
    /// A file written before v0.4 has exactly one layer.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// The file's GPU payload table, when it has one.
    ///
    /// A file without one has no GPU payloads at all, and
    /// [`Document::decode_gpu_tile`] refuses it.
    pub fn gpu_table(&self) -> Option<&GpuTable> {
        self.gpu.as_ref()
    }

    /// Borrow one tile's **GPU payload** without decoding it.
    ///
    /// The bytes are the raw block payload: BC1 colour blocks, then BC4 alpha
    /// blocks. The tile's checksum is verified before the bytes are handed
    /// over.
    pub fn gpu_payload(&self, index: usize) -> Result<&'a [u8]> {
        let table = self
            .gpu
            .as_ref()
            .ok_or_else(|| Error::unsupported("this file carries no GPU payload table"))?;
        let record = table
            .get(index)
            .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
        if !record.is_present() {
            return Ok(&[]);
        }
        let start = vpng_core::checked_usize(record.payload_offset)?;
        let len = vpng_core::checked_usize(record.payload_length as u64)?;
        let end = vpng_core::checked_add(record.payload_offset, u64::from(record.payload_length))?;
        if end > self.bytes.len() as u64 {
            return Err(Error::corrupt("the GPU payload escapes the file buffer"));
        }
        let payload = &self.bytes[start..start + len];
        let checksum = crc32c(payload);
        if checksum != record.payload_checksum {
            return Err(Error::corrupt(format!(
                "the GPU payload of tile {index} has checksum {checksum:#010x}, the table says {:#010x}",
                record.payload_checksum
            )));
        }
        Ok(payload)
    }

    /// Decode one tile's **GPU payload** into RGBA8 pixels.
    ///
    /// The result is **lossy**: BC1 and BC4 throw information away, and what
    /// comes out is close to the original and not equal to it. The exact
    /// pixels are [`Document::decode_tile`], and they are what every exact
    /// operation uses.
    pub fn decode_gpu_tile(&self, index: usize) -> Result<Vec<u8>> {
        let table = self
            .gpu
            .as_ref()
            .ok_or_else(|| Error::unsupported("this file carries no GPU payload table"))?;
        let record = *table
            .get(index)
            .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
        let entry = *self
            .directory
            .entries
            .get(index)
            .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
        let payload = self.gpu_payload(index)?;
        gpu::decode_gpu_tile(
            &record,
            u32::from(entry.width),
            u32::from(entry.height),
            payload,
        )
    }

    /// Decode a region of the **GPU payload** into RGBA8 pixels.
    ///
    /// The result is **lossy**, and it is always RGBA8 whatever the file's own
    /// format is: a GPU payload is always eight-bit, and a texture is always
    /// RGBA.
    pub fn decode_gpu_region(&self, rect: Rect, limits: &Limits) -> Result<CanonicalImage> {
        if self.gpu.is_none() {
            return Err(Error::unsupported("this file carries no GPU payload table"));
        }
        decode_region_from(
            self.base_level(),
            PixelFormat::rgba8(),
            rect,
            limits,
            |index| self.decode_gpu_tile(index),
        )
    }

    /// The number of layers in the file.
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// One layer by position. Position 0 is the base layer.
    pub fn layer(&self, position: usize) -> Option<&Layer> {
        self.layers.get(position)
    }

    /// The base layer: the image itself.
    pub fn base_layer(&self) -> &Layer {
        &self.layers[0]
    }

    /// The file's levels: the original image first, then any derived previews.
    ///
    /// The levels belong to the **base** layer.
    pub fn levels(&self) -> &[Level] {
        self.layers[0].levels()
    }

    /// The number of levels in the base layer.
    pub fn level_count(&self) -> usize {
        self.layers[0].levels().len()
    }

    /// One level of the base layer by position. Position 0 is the original
    /// image.
    pub fn level(&self, position: usize) -> Option<&Level> {
        self.layers[0].levels().get(position)
    }

    /// The original image's level.
    pub fn base_level(&self) -> &Level {
        self.layers[0].base_level()
    }

    /// The original image's exact partition.
    pub fn partition(&self) -> &Partition {
        self.base_layer().partition()
    }

    pub fn format(&self) -> PixelFormat {
        self.format
    }

    pub fn width(&self) -> u32 {
        self.header.width
    }

    pub fn height(&self) -> u32 {
        self.header.height
    }

    pub fn dimensions(&self) -> Dimensions {
        Dimensions {
            width: self.header.width,
            height: self.header.height,
        }
    }

    pub fn tile_count(&self) -> u32 {
        self.header.tile_count
    }

    /// Borrow one tile's compressed payload without decoding it.
    pub fn tile_payload(&self, index: usize) -> Result<&'a [u8]> {
        let entry = self
            .directory
            .entries
            .get(index)
            .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
        let start = vpng_core::checked_usize(entry.payload_offset)?;
        let len = vpng_core::checked_usize(entry.compressed_length)?;
        let end = vpng_core::checked_add(entry.payload_offset, entry.compressed_length)?;
        if end > self.bytes.len() as u64 {
            return Err(Error::internal("tile payload escapes the file buffer"));
        }
        Ok(&self.bytes[start..start + len])
    }

    /// Decode exactly one tile into a fresh buffer.
    pub fn decode_tile(&self, index: usize, limits: &Limits) -> Result<Vec<u8>> {
        let entry = *self
            .directory
            .entries
            .get(index)
            .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
        let payload = self.tile_payload(index)?;
        decode_tile_payload(index, payload, &entry, self.format, limits)
    }

    /// Decode the whole image. This is the only path that allocates a full-image
    /// pixel buffer, and only when the caller asks for it.
    ///
    /// The result is the **original** image: level 0's pixels, exactly. A
    /// derived preview level is never used to produce it.
    pub fn decode_all(&self, limits: &Limits) -> Result<CanonicalImage> {
        self.decode_level(0, limits)
    }

    /// Decode one level into a fresh buffer.
    ///
    /// Level 0 is the original image and is exact. A derived level is a
    /// preview: its pixels are whatever the file stored, and they are not a
    /// substitute for the original.
    pub fn decode_level(&self, position: usize, limits: &Limits) -> Result<CanonicalImage> {
        let level = self
            .levels()
            .get(position)
            .ok_or_else(|| Error::invalid(format!("level {position} is out of range")))?;

        let dims = level.dimensions();
        let mut image = CanonicalImage::new_zeroed(dims.width, dims.height, self.format, limits)?;

        for (offset, tile) in level.partition().tiles().iter().enumerate() {
            let index = level
                .entry_index(offset)
                .expect("a partition tile is always inside its level");
            let pixels = self.decode_tile(index, limits)?;
            blit(&mut image, tile, &pixels)?;
        }

        Ok(image)
    }

    /// Decode the tiles intersecting `rect` and return exactly that region.
    ///
    /// No unrelated tile is decoded, and the returned image is sized to the
    /// requested rectangle. The region is taken from the **original** level.
    pub fn decode_region(&self, rect: Rect, limits: &Limits) -> Result<CanonicalImage> {
        decode_region_from(self.base_level(), self.format, rect, limits, |index| {
            self.decode_tile(index, limits)
        })
    }

    /// Decode one layer into a fresh buffer.
    ///
    /// The result is sized to the **layer's own rectangle**, not the canvas,
    /// and it holds the layer's pixels exactly. A mask layer decodes to its
    /// coverage image: the same pixels, of which only the alpha channel
    /// carries meaning.
    pub fn decode_layer(&self, position: usize, limits: &Limits) -> Result<CanonicalImage> {
        let layer = self
            .layers
            .get(position)
            .ok_or_else(|| Error::invalid(format!("layer {position} is out of range")))?;

        let rect = layer.rect();
        let level = layer.base_level();
        decode_layer_from(level, self.format, rect, limits, |index| {
            self.decode_tile(index, limits)
        })
    }

    /// Composite every layer into one image on the canvas.
    ///
    /// The base layer is the canvas, exactly. Each layer above it is blended
    /// over the result in the order the layer table lists them, with its own
    /// opacity and — when it has one — its mask.
    ///
    /// A container with one layer composites to **exactly** that layer's
    /// pixels: the blend of a single layer is the identity, not an
    /// approximation.
    ///
    /// The composite is a *derived* image. Each layer's own pixels are exact
    /// and are recovered by [`Document::decode_layer`]; the composite is
    /// where opacity and masks are applied.
    pub fn composite(&self, limits: &Limits) -> Result<CanonicalImage> {
        let base = self.base_layer();
        let canvas = base.rect();
        let mut out = self.decode_layer(0, limits)?;

        for position in 1..self.layers.len() {
            let layer = &self.layers[position];
            if layer.is_mask() {
                // A mask is never drawn on its own; it modulates another
                // layer.
                continue;
            }
            let source = self.decode_layer(position, limits)?;
            let coverage = match layer.mask() {
                Some(mask_index) => {
                    let mask = self.layers.get(mask_index as usize).ok_or_else(|| {
                        Error::corrupt(format!(
                            "layer {position} names mask layer {mask_index}, which does not exist"
                        ))
                    })?;
                    Some(self.mask_coverage(mask, layer, limits)?)
                }
                None => None,
            };
            composite::blend(
                &mut out,
                canvas,
                &source,
                layer.rect(),
                coverage.as_deref(),
                layer.opacity(),
                self.format,
            )?;
        }

        Ok(out)
    }

    /// The coverage of a mask over a layer's rectangle.
    ///
    /// The result is one byte per pixel of the layer's rectangle: 0 means
    /// hidden and 255 means fully visible.
    fn mask_coverage(&self, mask: &Layer, layer: &Layer, limits: &Limits) -> Result<Vec<u8>> {
        let mask_image = self.decode_layer(mask.index() as usize, limits)?;
        let layer_rect = layer.rect();
        let mask_rect = mask.rect();
        let format = self.format;

        let offset = composite::alpha_offset(format).ok_or_else(|| {
            Error::unsupported("a mask needs a pixel format with an alpha channel")
        })?;
        let sample_bytes = format.bytes_per_channel() as usize;

        let mut coverage = vec![0u8; layer_rect.width as usize * layer_rect.height as usize];
        let mask_row = mask_image.row_bytes()? as usize;
        let bpp = format.bytes_per_pixel() as usize;

        for row in 0..layer_rect.height as usize {
            for column in 0..layer_rect.width as usize {
                let mx = layer_rect.x - mask_rect.x + column as u32;
                let my = layer_rect.y - mask_rect.y + row as u32;
                let start = my as usize * mask_row + mx as usize * bpp + offset;
                let value = if sample_bytes == 1 {
                    mask_image.data()[start] as u64
                } else {
                    u16::from_le_bytes([mask_image.data()[start], mask_image.data()[start + 1]])
                        as u64
                };
                // A 16-bit alpha is scaled down to a byte.
                let scaled = if sample_bytes == 1 {
                    value
                } else {
                    value / 257
                };
                coverage[row * layer_rect.width as usize + column] = scaled.min(255) as u8;
            }
        }

        Ok(coverage)
    }

    /// Verify every tile payload's CRC32C without decoding a single pixel.
    ///
    /// `Document::parse` already validates the header, the directory checksum,
    /// tile coverage, and every payload *range*. What it deliberately does not
    /// do is read payload bytes, which is what keeps parsing O(1) in the tile
    /// count. This method performs the remaining check in one pass so a caller
    /// can ask "is this file internally intact?" with no reference image.
    ///
    /// It allocates no pixel buffer, so it is safe on files far larger than
    /// memory; it does read the payload bytes, so it is O(file size).
    /// The first failing tile is reported.
    pub fn check_integrity(&self) -> Result<IntegrityReport> {
        let mut payload_bytes = 0u64;
        for (index, entry) in self.directory.entries.iter().enumerate() {
            let payload = self.tile_payload(index)?;
            let computed = crc32c(payload);
            if computed != entry.payload_checksum {
                return Err(Error::corrupt(format!(
                    "tile {index} payload checksum mismatch: stored {:#010x}, computed {computed:#010x}",
                    entry.payload_checksum
                )));
            }
            payload_bytes = vpng_core::checked_add(payload_bytes, entry.compressed_length)?;
        }
        Ok(IntegrityReport {
            tiles_checked: self.header.tile_count,
            payload_bytes,
        })
    }
}

/// Decode one tile's payload, checking its checksum and its length.
///
/// The payload's CRC32C is verified **before** the codec sees it, and the
/// decoded length is checked against the directory's declaration. The codec
/// is the one the directory names.
///
/// The helper is shared by the in-memory decoder, the streaming reader and
/// the WebAssembly package, so that all three refuse the same payloads and
/// decode the same pixels. `index` names the tile in the error messages.
pub fn decode_tile_payload(
    index: usize,
    payload: &[u8],
    entry: &DirectoryEntry,
    format: PixelFormat,
    limits: &Limits,
) -> Result<Vec<u8>> {
    let computed = crc32c(payload);
    if computed != entry.payload_checksum {
        return Err(Error::corrupt(format!(
            "tile {index} payload checksum mismatch: stored {:#010x}, computed {computed:#010x}",
            entry.payload_checksum
        )));
    }

    let codec = get_codec(entry.codec_id)?;
    let ctx = DecodeContext {
        limits: *limits,
        format,
    };
    let mut out = Vec::new();
    codec.decode(payload, entry.uncompressed_length as usize, &ctx, &mut out)?;

    if out.len() != entry.uncompressed_length as usize {
        return Err(Error::corrupt(format!(
            "tile {index} decoded to {} bytes, directory declares {}",
            out.len(),
            entry.uncompressed_length
        )));
    }
    Ok(out)
}

/// Assemble a region from a level, fetching each tile's pixels with `fetch`.
///
/// The helper is shared by the in-memory decoder and the streaming reader so
/// that the two cannot disagree about how a region is put together. The
/// in-memory decoder fetches from the bytes it borrowed; the streaming
/// reader fetches from a file, an HTTP object store, or anywhere else.
///
/// `fetch` is called with a **directory entry index** and must return that
/// tile's decoded pixels. Only the tiles the region intersects are fetched:
/// an unrelated tile is never asked for.
pub fn decode_region_from<F>(
    level: &Level,
    format: PixelFormat,
    rect: Rect,
    limits: &Limits,
    mut fetch: F,
) -> Result<CanonicalImage>
where
    F: FnMut(usize) -> Result<Vec<u8>>,
{
    let dims = level.dimensions();
    if !rect.is_within(dims) {
        return Err(Error::invalid(format!(
            "region {}x{}+{}+{} is outside the {}x{} image",
            rect.width, rect.height, rect.x, rect.y, dims.width, dims.height
        )));
    }

    let region_dims = Dimensions::new(rect.width, rect.height)?;
    limits.validate_dimensions(region_dims, "region")?;

    let mut out = CanonicalImage::new_zeroed(rect.width, rect.height, format, limits)?;
    let bpp = format.bytes_per_pixel() as usize;
    let out_row = out.row_bytes()? as usize;

    for position in level.partition().tiles_for(rect)? {
        let index = level
            .entry_index(position)
            .expect("a partition tile is always inside its level");
        let pixels = fetch(index)?;
        let geom = level
            .partition()
            .tile(position)
            .expect("a partition tile is always inside its level");

        let ix0 = geom.x.max(rect.x);
        let iy0 = geom.y.max(rect.y);
        let ix1 = (geom.x + geom.width).min(rect.right()?);
        let iy1 = (geom.y + geom.height).min(rect.bottom()?);

        for y in iy0..iy1 {
            let src = ((y - geom.y) as usize * geom.width as usize + (ix0 - geom.x) as usize) * bpp;
            let n = (ix1 - ix0) as usize * bpp;
            let dst = (y - rect.y) as usize * out_row + (ix0 - rect.x) as usize * bpp;
            out.data_mut()[dst..dst + n].copy_from_slice(&pixels[src..src + n]);
        }
    }

    Ok(out)
}

/// Assemble one layer's pixels, fetching each tile's pixels with `fetch`.
///
/// The result is sized to the **layer's own rectangle**, not the canvas: a
/// layer's tiles carry canvas coordinates, so the origin is subtracted on the
/// way in. The helper is shared by the in-memory decoder and the streaming
/// reader so that the two cannot disagree about how a layer is put together.
///
/// `fetch` is called with a **directory entry index** and must return that
/// tile's decoded pixels.
pub fn decode_layer_from<F>(
    level: &Level,
    format: PixelFormat,
    rect: Rect,
    limits: &Limits,
    mut fetch: F,
) -> Result<CanonicalImage>
where
    F: FnMut(usize) -> Result<Vec<u8>>,
{
    let dims = Dimensions::new(rect.width, rect.height)?;
    limits.validate_dimensions(dims, "layer")?;

    let mut out = CanonicalImage::new_zeroed(rect.width, rect.height, format, limits)?;

    for position in 0..level.partition().len() {
        let index = level
            .entry_index(position)
            .expect("a partition tile is always inside its level");
        let pixels = fetch(index)?;
        let geom = level
            .partition()
            .tile(position)
            .expect("a partition tile is always inside its level");
        blit_at(&mut out, geom, rect.x, rect.y, &pixels)?;
    }

    Ok(out)
}

/// Summary of a successful [`Document::check_integrity`] pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntegrityReport {
    /// Number of tile payloads verified.
    pub tiles_checked: u32,
    /// Total compressed payload bytes covered by the check.
    pub payload_bytes: u64,
}

/// Derive the nominal tile grid from a directory and confirm exact, ordered
/// coverage of the image.
///
/// The function is public so a reader that parses the container itself — the
/// editing engine's file-backed source, which cannot hold the whole file in
/// memory — derives exactly the same grid as [`Document::parse`]. Sharing
/// the derivation keeps the two readers from disagreeing about what a tile
/// is.
pub fn derive_grid(header: &Header, directory: &Directory, dims: Dimensions) -> Result<TileGrid> {
    if directory.entries.is_empty() {
        return Err(Error::corrupt("the tile directory is empty"));
    }

    let mut xs: Vec<u32> = directory.entries.iter().map(|e| e.x).collect();
    xs.sort_unstable();
    xs.dedup();
    let mut ys: Vec<u32> = directory.entries.iter().map(|e| e.y).collect();
    ys.sort_unstable();
    ys.dedup();

    if xs.first() != Some(&0) || ys.first() != Some(&0) {
        return Err(Error::corrupt(
            "the tile grid does not start at the image origin",
        ));
    }

    let tile_width = if xs.len() >= 2 {
        xs[1] - xs[0]
    } else {
        dims.width
    };
    let tile_height = if ys.len() >= 2 {
        ys[1] - ys[0]
    } else {
        dims.height
    };

    let grid = TileGrid::new(tile_width, tile_height, dims.width, dims.height)?;

    if grid.tile_count()? != header.tile_count {
        return Err(Error::corrupt(format!(
            "header declares {} tiles but a {tile_width}x{tile_height} grid over {}x{} needs {}",
            header.tile_count,
            dims.width,
            dims.height,
            grid.tile_count()?
        )));
    }

    let expected: Vec<TileGeometry> = grid.iter().collect();
    if expected.len() != directory.entries.len() {
        return Err(Error::corrupt("tile count does not match the derived grid"));
    }

    for (geom, entry) in expected.iter().zip(directory.entries.iter()) {
        if entry.x != geom.x
            || entry.y != geom.y
            || entry.width as u32 != geom.width
            || entry.height as u32 != geom.height
        {
            return Err(Error::corrupt(format!(
                "tile entries are not in canonical (y, x) order with exact coverage: \
                 found {}x{}+{}+{} where {}x{}+{}+{} was expected",
                entry.width,
                entry.height,
                entry.x,
                entry.y,
                geom.width,
                geom.height,
                geom.x,
                geom.y
            )));
        }
    }

    Ok(grid)
}

/// Copy a decoded tile's pixels into the destination image at its geometry.
fn blit(image: &mut CanonicalImage, geom: &TileRect, pixels: &[u8]) -> Result<()> {
    let bpp = image.bytes_per_pixel() as usize;
    let dst_row = image.row_bytes()? as usize;
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
        let src = row * tile_row;
        let dst = (geom.y as usize + row) * dst_row + geom.x as usize * bpp;
        data[dst..dst + tile_row].copy_from_slice(&pixels[src..src + tile_row]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_core::PixelFormat;
    use vpng_encoder::{encode, EncodeOptions};

    fn gradient(width: u32, height: u32, format: PixelFormat) -> CanonicalImage {
        let bpp = format.bytes_per_pixel() as usize;
        let mut data = vec![0u8; width as usize * height as usize * bpp];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let base = (y * width as usize + x) * bpp;
                for c in 0..bpp {
                    data[base + c] = ((x * 3 + y * 11 + c * 41) & 0xff) as u8;
                }
            }
        }
        CanonicalImage::from_vec(width, height, format, data).unwrap()
    }

    #[test]
    fn full_roundtrip_is_exact() {
        let limits = Limits::default();
        let image = gradient(300, 180, PixelFormat::rgba8());
        let file = encode(&image, &EncodeOptions::default().with_tile_size(128, 128))
            .unwrap()
            .bytes;
        let doc = Document::parse(&file, &limits).unwrap();
        let decoded = doc.decode_all(&limits).unwrap();
        assert_eq!(decoded, image);
    }

    #[test]
    fn region_equals_crop_of_full() {
        let limits = Limits::default();
        let image = gradient(300, 180, PixelFormat::rgb8());
        let file = encode(&image, &EncodeOptions::default().with_tile_size(128, 128))
            .unwrap()
            .bytes;
        let doc = Document::parse(&file, &limits).unwrap();

        let full = doc.decode_all(&limits).unwrap();
        let rect = Rect::new(100, 40, 150, 90).unwrap();
        let region = doc.decode_region(rect, &limits).unwrap();

        let bpp = image.bytes_per_pixel() as usize;
        let src_row = image.row_bytes().unwrap() as usize;
        for y in 0..rect.height as usize {
            let src = (rect.y as usize + y) * src_row + rect.x as usize * bpp;
            let dst = y * region.row_bytes().unwrap() as usize;
            assert_eq!(
                &region.data()[dst..dst + rect.width as usize * bpp],
                &full.data()[src..src + rect.width as usize * bpp]
            );
        }
    }

    #[test]
    fn region_does_not_decode_unrelated_tiles() {
        // A single-tile region must not need the whole file's pixels: we assert
        // the returned buffer is exactly region-sized, which is the observable
        // contract in v0.1.
        let limits = Limits::default();
        let image = gradient(512, 512, PixelFormat::rgba8());
        let file = encode(&image, &EncodeOptions::default()).unwrap().bytes;
        let doc = Document::parse(&file, &limits).unwrap();
        let region = doc
            .decode_region(Rect::new(0, 0, 8, 8).unwrap(), &limits)
            .unwrap();
        assert_eq!(region.width(), 8);
        assert_eq!(region.height(), 8);
        assert_eq!(region.len(), 8 * 8 * 4);
    }

    #[test]
    fn region_outside_image_is_rejected() {
        let limits = Limits::default();
        let image = gradient(64, 64, PixelFormat::rgba8());
        let file = encode(&image, &EncodeOptions::default()).unwrap().bytes;
        let doc = Document::parse(&file, &limits).unwrap();
        assert!(doc
            .decode_region(Rect::new(60, 60, 10, 10).unwrap(), &limits)
            .is_err());
    }

    #[test]
    fn tile_decode_matches_full_image_span() {
        let limits = Limits::default();
        let image = gradient(300, 180, PixelFormat::rgba8());
        let file = encode(&image, &EncodeOptions::default().with_tile_size(256, 256))
            .unwrap()
            .bytes;
        let doc = Document::parse(&file, &limits).unwrap();
        let partition = doc.partition();
        let geom = partition.tile(1).unwrap();
        let tile = doc.decode_tile(1, &limits).unwrap();

        let bpp = 4usize;
        let src_row = 300 * bpp;
        for row in 0..geom.height as usize {
            let src = (row * src_row) + geom.x as usize * bpp;
            let dst = row * geom.width as usize * bpp;
            assert_eq!(
                &tile[dst..dst + geom.width as usize * bpp],
                &image.data()[src..src + geom.width as usize * bpp]
            );
        }
    }

    #[test]
    fn integrity_check_passes_on_a_valid_file() {
        let limits = Limits::default();
        let image = gradient(300, 180, PixelFormat::rgba8());
        let file = encode(&image, &EncodeOptions::default().with_tile_size(64, 64))
            .unwrap()
            .bytes;
        let doc = Document::parse(&file, &limits).unwrap();
        let report = doc.check_integrity().unwrap();
        assert_eq!(report.tiles_checked, doc.tile_count());
        assert!(report.payload_bytes > 0);
    }

    #[test]
    fn integrity_check_detects_a_payload_flip_that_parse_cannot() {
        let limits = Limits::default();
        let image = gradient(128, 96, PixelFormat::rgba8());
        let mut file = encode(&image, &EncodeOptions::default().with_tile_size(64, 64))
            .unwrap()
            .bytes;

        // Flip the last byte, which lives inside a tile payload.
        let last = file.len() - 1;
        file[last] ^= 0xff;

        // Parsing still succeeds: it never reads payload bytes.
        let doc = Document::parse(&file, &limits).unwrap();

        let err = doc.check_integrity().unwrap_err();
        assert_eq!(err.kind(), vpng_core::ErrorKind::CorruptFile);
        assert!(format!("{err}").contains("payload checksum mismatch"));
    }

    #[test]
    fn integrity_check_reports_the_first_bad_tile() {
        let limits = Limits::default();
        let image = gradient(128, 96, PixelFormat::rgba8());
        let mut file = encode(&image, &EncodeOptions::default().with_tile_size(64, 64))
            .unwrap()
            .bytes;

        // Corrupt the very first payload byte, which belongs to tile 0.
        let doc_before = Document::parse(&file, &limits).unwrap();
        let first = doc_before.tile_entry(0).unwrap().payload_offset as usize;
        file[first] ^= 0xff;

        let doc = Document::parse(&file, &limits).unwrap();
        let message = format!("{}", doc.check_integrity().unwrap_err());
        assert!(message.contains("tile 0"), "unexpected message: {message}");
    }
}
