//! `vpng-encoder` — turn canonical pixels into a complete `.vpng` file.
//!
//! The encoder is deliberately simple: tiles, payloads written back to back
//! after the directory. Every size and offset is computed with checked
//! arithmetic, and the final byte count is verified against the header's
//! declared file size before returning.
//!
//! Each tile is encoded with the smallest payload offered by the codecs the
//! options permit (see [`EncodeOptions::codec_set`]); ties go to the lowest
//! codec id, and Raw is always a candidate, so a valid lossless encoding always
//! exists. A caller may force a single codec with
//! [`EncodeOptions::with_codec`].
//!
//! The encoder never drops information to satisfy a size request. If the output
//! cannot fit `max_size`, it returns [`vpng_core::ErrorKind::SizeLimitExceeded`]
//! and reports the smallest lossless size it found.
//!
//! # Parallel and streaming encoding
//!
//! [`parallel::encode_parallel`] encodes the tiles on several threads, and
//! [`stream::encode_to_file`] writes a file without ever holding the whole
//! *output* in memory. Both are **byte-identical** to [`encode`]: the per-tile
//! work sees only the tile's own pixels and geometry, so the file's bytes
//! depend on the tiles, never on the order the workers finished in. That rule
//! is checked by the test suite over every pixel format, several sizes and
//! several tile geometries.
//!
//! # v0.2: previews and adaptive tiling
//!
//! Two features take the file to **v0.2**. Both are opt-in, and the default
//! path still writes the **v0.1** bytes the golden fixtures pin.
//!
//! * **The preview pyramid** ([`pyramid`]) adds derived levels above the
//!   original. Level 0 is always the original and is always exact; a derived
//!   level is a preview and is never used to reconstruct it.
//! * **Adaptive tiling** ([`adaptive`]) chooses the tile sizes from the
//!   picture's content: calm regions are covered by large tiles, busy regions
//!   by small ones.
//!
//! A file that uses either feature is a v0.2 file: its minor version is 2,
//! its directory entries carry the pyramid level in their `flags` field, and
//! its levels may hold tiles of different sizes. The header and the directory
//! entry layouts are **unchanged**: v0.2 is a set of rules on top of the same
//! 64-byte header and the same 40-byte entries.
//!
//! # v0.3: the promoted codecs
//!
//! Format **v0.3** adds three codecs to the stable set: the tile-local LZ77
//! dictionary (id 4), prediction layered over that dictionary (id 5), and
//! context-adaptive entropy coding (id 6). A file whose directory names one
//! of them is a v0.3 file, and its minor version says so.
//!
//! The minor version states what the file **contains**, not what the options
//! allowed: a file that ends up using only the original four codecs stays a
//! v0.1 file even when the extended set was permitted. The default codec set
//! is the original four, so the default path's bytes do not move.

#![forbid(unsafe_code)]

pub mod adaptive;
pub mod parallel;
pub mod pyramid;
pub mod source;
pub mod stream;

use vpng_codecs::{get_codec, CodecSet, EncodeContext, CODEC_AUTO, CODEC_RAW};
use vpng_container::{
    crc32c, Directory, DirectoryEntry, Header, FEATURE_PYRAMID, HEADER_SIZE, MINOR_V0_1,
    MINOR_V0_2, MINOR_V0_3,
};
use vpng_core::{CanonicalImage, Dimensions, Error, ErrorKind, Limits, PixelFormat, Result};
use vpng_tiling::{Partition, TileGeometry, TileGrid, TileRect};

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
    /// Which codecs the per-tile selector may choose from.
    ///
    /// The default is [`CodecSet::V0_1`], the four original codecs: their
    /// payloads are the ones the published measurements and the golden
    /// fixtures pin. [`CodecSet::V0_3`] adds the three codecs promoted in
    /// format v0.3 — the dictionary, the predictor over a dictionary, and
    /// the context-adaptive model — which produce smaller files and cost
    /// several times more to compute.
    pub codec_set: CodecSet,
    /// Optional decimal byte ceiling (see ADR-0006 for MB vs MiB).
    pub max_size: Option<u64>,
    pub limits: Limits,
    /// Build a preview pyramid above the original (a v0.2 file).
    pub pyramid: bool,
    /// How many derived preview levels to build.
    pub pyramid_levels: u32,
    /// Choose the tile sizes from the picture's content (a v0.2 file).
    pub adaptive: bool,
    /// The busyness threshold of the adaptive planner.
    pub adaptive_threshold: u32,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        EncodeOptions {
            tile_width: DEFAULT_TILE_SIZE,
            tile_height: DEFAULT_TILE_SIZE,
            codec_id: CODEC_AUTO,
            codec_set: CodecSet::V0_1,
            max_size: None,
            limits: Limits::default(),
            pyramid: false,
            pyramid_levels: DEFAULT_PYRAMID_LEVELS,
            adaptive: false,
            adaptive_threshold: adaptive::DEFAULT_THRESHOLD,
        }
    }
}

/// How many derived preview levels a pyramid holds by default.
pub const DEFAULT_PYRAMID_LEVELS: u32 = 3;

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

    /// Choose the codecs the per-tile selector may use.
    ///
    /// [`CodecSet::V0_3`] adds the three codecs promoted in format v0.3. A
    /// file that ends up using one of them is a **v0.3** file: its minor
    /// version is 3.
    pub fn with_codec_set(mut self, set: CodecSet) -> Self {
        self.codec_set = set;
        self
    }

    /// Let the selector use every implemented codec, including the three
    /// promoted in format v0.3.
    pub fn with_extended_codecs(mut self) -> Self {
        self.codec_set = CodecSet::V0_3;
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

    /// Build a preview pyramid with `levels` derived levels above the
    /// original.
    ///
    /// The file becomes a v0.2 file. The original is still level 0 and is
    /// still exact.
    pub fn with_pyramid(mut self, levels: u32) -> Self {
        self.pyramid = true;
        self.pyramid_levels = levels;
        self
    }

    /// Choose the tile sizes from the picture's content.
    ///
    /// The file becomes a v0.2 file. The partition is still an exact cover of
    /// the image: every pixel is in exactly one tile.
    pub fn with_adaptive(mut self) -> Self {
        self.adaptive = true;
        self
    }

    /// The busyness threshold of the adaptive planner.
    ///
    /// A tile whose mean neighbour difference is above the threshold is
    /// "busy" and is split; a calmer tile is left alone.
    pub fn with_adaptive_threshold(mut self, threshold: u32) -> Self {
        self.adaptive_threshold = threshold;
        self
    }

    /// Whether the options ask for anything that makes the file a v0.2 file.
    pub fn is_v2(&self) -> bool {
        self.pyramid || self.adaptive
    }

    /// Whether the options allow a codec that only a v0.3 file may name.
    ///
    /// The file's minor version is decided by what the file *contains*, not
    /// by what the options allow: a file that ends up using only the v0.1
    /// codecs stays a v0.1 file even when the extended set was permitted.
    pub fn allows_v0_3_codecs(&self) -> bool {
        self.codec_set == CodecSet::V0_3
            || (self.codec_id != CODEC_AUTO && vpng_codecs::is_v0_3_codec(self.codec_id))
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
    /// The number of levels in the file. One for a v0.1 file.
    pub level_count: u32,
    /// The number of tiles in each level, in level order.
    pub level_tile_counts: Vec<u32>,
    /// The number of payload bytes in each level, in level order.
    pub level_payload_bytes: Vec<u64>,
    /// The original image's pixel bytes: level 0's uncompressed bytes.
    ///
    /// A preview level's pixels are **not** counted here: the ratio below is
    /// always "original pixels over file bytes", so a pyramid cannot make the
    /// picture look better than it is.
    pub base_uncompressed_bytes: u64,
    /// The minor version the file's header carries.
    ///
    /// One for a v0.1 file, two for a file that uses a v0.2 feature, three
    /// for a file that names a codec promoted in v0.3.
    pub format_minor: u16,
}

impl EncodeReport {
    /// Ratio of uncompressed pixel bytes to file bytes. `1.0` means "no gain".
    ///
    /// The numerator is the **original** image's pixels and the denominator is
    /// the whole file, so a file with previews scores lower than the same
    /// picture without them. That is the honest number.
    pub fn compression_ratio(&self) -> f64 {
        if self.file_size == 0 {
            return 1.0;
        }
        self.base_uncompressed_bytes as f64 / self.file_size as f64
    }

    /// Bytes per pixel over the whole file, including container overhead.
    pub fn bytes_per_pixel(&self, pixel_count: u64) -> f64 {
        if pixel_count == 0 {
            return 0.0;
        }
        self.file_size as f64 / pixel_count as f64
    }

    /// The size the file would have if it held the original level only.
    ///
    /// The comparison keeps the denominator honest: a pyramid's previews are
    /// extra bytes, and this is how much of the file they account for.
    pub fn base_only_file_size(&self) -> u64 {
        let tiles = self.level_tile_counts.first().copied().unwrap_or(0) as u64;
        let payloads = self.level_payload_bytes.first().copied().unwrap_or(0);
        HEADER_SIZE as u64 + tiles * 40 + 4 + payloads
    }
}

/// One encoded tile: its codec, its payload and its canonical size.
pub(crate) struct TilePayload {
    pub(crate) codec_id: u8,
    pub(crate) bytes: Vec<u8>,
    pub(crate) uncompressed_length: u32,
}

/// One encoded level: its geometry, its partition and its tiles.
pub(crate) struct EncodedLevel {
    pub(crate) index: u16,
    pub(crate) partition: Partition,
    pub(crate) payloads: Vec<TilePayload>,
}

/// One planned preview level: its partition and its pixels.
pub(crate) struct PreviewPlan {
    pub(crate) partition: Partition,
    pub(crate) image: CanonicalImage,
}

/// The whole file's plan: the original level and any preview levels.
pub(crate) struct FilePlan {
    pub(crate) base: Partition,
    pub(crate) previews: Vec<PreviewPlan>,
}

/// Encode `image` into a complete `.vpng` byte stream.
pub fn encode(image: &CanonicalImage, options: &EncodeOptions) -> Result<EncodeReport> {
    let dims = image.dimensions();
    let plan = plan_file(image, options)?;
    let levels = encode_plan(image, &plan, options, &parallel::ParallelOptions::default())?;
    assemble(dims, image.format(), options, &levels)
}

/// Plan the file's levels from an image.
///
/// The original level's partition is a uniform grid, or an adaptive partition
/// when the options ask for one. The preview levels, if any, are uniform
/// grids over the downsampled image.
pub(crate) fn plan_file(image: &CanonicalImage, options: &EncodeOptions) -> Result<FilePlan> {
    validate_options(image.dimensions(), options)?;

    let base = if options.adaptive {
        let grid = uniform_grid(image.dimensions(), options)?;
        let mut adaptive_options =
            adaptive::AdaptiveOptions::for_nominal(options.tile_width, grid.tile_count()?);
        adaptive_options.threshold = options.adaptive_threshold;
        adaptive::plan_adaptive(image, &adaptive_options)?
    } else {
        uniform_partition(image.dimensions(), options)?
    };

    let mut previews = Vec::new();
    if options.pyramid {
        for preview in pyramid::build_previews(image, options.pyramid_levels)? {
            let partition = uniform_partition(preview.dimensions, options)?;
            previews.push(PreviewPlan {
                partition,
                image: preview.image,
            });
        }
    }

    Ok(FilePlan { base, previews })
}

/// Plan a file with a uniform original level and no previews.
///
/// This is the path that needs no access to the picture's content, so it
/// works with any pixel source.
pub(crate) fn plan_uniform(dims: Dimensions, options: &EncodeOptions) -> Result<FilePlan> {
    validate_options(dims, options)?;
    Ok(FilePlan {
        base: uniform_partition(dims, options)?,
        previews: Vec::new(),
    })
}

/// Check everything that must hold before any work starts, so a bad option
/// fails cleanly rather than partway through a file.
fn validate_options(dims: Dimensions, options: &EncodeOptions) -> Result<()> {
    options.limits.validate_dimensions(dims, "encode input")?;

    if options.tile_width == 0 || options.tile_height == 0 {
        return Err(Error::invalid("tile dimensions must be non-zero"));
    }
    if options.tile_width > u16::MAX as u32 || options.tile_height > u16::MAX as u32 {
        return Err(Error::unsupported(format!(
            "tile size {}x{} does not fit the 16-bit directory fields",
            options.tile_width, options.tile_height
        )));
    }

    // A forced codec must be supported before any work starts, so an unknown id
    // fails cleanly rather than partway through a file.
    if options.codec_id != CODEC_AUTO {
        let _ = get_codec(options.codec_id)?;
    }

    Ok(())
}

/// The uniform grid of the original level.
pub(crate) fn uniform_grid(dims: Dimensions, options: &EncodeOptions) -> Result<TileGrid> {
    let grid = TileGrid::new(
        options.tile_width,
        options.tile_height,
        dims.width,
        dims.height,
    )?;
    options.limits.validate_tile_count(grid.tile_count()?)?;
    Ok(grid)
}

/// The uniform partition of a level.
pub(crate) fn uniform_partition(dims: Dimensions, options: &EncodeOptions) -> Result<Partition> {
    let grid = uniform_grid(dims, options)?;
    grid.partition()
}

/// Encode every level of a plan.
pub(crate) fn encode_plan(
    level0: &dyn source::PixelSource,
    plan: &FilePlan,
    options: &EncodeOptions,
    parallel: &parallel::ParallelOptions,
) -> Result<Vec<EncodedLevel>> {
    let mut levels = Vec::with_capacity(1 + plan.previews.len());

    let payloads = parallel::encode_all_tiles(level0, &plan.base, options, parallel)?;
    levels.push(EncodedLevel {
        index: 0,
        partition: plan.base.clone(),
        payloads,
    });

    for (offset, preview) in plan.previews.iter().enumerate() {
        let payloads =
            parallel::encode_all_tiles(&preview.image, &preview.partition, options, parallel)?;
        levels.push(EncodedLevel {
            index: (offset + 1) as u16,
            partition: preview.partition.clone(),
            payloads,
        });
    }

    Ok(levels)
}

/// The minor version a file's **contents** call for.
///
/// The minor version states what the file contains, not what the options
/// allowed:
///
/// * a file that names a codec promoted in v0.3 (ids 4, 5 or 6) is a v0.3
///   file, because a v0.1 or v0.2 reader has no codec to decode it with;
/// * a file that uses a v0.2 feature (a preview pyramid or an adaptive
///   partition) is a v0.2 file;
/// * anything else is a v0.1 file, which is what the default path writes and
///   what the golden fixtures pin.
///
/// The batch assembler and the streaming writer both call this, so the two
/// cannot disagree about a file's version.
pub(crate) fn content_minor(codec_ids: &[u8], v2_features: bool) -> u16 {
    if codec_ids.iter().any(|&id| vpng_codecs::is_v0_3_codec(id)) {
        MINOR_V0_3
    } else if v2_features {
        MINOR_V0_2
    } else {
        MINOR_V0_1
    }
}

/// Build the header a file's contents call for.
///
/// The header layout is the same 64 bytes in every version; only the minor
/// version and the feature flags differ.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_header(
    minor: u16,
    dims: Dimensions,
    format: PixelFormat,
    default_codec: u8,
    tile_count: u32,
    declared_file_size: u64,
    feature_flags: u32,
) -> Result<Header> {
    let header = match minor {
        MINOR_V0_3 => Header::new_v3(
            dims.width,
            dims.height,
            format,
            default_codec,
            tile_count,
            HEADER_SIZE as u64,
            declared_file_size,
        )?,
        MINOR_V0_2 => Header::new_v2(
            dims.width,
            dims.height,
            format,
            default_codec,
            tile_count,
            HEADER_SIZE as u64,
            declared_file_size,
        )?,
        _ => Header::new(
            dims.width,
            dims.height,
            format,
            default_codec,
            tile_count,
            HEADER_SIZE as u64,
            declared_file_size,
        )?,
    };
    Ok(header.with_feature_flags(feature_flags))
}

/// Assemble the container from the encoded levels.
///
/// This is the single place where offsets, checksums and the header are
/// computed. The serial encoder, the parallel encoder and the streaming writer
/// all build the container the same way, which is why they cannot disagree
/// about the bytes.
pub(crate) fn assemble(
    dims: Dimensions,
    format: PixelFormat,
    options: &EncodeOptions,
    levels: &[EncodedLevel],
) -> Result<EncodeReport> {
    let mut tile_count = 0u32;
    for level in levels {
        tile_count = tile_count
            .checked_add(level.partition.len() as u32)
            .ok_or_else(|| Error::resource_limit("the file's tile count exceeds u32"))?;
    }

    let directory_size = Directory::total_size(tile_count)?;
    let mut cursor = vpng_core::checked_add(HEADER_SIZE as u64, directory_size as u64)?;

    let mut entries: Vec<DirectoryEntry> = Vec::with_capacity(tile_count as usize);
    let mut chosen_codecs: Vec<u8> = Vec::with_capacity(tile_count as usize);
    let mut compressed_lengths: Vec<u64> = Vec::with_capacity(tile_count as usize);
    let mut uncompressed_lengths: Vec<u32> = Vec::with_capacity(tile_count as usize);
    let mut level_tile_counts: Vec<u32> = Vec::with_capacity(levels.len());
    let mut level_payload_bytes: Vec<u64> = Vec::with_capacity(levels.len());

    for level in levels {
        let flags = DirectoryEntry::flags_for_level(level.index)?;
        let mut level_bytes = 0u64;
        for (tile, payload) in level.partition.tiles().iter().zip(level.payloads.iter()) {
            let checksum = crc32c(&payload.bytes);

            entries.push(DirectoryEntry {
                x: tile.x,
                y: tile.y,
                width: tile.width as u16,
                height: tile.height as u16,
                codec_id: payload.codec_id,
                predictor_id: 0,
                flags,
                payload_offset: cursor,
                compressed_length: payload.bytes.len() as u64,
                uncompressed_length: payload.uncompressed_length,
                payload_checksum: checksum,
            });

            cursor = vpng_core::checked_add(cursor, payload.bytes.len() as u64)?;
            level_bytes = vpng_core::checked_add(level_bytes, payload.bytes.len() as u64)?;
            chosen_codecs.push(payload.codec_id);
            compressed_lengths.push(payload.bytes.len() as u64);
            uncompressed_lengths.push(payload.uncompressed_length);
        }
        level_tile_counts.push(level.partition.len() as u32);
        level_payload_bytes.push(level_bytes);
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

    // The minor version states what the file *contains*, not what the options
    // allowed. A file that names a codec promoted in v0.3 needs a v0.3
    // reader; a file that does not, does not. The default path stays v0.1, so
    // the bytes the golden fixtures pin do not move.
    let minor = content_minor(&chosen_codecs, options.is_v2());

    // The pyramid flag and the levels must agree: a file with more than one
    // level says so, and a file that says so has more than one level.
    let feature_flags = if levels.len() > 1 { FEATURE_PYRAMID } else { 0 };
    let header = build_header(
        minor,
        dims,
        format,
        file_codec,
        tile_count,
        cursor,
        feature_flags,
    )?;

    let capacity = vpng_core::checked_usize(cursor)?;
    let mut bytes = Vec::with_capacity(capacity);
    header.write_to(&mut bytes)?;
    bytes.extend_from_slice(&directory_bytes);
    for level in levels {
        for payload in &level.payloads {
            bytes.extend_from_slice(&payload.bytes);
        }
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

    let base_uncompressed_bytes = levels
        .first()
        .map(|level| {
            level
                .payloads
                .iter()
                .map(|payload| payload.uncompressed_length as u64)
                .sum()
        })
        .unwrap_or(0);

    Ok(EncodeReport {
        bytes,
        tile_count,
        file_size: cursor,
        codec_ids: chosen_codecs,
        compressed_lengths,
        uncompressed_lengths,
        level_count: levels.len() as u32,
        level_tile_counts,
        level_payload_bytes,
        base_uncompressed_bytes,
        format_minor: minor,
    })
}

/// Encode one tile with the smallest payload the permitted codecs offer.
///
/// When [`EncodeOptions::codec_id`] forces a codec, only that codec is tried and
/// any error is returned. Otherwise every codec in [`EncodeOptions::codec_set`]
/// is tried in ascending id order; a codec that reports the tile as not
/// representable ([`ErrorKind::UnsupportedFeature`]) is skipped, and the
/// smallest payload wins. Because the ids are visited in ascending order and a
/// candidate must be *strictly* smaller to replace the incumbent, ties go to
/// the lowest id.
///
/// The function is public so the editing engine rewrites a tile exactly the
/// way the encoder would. Sharing the selection keeps a re-encoded tile from
/// differing from the tile the encoder would have produced.
pub fn select_payload(
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
    for &id in options.codec_set.ids() {
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
    extract_rect(
        image,
        &TileRect::new(geom.x, geom.y, geom.width, geom.height)?,
        out,
    )
}

/// Copy the pixels of a rectangle out of the image into a tight buffer.
///
/// This is the general form of [`extract_tile`]: it works for any rectangle,
/// so it works for an adaptive partition's tiles as well as a uniform grid's.
pub fn extract_rect(image: &CanonicalImage, rect: &TileRect, out: &mut Vec<u8>) -> Result<()> {
    let bpp = image.bytes_per_pixel() as usize;
    let src_row = image.row_bytes()? as usize;
    let tile_row = rect.width as usize * bpp;
    let total = tile_row * rect.height as usize;

    out.clear();
    out.reserve(total);

    let data = image.data();
    for row in 0..rect.height as usize {
        let start = (rect.y as usize + row) * src_row + rect.x as usize * bpp;
        out.extend_from_slice(&data[start..start + tile_row]);
    }

    if out.len() != total {
        return Err(Error::internal(
            "tile extraction produced an unexpected size",
        ));
    }
    Ok(())
}

/// Materialize a pixel source into a whole image.
///
/// The adaptive planner and the pyramid builder both need the picture's
/// content, and a generic pixel source cannot be asked for "some pixels" — it
/// can only produce one tile at a time. This reads the source tile by tile
/// and assembles the image, so the two features work with any source.
///
/// The cost is a whole-image buffer, which is why the default path (a uniform
/// grid, no previews) never calls it.
pub(crate) fn materialize(
    source: &dyn source::PixelSource,
    options: &EncodeOptions,
) -> Result<CanonicalImage> {
    let dims = Dimensions {
        width: source.width(),
        height: source.height(),
    };
    let partition = uniform_partition(dims, options)?;
    let mut image =
        CanonicalImage::new_zeroed(dims.width, dims.height, source.format(), &options.limits)?;

    let bpp = source.format().bytes_per_pixel() as usize;
    let row = image.row_bytes()? as usize;
    let mut pixels = Vec::new();

    for (position, tile) in partition.tiles().iter().enumerate() {
        source.tile_pixels(position, tile, &mut pixels)?;
        let expected = tile.width as usize * tile.height as usize * bpp;
        if pixels.len() != expected {
            return Err(Error::internal(format!(
                "tile {position} produced {} bytes, expected {expected}",
                pixels.len()
            )));
        }
        let tile_row = tile.width as usize * bpp;
        let data = image.data_mut();
        for row_index in 0..tile.height as usize {
            let src = row_index * tile_row;
            let dst = (tile.y as usize + row_index) * row + tile.x as usize * bpp;
            data[dst..dst + tile_row].copy_from_slice(&pixels[src..src + tile_row]);
        }
    }

    Ok(image)
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
    fn the_default_path_is_a_v0_1_file() {
        let image = gradient(300, 200, PixelFormat::rgba8());
        let report = encode(&image, &EncodeOptions::default()).unwrap();
        let header = Header::parse(&report.bytes).unwrap();
        assert_eq!(header.minor, 1);
        assert_eq!(header.feature_flags, 0);
        assert_eq!(report.level_count, 1);
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
    fn unknown_codec_is_rejected() {
        let image = gradient(64, 64, PixelFormat::rgba8());
        let options = EncodeOptions::default().with_codec(7);
        assert_eq!(
            encode(&image, &options).unwrap_err().kind(),
            vpng_core::ErrorKind::UnsupportedFeature
        );
    }

    #[test]
    fn a_promoted_codec_may_be_forced() {
        // Codec id 4 was reserved in v0.1 and v0.2. From v0.3 it is a stable
        // codec, and a file that names it says so in its minor version.
        let image = gradient(64, 64, PixelFormat::rgba8());
        let options = EncodeOptions::default().with_codec(vpng_codecs::CODEC_LZ77);
        let report = encode(&image, &options).unwrap();
        assert!(
            report
                .codec_ids
                .iter()
                .all(|&id| id == vpng_codecs::CODEC_LZ77),
            "every tile must use the forced codec"
        );
        assert_eq!(report.format_minor, vpng_container::MINOR_V0_3);
    }

    #[test]
    fn the_default_path_still_writes_a_v0_1_file() {
        // The default codec set is the four original codecs, so the default
        // file's bytes do not move.
        let image = gradient(64, 64, PixelFormat::rgba8());
        let report = encode(&image, &EncodeOptions::default()).unwrap();
        assert_eq!(report.format_minor, vpng_container::MINOR_V0_1);
        assert!(
            report
                .codec_ids
                .iter()
                .all(|&id| vpng_codecs::CODEC_IDS_V0_2.contains(&id)),
            "the default path must not name a v0.3 codec"
        );
    }

    /// A two-color image: the palette's ideal case.
    fn two_color(width: u32, height: u32, format: PixelFormat) -> CanonicalImage {
        let bpp = format.bytes_per_pixel() as usize;
        let mut data = vec![0u8; (width as usize) * (height as usize) * bpp];
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
        // A 4x2 grid where the top-left tile is flat and the rest are
        // full-color noise, so a compressing codec and Raw must both appear.
        // Every channel of the noise tiles is random, so no predictor can
        // predict them and Raw is the smallest representation.
        let width = 256u32;
        let height = 128u32;
        let mut data = vec![0u8; (width as usize) * (height as usize) * 4];
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
                if flat { 0xFF } else { (noise >> 24) as u8 },
            ]);
        }
        let image = CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), data).unwrap();
        let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();

        assert_eq!(report.tile_count, 8);
        assert_eq!(report.codec_ids.len(), 8);
        assert_ne!(
            report.codec_ids[0],
            vpng_codecs::CODEC_RAW,
            "the flat tile should be compressed by some codec"
        );
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

    /// A single flat color: the run-length codec's ideal case.
    fn flat(width: u32, height: u32, format: PixelFormat, value: u8) -> CanonicalImage {
        let bpp = format.bytes_per_pixel() as usize;
        let data = vec![value; (width as usize) * (height as usize) * bpp];
        CanonicalImage::from_vec(width, height, format, data).unwrap()
    }

    #[test]
    fn auto_selection_uses_rle_for_flat_tiles() {
        let image = flat(64, 64, PixelFormat::rgba8(), 0x42);
        let options = EncodeOptions::default().with_tile_size(64, 64);

        let auto = encode(&image, &options).unwrap();
        assert_eq!(
            auto.codec_ids,
            vec![vpng_codecs::CODEC_RLE],
            "a flat tile is a run: {:?}",
            auto.codec_ids
        );

        let raw = encode(&image, &options.with_codec(vpng_codecs::CODEC_RAW)).unwrap();
        let palette = encode(&image, &options.with_codec(vpng_codecs::CODEC_PALETTE)).unwrap();
        assert!(
            auto.file_size < raw.file_size && auto.file_size < palette.file_size,
            "rle {} should beat raw {} and palette {}",
            auto.file_size,
            raw.file_size,
            palette.file_size
        );
    }
}
