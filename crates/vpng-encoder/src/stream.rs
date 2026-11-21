//! The streaming encoder: a file out of a pixel source, without holding the
//! whole *output*.
//!
//! A VPNG file is the header, then the directory, then the payloads. The
//! directory records each payload's offset, and an offset is known only once
//! the length of every earlier payload is known — so the directory cannot be
//! written before the payloads.
//!
//! The streaming writer therefore writes the file twice, in one pass:
//!
//! 1. it reserves the header and the directory as zeroed placeholders;
//! 2. it encodes each tile and writes its payload straight to the file, in
//!    tile order;
//! 3. it seeks back and writes the real header and directory.
//!
//! The result is an ordinary VPNG file: the decoder cannot tell how it was
//! produced. With one worker it is byte-identical to [`crate::encode`]; with
//! several it is still byte-identical, because the per-tile work is
//! independent and the tiles are consumed in tile order.
//!
//! The file is written to a temporary name in the target's own directory and
//! renamed over the target once it is complete, so a reader never sees a
//! half-written image and a failed encode leaves no partial file behind.
//!
//! # v0.2 files
//!
//! A file with a preview pyramid holds several levels, and the writer writes
//! them one after the other: the original first, then the derived levels. The
//! directory is still written last, and the entries are in canonical
//! `(level, y, x)` order, so the file is exactly what the batch encoder would
//! have produced.
//!
//! The adaptive planner and the pyramid builder need the picture's content,
//! so a source that cannot be asked for "some pixels" is read into a whole
//! image first. The default path — a uniform grid and no previews — never
//! does that, and keeps its bounded memory.

use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use vpng_codecs::{CODEC_AUTO, CODEC_RAW};
use vpng_container::{crc32c, Directory, DirectoryEntry, Header, FEATURE_PYRAMID, HEADER_SIZE};
use vpng_core::{Dimensions, Error, Result};
use vpng_tiling::Partition;

use crate::parallel::{encode_tiles_in_order, ParallelOptions};
use crate::source::PixelSource;
use crate::EncodeOptions;

/// What the streaming encoder did.
#[derive(Debug, Clone)]
pub struct StreamEncodeReport {
    /// The size of the finished file.
    pub file_size: u64,
    /// The number of tiles.
    pub tile_count: u32,
    /// The number of bytes of tile payload.
    pub payload_bytes: u64,
    /// The number of bytes of container framing: the header and the directory.
    pub framing_bytes: u64,
    /// The codec each tile used.
    pub codec_ids: Vec<u8>,
    /// The encoded length of each tile's payload.
    pub compressed_lengths: Vec<u64>,
    /// The canonical length of each tile's pixels.
    pub uncompressed_lengths: Vec<u32>,
    /// The number of levels in the file. One for a v0.1 file.
    pub level_count: u32,
    /// The number of tiles in each level, in level order.
    pub level_tile_counts: Vec<u32>,
    /// The number of payload bytes in each level, in level order.
    pub level_payload_bytes: Vec<u64>,
    /// The original image's pixel bytes: level 0's uncompressed bytes.
    pub base_uncompressed_bytes: u64,
    /// How long the encode took, including the final write and rename.
    pub encode_ms: u64,
    /// How many workers were asked for.
    pub workers: usize,
}

impl StreamEncodeReport {
    /// Ratio of uncompressed pixel bytes to file bytes. `1.0` means "no gain".
    ///
    /// The numerator is the **original** image's pixels, so a file with
    /// previews scores lower than the same picture without them.
    pub fn compression_ratio(&self) -> f64 {
        if self.file_size == 0 {
            return 1.0;
        }
        self.base_uncompressed_bytes as f64 / self.file_size as f64
    }

    /// The size the file would have if it held the original level only.
    pub fn base_only_file_size(&self) -> u64 {
        let tiles = self.level_tile_counts.first().copied().unwrap_or(0) as u64;
        let payloads = self.level_payload_bytes.first().copied().unwrap_or(0);
        HEADER_SIZE as u64 + tiles * 40 + 4 + payloads
    }
}

/// Encode a pixel source straight into a `.vpng` file.
///
/// Only a bounded number of tiles is ever held in memory, so this can write
/// an image far larger than the available memory. The file is byte-identical
/// to the one [`crate::encode`] produces for the same pixels and options.
pub fn encode_to_file(
    source: &dyn PixelSource,
    options: &EncodeOptions,
    parallel: &ParallelOptions,
    path: &Path,
) -> Result<StreamEncodeReport> {
    let started = std::time::Instant::now();
    let dims = Dimensions {
        width: source.width(),
        height: source.height(),
    };

    // The adaptive planner and the pyramid builder need the picture's
    // content, so a source that cannot be asked for "some pixels" is read
    // into a whole image first. The default path never does that.
    let (materialized, plan) = if options.is_v2() {
        let image = crate::materialize(source, options)?;
        let plan = crate::plan_file(&image, options)?;
        (Some(image), plan)
    } else {
        (None, crate::plan_uniform(dims, options)?)
    };

    let base_source: &dyn PixelSource = match &materialized {
        Some(image) => image,
        None => source,
    };

    // The levels, in the order they are written: the original first, then the
    // derived previews.
    let mut levels: Vec<(&Partition, &dyn PixelSource, u16)> = vec![(&plan.base, base_source, 0)];
    for (offset, preview) in plan.previews.iter().enumerate() {
        levels.push((
            &preview.partition,
            &preview.image as &dyn PixelSource,
            (offset + 1) as u16,
        ));
    }

    let mut tile_count = 0u32;
    for (partition, _, _) in levels.iter() {
        tile_count = tile_count
            .checked_add(partition.len() as u32)
            .ok_or_else(|| Error::resource_limit("the file's tile count exceeds u32"))?;
    }
    let directory_size = Directory::total_size(tile_count)?;

    let temp = temporary_path(path);
    let report = match write_stream(
        source.format(),
        options,
        parallel,
        &levels,
        tile_count,
        directory_size,
        dims,
        &temp,
    ) {
        Ok(report) => report,
        Err(error) => {
            // The target has not been touched, so all that is left is to
            // remove the partial temporary file.
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
    };

    if let Some(limit) = options.max_size {
        if report.file_size > limit {
            // The smallest lossless representation the enabled codecs can
            // produce. It is reported, never violated.
            let _ = fs::remove_file(&temp);
            return Err(Error::size_limit(limit, report.file_size));
        }
    }

    // The file is complete. Replace the target in one step: a reader never
    // sees a half-written image.
    fs::rename(&temp, path)?;

    Ok(StreamEncodeReport {
        encode_ms: started.elapsed().as_millis() as u64,
        ..report
    })
}

/// Write the temporary file: placeholders, then the payloads, then the real
/// header and directory.
#[allow(clippy::too_many_arguments)]
fn write_stream(
    format: vpng_core::PixelFormat,
    options: &EncodeOptions,
    parallel: &ParallelOptions,
    levels: &[(&Partition, &dyn PixelSource, u16)],
    tile_count: u32,
    directory_size: usize,
    dims: Dimensions,
    path: &Path,
) -> Result<StreamEncodeReport> {
    let mut file = File::create(path)?;
    file.write_all(&vec![0u8; HEADER_SIZE + directory_size])?;

    let mut entries: Vec<DirectoryEntry> = Vec::with_capacity(tile_count as usize);
    let mut codec_ids: Vec<u8> = Vec::with_capacity(tile_count as usize);
    let mut compressed_lengths: Vec<u64> = Vec::with_capacity(tile_count as usize);
    let mut uncompressed_lengths: Vec<u32> = Vec::with_capacity(tile_count as usize);
    let mut level_tile_counts: Vec<u32> = Vec::with_capacity(levels.len());
    let mut level_payload_bytes: Vec<u64> = Vec::with_capacity(levels.len());
    let mut cursor = (HEADER_SIZE + directory_size) as u64;

    for (partition, source, index) in levels {
        let flags = DirectoryEntry::flags_for_level(*index)?;
        let mut level_bytes = 0u64;
        {
            let file = &mut file;
            encode_tiles_in_order(
                *source,
                partition,
                options,
                parallel,
                &mut |position, payload| {
                    let geom = partition.tile(position).ok_or_else(|| {
                        Error::internal(format!("tile {position} is out of range"))
                    })?;
                    let checksum = crc32c(&payload.bytes);
                    file.write_all(&payload.bytes)?;

                    entries.push(DirectoryEntry {
                        x: geom.x,
                        y: geom.y,
                        width: geom.width as u16,
                        height: geom.height as u16,
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
                    codec_ids.push(payload.codec_id);
                    compressed_lengths.push(payload.bytes.len() as u64);
                    uncompressed_lengths.push(payload.uncompressed_length);
                    Ok(())
                },
            )?;
        }
        level_tile_counts.push(partition.len() as u32);
        level_payload_bytes.push(level_bytes);
    }

    // The header's default codec is informational; the per-tile entry is
    // authoritative (ADR-0003). Record Raw for an auto-selected file.
    let file_codec = if options.codec_id == CODEC_AUTO {
        CODEC_RAW
    } else {
        options.codec_id
    };

    // A file is v0.2 when it uses a feature v0.2 defines. The default path
    // stays v0.1, so the bytes the golden fixtures pin do not move.
    let header = if options.is_v2() {
        Header::new_v2(
            dims.width,
            dims.height,
            format,
            file_codec,
            tile_count,
            HEADER_SIZE as u64,
            cursor,
        )?
    } else {
        Header::new(
            dims.width,
            dims.height,
            format,
            file_codec,
            tile_count,
            HEADER_SIZE as u64,
            cursor,
        )?
    };

    // The pyramid flag and the levels must agree.
    let feature_flags = if levels.len() > 1 { FEATURE_PYRAMID } else { 0 };
    let header = header.with_feature_flags(feature_flags);
    let header_bytes = header.to_bytes()?;
    let directory_bytes = Directory::new(entries).encode()?;

    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header_bytes)?;
    file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;
    file.write_all(&directory_bytes)?;
    file.flush()?;
    file.sync_all()?;

    let base_uncompressed_bytes = levels
        .first()
        .map(|(_, _, _)| {
            uncompressed_lengths
                .iter()
                .take(levels[0].0.len())
                .map(|&length| length as u64)
                .sum()
        })
        .unwrap_or(0);

    Ok(StreamEncodeReport {
        file_size: cursor,
        tile_count,
        payload_bytes: compressed_lengths.iter().sum(),
        framing_bytes: (HEADER_SIZE + directory_size) as u64,
        codec_ids,
        compressed_lengths,
        uncompressed_lengths,
        level_count: levels.len() as u32,
        level_tile_counts,
        level_payload_bytes,
        base_uncompressed_bytes,
        encode_ms: 0,
        workers: parallel.workers,
    })
}

/// The temporary file's name, in the target's own directory so the rename
/// stays within one volume.
fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".partial-{}", std::process::id()));
    PathBuf::from(name)
}
