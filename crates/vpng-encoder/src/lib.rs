//! `vpng-encoder` — turn a [`CanonicalImage`] into a complete `.vpng` file.
//!
//! The encoder is deliberately simple: uniform tiles, payloads written back to
//! back after the directory. Every size and offset is computed with checked
//! arithmetic, and the final byte count is verified against the header's
//! declared file size before returning.
//!
//! Each tile is encoded with the smallest payload offered by any implemented
//! codec (see [`vpng_codecs::IMPLEMENTED_CODEC_IDS`]); ties go to the lowest
//! codec id, and Raw is always a candidate, so a valid lossless encoding always
//! exists. A caller may force a single codec with
//! [`EncodeOptions::with_codec`].
//!
//! The encoder never drops information to satisfy a size request. If the output
//! cannot fit `max_size`, it returns [`vpng_core::ErrorKind::SizeLimitExceeded`]
//! and reports the smallest lossless size it found.

#![forbid(unsafe_code)]

use vpng_codecs::{get_codec, EncodeContext, CODEC_AUTO, CODEC_RAW, IMPLEMENTED_CODEC_IDS};
use vpng_container::{crc32c, Directory, DirectoryEntry, Header, HEADER_SIZE};
use vpng_core::{CanonicalImage, Error, ErrorKind, Limits, Result};
use vpng_tiling::{TileGeometry, TileGrid};

/// Nominal tile edge for the reference implementation (256 x 256).
pub const DEFAULT_TILE_SIZE: u32 = 256;

/// Configuration for one encode pass.
#[derive(Debug, Clone, Copy)]
pub struct EncodeOptions {
    pub tile_width: u32,
    pub tile_height: u32,
    /// Codec to force for every tile, or [`vpng_codecs::CODEC_AUTO`] (the
    /// default) to let the encoder pick the smallest payload per tile.
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
            codec_id: CODEC_AUTO,
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

    /// Force one codec for every tile instead of selecting per tile.
    pub fn with_codec(mut self, codec_id: u8) -> Self {
        self.codec_id = codec_id;
        self
    }

    /// Restore per-tile selection (the default).
    pub fn with_auto_codec(mut self) -> Self {
        self.codec_id = CODEC_AUTO;
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

    // A forced codec must be supported before any work starts, so an unknown id
    // fails cleanly rather than partway through a file.
    if options.codec_id != CODEC_AUTO {
        let _ = get_codec(options.codec_id)?;
    }

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
    let mut chosen_codecs: Vec<u8> = Vec::with_capacity(tile_count as usize);
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

        let (codec_id, payload) = select_payload(&tile_buffer, &enc_ctx, options)?;
        let checksum = crc32c(&payload);

        entries.push(DirectoryEntry {
            x: geom.x,
            y: geom.y,
            width: geom.width as u16,
            height: geom.height as u16,
            codec_id,
            predictor_id: 0,
            flags: 0,
            payload_offset: cursor,
            compressed_length: payload.len() as u64,
            uncompressed_length: uncompressed_len,
            payload_checksum: checksum,
        });

        cursor = vpng_core::checked_add(cursor, payload.len() as u64)?;
        payloads.push(payload);
        chosen_codecs.push(codec_id);
    }

    let directory = Directory::new(entries);
    let directory_bytes = directory.encode()?;

    // The header's default codec is informational; the per-tile entry is
    // authoritative (ADR-0003). Record Raw for an auto-selected file.
    let file_codec = if options.codec_id == CODEC_AUTO {
        CODEC_RAW
    } else {
        options.codec_id
    };

    let header = Header::new(
        dims.width,
        dims.height,
        format,
        file_codec,
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
            // This is the smallest lossless representation the enabled codecs
            // can produce. It is reported, never violated.
            return Err(Error::size_limit(limit, cursor));
        }
    }

    Ok(EncodeReport {
        bytes,
        tile_count,
        file_size: cursor,
        codec_ids: chosen_codecs,
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

/// Encode one tile with the smallest payload any implemented codec offers.
///
/// When [`EncodeOptions::codec_id`] forces a codec, only that codec is tried and
/// any error is returned. Otherwise every implemented codec is tried in
/// ascending id order; a codec that reports the tile as not representable
/// ([`ErrorKind::UnsupportedFeature`]) is skipped, and the smallest payload
/// wins. Because the ids are visited in ascending order and a candidate must be
/// *strictly* smaller to replace the incumbent, ties go to the lowest id.
fn select_payload(
    tile: &[u8],
    ctx: &EncodeContext,
    options: &EncodeOptions,
) -> Result<(u8, Vec<u8>)> {
    if options.codec_id != CODEC_AUTO {
        let codec = get_codec(options.codec_id)?;
        let mut payload = Vec::new();
        codec.encode(tile, ctx, &mut payload)?;
        return Ok((options.codec_id, payload));
    }

    let mut best: Option<(u8, Vec<u8>)> = None;
    for &id in IMPLEMENTED_CODEC_IDS {
        let codec = get_codec(id)?;
        let mut payload = Vec::new();
        match codec.encode(tile, ctx, &mut payload) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::UnsupportedFeature => continue,
            Err(err) => return Err(err),
        }
        if best
            .as_ref()
            .map_or(true, |(_, current)| payload.len() < current.len())
        {
            best = Some((id, payload));
        }
    }
    best.ok_or_else(|| Error::internal("no implemented codec could encode the tile"))
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
        let report = encode(
            &image,
            &EncodeOptions::default().with_codec(vpng_codecs::CODEC_RAW),
        )
        .unwrap();
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

    /// A two-color image: the palette's ideal case.
    fn two_color(width: u32, height: u32, format: PixelFormat) -> CanonicalImage {
        let bpp = format.bytes_per_pixel() as usize;
        let mut data = vec![0u8; width as usize * height as usize * bpp];
        for (i, pixel) in data.chunks_exact_mut(bpp).enumerate() {
            let value = if i % 2 == 0 { 0x10 } else { 0xEF };
            for (channel, byte) in pixel.iter_mut().enumerate() {
                *byte = if channel == bpp - 1 && format.color_model.has_alpha() {
                    0xFF
                } else {
                    value
                };
            }
        }
        CanonicalImage::from_vec(width, height, format, data).unwrap()
    }

    #[test]
    fn auto_selection_uses_palette_when_it_is_smaller() {
        let image = two_color(64, 64, PixelFormat::rgba8());
        let options = EncodeOptions::default().with_tile_size(64, 64);

        let auto = encode(&image, &options).unwrap();
        assert_eq!(auto.codec_ids, vec![vpng_codecs::CODEC_PALETTE]);

        let raw = encode(&image, &options.with_codec(vpng_codecs::CODEC_RAW)).unwrap();
        assert!(
            auto.file_size < raw.file_size,
            "palette {} should beat raw {}",
            auto.file_size,
            raw.file_size
        );
    }

    #[test]
    fn auto_selection_falls_back_to_raw_for_noise() {
        // Uniform random bytes give far more than 256 distinct colors per tile.
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        let mut data = vec![0u8; 64 * 64 * 4];
        for byte in data.iter_mut() {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            *byte = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as u8;
        }
        let image = CanonicalImage::from_vec(64, 64, PixelFormat::rgba8(), data).unwrap();

        let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();
        assert_eq!(report.codec_ids, vec![vpng_codecs::CODEC_RAW]);
        // Raw never expands: the payload is exactly the tile's bytes.
        assert_eq!(report.compressed_lengths, vec![64 * 64 * 4]);
    }

    #[test]
    fn auto_selection_records_a_codec_per_tile() {
        // A 4x2 grid where the top-left tile is flat (palette) and the rest are
        // full-color noise (raw), so both codecs must appear.
        let width = 256u32;
        let height = 128u32;
        let mut data = vec![0u8; width as usize * height as usize * 4];
        let mut state = 0xDEAD_BEEF_CAFE_1234u64;
        for (i, pixel) in data.chunks_exact_mut(4).enumerate() {
            let x = i % width as usize;
            let y = i / width as usize;
            let flat = x < 64 && y < 64;
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            let noise = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
            pixel.copy_from_slice(&[
                if flat { 0x20 } else { (noise >> 40) as u8 },
                if flat { 0x20 } else { (noise >> 48) as u8 },
                if flat { 0x20 } else { (noise >> 56) as u8 },
                0xFF,
            ]);
        }
        let image = CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), data).unwrap();
        let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();

        assert_eq!(report.tile_count, 8);
        assert_eq!(report.codec_ids.len(), 8);
        assert_eq!(report.codec_ids[0], vpng_codecs::CODEC_PALETTE);
        assert!(
            report
                .codec_ids
                .iter()
                .skip(1)
                .all(|&id| id == vpng_codecs::CODEC_RAW),
            "noise tiles should fall back to raw: {:?}",
            report.codec_ids
        );
    }
}
