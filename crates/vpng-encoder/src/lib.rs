//! `vpng-encoder` — turn a [`CanonicalImage`] into a complete `.vpng` file.
//!
//! The encoder is deliberately simple in v0.1: one codec, uniform tiles,
//! payloads written back to back after the directory. Every size and offset is
//! computed with checked arithmetic, and the final byte count is verified
//! against the header's declared file size before returning.
//!
//! The encoder never drops information to satisfy a size request. If the output
//! cannot fit `max_size`, it returns [`vpng_core::ErrorKind::SizeLimitExceeded`]
//! and reports the smallest lossless size it found.

#![forbid(unsafe_code)]

use vpng_codecs::{get_codec, EncodeContext};
use vpng_container::{crc32c, Directory, DirectoryEntry, Header, HEADER_SIZE};
use vpng_core::{CanonicalImage, Error, Limits, Result};
use vpng_tiling::{TileGeometry, TileGrid};

/// Nominal tile edge for the reference implementation (256 x 256).
pub const DEFAULT_TILE_SIZE: u32 = 256;

/// Configuration for one encode pass.
#[derive(Debug, Clone, Copy)]
pub struct EncodeOptions {
    pub tile_width: u32,
    pub tile_height: u32,
    /// Codec id to force. v0.1 supports Raw (0) only.
    pub codec_id: u8,
    /// Optional decimal byte ceiling (see ADR-0006 for MB vs MiB).
    pub max_size: Option<u64>,
    pub limits: Limits,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        EncodeOptions {
            tile_width: DEFAULT_TILE_SIZE,
            tile_height: DEFAULT_TILE_SIZE,
            codec_id: vpng_codecs::CODEC_RAW,
            max_size: None,
            limits: Limits::default(),
        }
    }
}

impl EncodeOptions {
    pub fn with_tile_size(mut self, width: u32, height: u32) -> Self {
        self.tile_width = width;
        self.tile_height = height;
        self
    }

    pub fn with_codec(mut self, codec_id: u8) -> Self {
        self.codec_id = codec_id;
        self
    }

    pub fn with_max_size(mut self, max_size: u64) -> Self {
        self.max_size = Some(max_size);
        self
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }
}

/// The produced file plus per-tile diagnostics.
#[derive(Debug, Clone)]
pub struct EncodeReport {
    pub bytes: Vec<u8>,
    pub tile_count: u32,
    pub file_size: u64,
    pub codec_ids: Vec<u8>,
    pub compressed_lengths: Vec<u64>,
    pub uncompressed_lengths: Vec<u32>,
}

impl EncodeReport {
    /// Ratio of uncompressed pixel bytes to file bytes. `1.0` means "no gain".
    pub fn compression_ratio(&self) -> f64 {
        let total: u64 = self.uncompressed_lengths.iter().map(|&v| v as u64).sum();
        if self.file_size == 0 {
            return 1.0;
        }
        total as f64 / self.file_size as f64
    }

    /// Bytes per pixel over the whole file, including container overhead.
    pub fn bytes_per_pixel(&self, pixel_count: u64) -> f64 {
        if pixel_count == 0 {
            return 0.0;
        }
        self.file_size as f64 / pixel_count as f64
    }
}

/// Encode `image` into a complete `.vpng` byte stream.
pub fn encode(image: &CanonicalImage, options: &EncodeOptions) -> Result<EncodeReport> {
    let limits = options.limits;
    let format = image.format();
    let dims = image.dimensions();

    limits.validate_dimensions(dims, "encode input")?;

    if options.tile_width == 0 || options.tile_height == 0 {
        return Err(Error::invalid("tile dimensions must be non-zero"));
    }
    if options.tile_width > u16::MAX as u32 || options.tile_height > u16::MAX as u32 {
        return Err(Error::unsupported(format!(
            "tile size {}x{} does not fit the 16-bit directory fields",
            options.tile_width, options.tile_height
        )));
    }

    let grid = TileGrid::new(
        options.tile_width,
        options.tile_height,
        dims.width,
        dims.height,
    )?;
    let tile_count = grid.tile_count()?;
    limits.validate_tile_count(tile_count)?;

    let codec = get_codec(options.codec_id)?;
    let enc_ctx = EncodeContext {
        limits,
        format,
        tile_width: options.tile_width as u16,
        tile_height: options.tile_height as u16,
    };

    let directory_size = Directory::total_size(tile_count)?;
    let mut cursor = vpng_core::checked_add(HEADER_SIZE as u64, directory_size as u64)?;

    let mut entries: Vec<DirectoryEntry> = Vec::with_capacity(tile_count as usize);
    let mut payloads: Vec<Vec<u8>> = Vec::with_capacity(tile_count as usize);
    let mut tile_buffer: Vec<u8> = Vec::new();

    for geom in grid.iter() {
        if geom.width > u16::MAX as u32 || geom.height > u16::MAX as u32 {
            return Err(Error::unsupported(
                "a tile's pixel width or height does not fit the 16-bit directory fields",
            ));
        }

        extract_tile(image, &geom, &mut tile_buffer)?;
        let uncompressed_len = u32::try_from(tile_buffer.len())
            .map_err(|_| Error::resource_limit("tile uncompressed size exceeds u32"))?;

        let mut payload = Vec::new();
        codec.encode(&tile_buffer, &enc_ctx, &mut payload)?;
        let checksum = crc32c(&payload);

        entries.push(DirectoryEntry {
            x: geom.x,
            y: geom.y,
            width: geom.width as u16,
            height: geom.height as u16,
            codec_id: options.codec_id,
            predictor_id: 0,
            flags: 0,
            payload_offset: cursor,
            compressed_length: payload.len() as u64,
            uncompressed_length: uncompressed_len,
            payload_checksum: checksum,
        });

        cursor = vpng_core::checked_add(cursor, payload.len() as u64)?;
        payloads.push(payload);
    }

    let directory = Directory::new(entries);
    let directory_bytes = directory.encode()?;

    let header = Header::new(
        dims.width,
        dims.height,
        format,
        options.codec_id,
        tile_count,
        HEADER_SIZE as u64,
        cursor,
    )?;

    let capacity = vpng_core::checked_usize(cursor)?;
    let mut bytes = Vec::with_capacity(capacity);
    header.write_to(&mut bytes)?;
    bytes.extend_from_slice(&directory_bytes);
    for payload in &payloads {
        bytes.extend_from_slice(payload);
    }

    if bytes.len() as u64 != cursor {
        return Err(Error::internal(format!(
            "assembled {} bytes but accounted for {cursor}",
            bytes.len()
        )));
    }

    if let Some(limit) = options.max_size {
        if cursor > limit {
            // Raw is the only codec in v0.1, so this is the smallest lossless
            // representation we can currently produce.
            return Err(Error::size_limit(limit, cursor));
        }
    }

    Ok(EncodeReport {
        bytes,
        tile_count,
        file_size: cursor,
        codec_ids: vec![options.codec_id; tile_count as usize],
        compressed_lengths: payloads.iter().map(|p| p.len() as u64).collect(),
        uncompressed_lengths: {
            let mut lengths = Vec::with_capacity(tile_count as usize);
            for geom in grid.iter() {
                lengths.push(
                    (geom.width as u64 * geom.height as u64 * format.bytes_per_pixel()) as u32,
                );
            }
            lengths
        },
    })
}

/// Copy one tile's pixels out of the image into a tight buffer.
pub fn extract_tile(image: &CanonicalImage, geom: &TileGeometry, out: &mut Vec<u8>) -> Result<()> {
    let bpp = image.bytes_per_pixel() as usize;
    let src_row = image.row_bytes()? as usize;
    let tile_row = geom.width as usize * bpp;
    let total = tile_row * geom.height as usize;

    out.clear();
    out.reserve(total);

    let data = image.data();
    for row in 0..geom.height as usize {
        let start = (geom.y as usize + row) * src_row + geom.x as usize * bpp;
        out.extend_from_slice(&data[start..start + tile_row]);
    }

    if out.len() != total {
        return Err(Error::internal(
            "tile extraction produced an unexpected size",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_core::PixelFormat;

    fn gradient(width: u32, height: u32, format: PixelFormat) -> CanonicalImage {
        let bpp = format.bytes_per_pixel() as usize;
        let mut data = vec![0u8; (width as usize) * (height as usize) * bpp];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let base = (y * width as usize + x) * bpp;
                for c in 0..bpp {
                    data[base + c] = ((x * 7 + y * 13 + c * 29) & 0xff) as u8;
                }
            }
        }
        CanonicalImage::from_vec(width, height, format, data).unwrap()
    }

    #[test]
    fn encodes_deterministically() {
        let image = gradient(300, 200, PixelFormat::rgba8());
        let options = EncodeOptions::default().with_tile_size(128, 128);
        let a = encode(&image, &options).unwrap();
        let b = encode(&image, &options).unwrap();
        assert_eq!(a.bytes, b.bytes);
        assert_eq!(a.tile_count, 6); // 3 columns x 2 rows
    }

    #[test]
    fn size_accounting_is_exact() {
        let image = gradient(256, 256, PixelFormat::rgba8());
        let report = encode(&image, &EncodeOptions::default()).unwrap();
        assert_eq!(report.file_size, report.bytes.len() as u64);
        // Raw: header + directory(1 entry) + exactly one raw tile.
        let expected = 64 + 44 + 256 * 256 * 4;
        assert_eq!(report.file_size, expected as u64);
    }

    #[test]
    fn size_limit_is_reported_not_violated() {
        let image = gradient(256, 256, PixelFormat::rgba8());
        let options = EncodeOptions::default().with_max_size(1024);
        let err = encode(&image, &options).unwrap_err();
        assert_eq!(err.kind(), vpng_core::ErrorKind::SizeLimitExceeded);
        match err {
            Error::SizeLimitExceeded { limit, smallest } => {
                assert_eq!(limit, 1024);
                assert!(smallest > limit);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn tile_extraction_matches_manual_slice() {
        let image = gradient(100, 100, PixelFormat::rgb8());
        let grid = TileGrid::new(64, 64, 100, 100).unwrap();
        let geom = grid.geometry(1, 1).unwrap();
        let mut tile = Vec::new();
        extract_tile(&image, &geom, &mut tile).unwrap();
        assert_eq!(tile.len(), 36 * 36 * 3);
        // Top-left pixel of the bottom-right tile is image pixel (64, 64).
        let bpp = 3usize;
        let src = (64 * 100 + 64) * bpp;
        assert_eq!(&tile[0..3], &image.data()[src..src + 3]);
    }

    #[test]
    fn reserved_codec_is_rejected() {
        let image = gradient(64, 64, PixelFormat::rgba8());
        let options = EncodeOptions::default().with_codec(vpng_codecs::CODEC_RLE);
        assert_eq!(
            encode(&image, &options).unwrap_err().kind(),
            vpng_core::ErrorKind::UnsupportedFeature
        );
    }
}
