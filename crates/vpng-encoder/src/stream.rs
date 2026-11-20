//! The streaming encoder: a file out of a pixel source, without holding the
//! whole image.
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

use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use vpng_codecs::{CODEC_AUTO, CODEC_RAW};
use vpng_container::{crc32c, Directory, DirectoryEntry, Header, HEADER_SIZE};
use vpng_core::{Dimensions, Error, Result};
use vpng_tiling::TileGrid;

use crate::parallel::{encode_tiles_in_order, ParallelOptions};
use crate::source::PixelSource;
use crate::{plan_grid, EncodeOptions};

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
    /// How long the encode took, including the final write and rename.
    pub encode_ms: u64,
    /// How many workers were asked for.
    pub workers: usize,
}

impl StreamEncodeReport {
    /// Ratio of uncompressed pixel bytes to file bytes. `1.0` means "no gain".
    pub fn compression_ratio(&self) -> f64 {
        let total: u64 = self.uncompressed_lengths.iter().map(|&v| v as u64).sum();
        if self.file_size == 0 {
            return 1.0;
        }
        total as f64 / self.file_size as f64
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
    let grid = plan_grid(dims, options)?;
    let tile_count = grid.tile_count()?;
    let directory_size = Directory::total_size(tile_count)?;

    let temp = temporary_path(path);
    let report = match write_stream(
        source,
        options,
        parallel,
        &grid,
        tile_count,
        directory_size,
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
    source: &dyn PixelSource,
    options: &EncodeOptions,
    parallel: &ParallelOptions,
    grid: &TileGrid,
    tile_count: u32,
    directory_size: usize,
    path: &Path,
) -> Result<StreamEncodeReport> {
    let mut file = File::create(path)?;
    file.write_all(&vec![0u8; HEADER_SIZE + directory_size])?;

    let mut entries: Vec<DirectoryEntry> = Vec::with_capacity(tile_count as usize);
    let mut codec_ids: Vec<u8> = Vec::with_capacity(tile_count as usize);
    let mut compressed_lengths: Vec<u64> = Vec::with_capacity(tile_count as usize);
    let mut uncompressed_lengths: Vec<u32> = Vec::with_capacity(tile_count as usize);
    let mut cursor = (HEADER_SIZE + directory_size) as u64;

    {
        let file = &mut file;
        encode_tiles_in_order(source, grid, options, parallel, &mut |index, payload| {
            let geom = grid.geometry_for_index(index)?;
            let checksum = crc32c(&payload.bytes);
            file.write_all(&payload.bytes)?;

            entries.push(DirectoryEntry {
                x: geom.x,
                y: geom.y,
                width: geom.width as u16,
                height: geom.height as u16,
                codec_id: payload.codec_id,
                predictor_id: 0,
                flags: 0,
                payload_offset: cursor,
                compressed_length: payload.bytes.len() as u64,
                uncompressed_length: payload.uncompressed_length,
                payload_checksum: checksum,
            });
            cursor = vpng_core::checked_add(cursor, payload.bytes.len() as u64)?;
            codec_ids.push(payload.codec_id);
            compressed_lengths.push(payload.bytes.len() as u64);
            uncompressed_lengths.push(payload.uncompressed_length);
            Ok(())
        })?;
    }

    // The header's default codec is informational; the per-tile entry is
    // authoritative (ADR-0003). Record Raw for an auto-selected file.
    let file_codec = if options.codec_id == CODEC_AUTO {
        CODEC_RAW
    } else {
        options.codec_id
    };

    let dims = grid.dimensions();
    let header = Header::new(
        dims.width,
        dims.height,
        source.format(),
        file_codec,
        tile_count,
        HEADER_SIZE as u64,
        cursor,
    )?;
    let header_bytes = header.to_bytes()?;
    let directory_bytes = Directory::new(entries).encode()?;

    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header_bytes)?;
    file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;
    file.write_all(&directory_bytes)?;
    file.flush()?;
    file.sync_all()?;

    Ok(StreamEncodeReport {
        file_size: cursor,
        tile_count,
        payload_bytes: compressed_lengths.iter().sum(),
        framing_bytes: (HEADER_SIZE + directory_size) as u64,
        codec_ids,
        compressed_lengths,
        uncompressed_lengths,
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
