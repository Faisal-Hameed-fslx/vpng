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

#![forbid(unsafe_code)]

pub mod levels;

pub use levels::{derive_levels, Level};

use vpng_codecs::{get_codec, DecodeContext};
use vpng_container::{crc32c, Directory, DirectoryEntry, Header, HEADER_SIZE};
use vpng_core::{CanonicalImage, Dimensions, Error, Limits, PixelFormat, Rect, Result};
use vpng_tiling::{Partition, TileGeometry, TileGrid, TileRect};

/// A validated, borrowable view over one VPNG file.
#[derive(Debug)]
pub struct Document<'a> {
    bytes: &'a [u8],
    header: Header,
    directory: Directory,
    levels: Vec<Level>,
    format: PixelFormat,
}

impl<'a> Document<'a> {
    /// Parse and fully validate a file. `bytes` must be the complete file.
    pub fn parse(bytes: &'a [u8], limits: &Limits) -> Result<Document<'a>> {
        limits.validate_file_size(bytes.len() as u64)?;

        let header = Header::parse(bytes)?;
        let dims = header.dimensions()?;
        limits.validate_dimensions(dims, "file")?;
        let format = header.format()?;

        if header.declared_file_size != bytes.len() as u64 {
            return Err(Error::corrupt(format!(
                "declared file size {} does not match the {} bytes actually present",
                header.declared_file_size,
                bytes.len()
            )));
        }

        limits.validate_tile_count(header.tile_count)?;

        if header.directory_offset < HEADER_SIZE as u64 {
            return Err(Error::corrupt(
                "directory offset points inside the fixed header",
            ));
        }

        let directory_size = Directory::total_size(header.tile_count)?;
        vpng_core::checked_range(
            header.directory_offset,
            directory_size as u64,
            bytes.len() as u64,
        )?;
        let directory_start = header.directory_offset as usize;
        let directory_slice = &bytes[directory_start..directory_start + directory_size];
        let directory = Directory::decode(directory_slice, header.tile_count)?;

        if directory.len() != header.tile_count as usize {
            return Err(Error::corrupt(
                "directory entry count disagrees with the header",
            ));
        }

        directory.validate_flags(header.minor)?;

        let directory_end = vpng_core::checked_add(header.directory_offset, directory_size as u64)?;

        // Per-tile structural checks that need cross-field context.
        for (index, entry) in directory.entries.iter().enumerate() {
            if entry.payload_offset < directory_end {
                return Err(Error::corrupt(format!(
                    "tile {index} payload begins before the end of the tile directory"
                )));
            }
            limits
                .validate_tile_payload(entry.compressed_length, entry.uncompressed_length as u64)?;
            if !vpng_codecs::is_implemented(entry.codec_id) {
                return Err(Error::unsupported(format!(
                    "tile {index} uses codec id {} ({}), which this build does not implement",
                    entry.codec_id,
                    vpng_codecs::codec_name(entry.codec_id).unwrap_or("unknown")
                )));
            }
        }

        directory.validate_payload_ranges(bytes.len() as u64)?;
        directory.validate_tile_sizes(format.bytes_per_pixel())?;

        let levels = derive_levels(&header, &directory, dims)?;

        Ok(Document {
            bytes,
            header,
            directory,
            levels,
            format,
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

    /// The file's levels: the original image first, then any derived previews.
    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// The number of levels in the file.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// One level by position. Position 0 is the original image.
    pub fn level(&self, position: usize) -> Option<&Level> {
        self.levels.get(position)
    }

    /// The original image's level.
    pub fn base_level(&self) -> &Level {
        &self.levels[0]
    }

    /// The original image's exact partition.
    pub fn partition(&self) -> &Partition {
        self.base_level().partition()
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
            format: self.format,
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
            .levels
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
        let dims = self.dimensions();
        if !rect.is_within(dims) {
            return Err(Error::invalid(format!(
                "region {}x{}+{}+{} is outside the {}x{} image",
                rect.width, rect.height, rect.x, rect.y, dims.width, dims.height
            )));
        }

        let region_dims = Dimensions::new(rect.width, rect.height)?;
        limits.validate_dimensions(region_dims, "region")?;

        let mut out = CanonicalImage::new_zeroed(rect.width, rect.height, self.format, limits)?;
        let bpp = self.format.bytes_per_pixel() as usize;
        let out_row = out.row_bytes()? as usize;

        let level = self.base_level();
        for position in level.partition().tiles_for(rect)? {
            let index = level
                .entry_index(position)
                .expect("a partition tile is always inside its level");
            let pixels = self.decode_tile(index, limits)?;
            let geom = level
                .partition()
                .tile(position)
                .expect("a partition tile is always inside its level");

            let ix0 = geom.x.max(rect.x);
            let iy0 = geom.y.max(rect.y);
            let ix1 = (geom.x + geom.width).min(rect.right()?);
            let iy1 = (geom.y + geom.height).min(rect.bottom()?);

            for y in iy0..iy1 {
                let src =
                    ((y - geom.y) as usize * geom.width as usize + (ix0 - geom.x) as usize) * bpp;
                let n = (ix1 - ix0) as usize * bpp;
                let dst = (y - rect.y) as usize * out_row + (ix0 - rect.x) as usize * bpp;
                out.data_mut()[dst..dst + n].copy_from_slice(&pixels[src..src + n]);
            }
        }

        Ok(out)
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
