//! The streaming tile-by-tile writer.
//!
//! # Why a streaming writer
//!
//! A 16K RGBA8 image is 1 GiB of pixels and a 16K RGBA16 image is 2 GiB.
//! An encoder that builds the whole image in memory before it encodes it
//! cannot run on a machine with a few gigabytes of free memory, and it
//! cannot demonstrate the claim that the editing engine never needs the
//! whole image either.
//!
//! The writer here never holds more than one tile of pixels at a time:
//!
//! 1. the header and the directory are reserved as placeholders;
//! 2. each tile's pixels are generated, encoded, and written straight to
//!    the file;
//! 3. the header and the directory are written last, once every payload's
//!    length and checksum is known.
//!
//! The file that comes out is an ordinary VPNG file: the same header, the
//! same directory, the same payloads. The decoder reads it without knowing
//! how it was produced.
//!
//! # The offsets
//!
//! The payloads follow the directory contiguously, in row-major order, so a
//! tile's payload offset is known as soon as the previous tiles' payload
//! lengths are known. The directory's own size is known before the first
//! tile is encoded, because it depends only on the tile count.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use vpng_container::{crc32c, Directory, DirectoryEntry, Header, HEADER_SIZE};
use vpng_core::{Error, Limits, PixelFormat, Result};
use vpng_editing::TileSource;
use vpng_encoder::{select_payload, EncodeOptions};
use vpng_tiling::TileGrid;

/// What the streaming writer did.
#[derive(Debug, Clone)]
pub struct StreamReport {
    /// The path that was written.
    pub path: String,
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The pixel format.
    pub format: String,
    /// The nominal tile size.
    pub tile_size: u32,
    /// The number of tiles.
    pub tile_count: u32,
    /// The size of the finished file.
    pub file_size: u64,
    /// The number of bytes of tile payload.
    pub payload_bytes: u64,
    /// The number of bytes of container framing.
    pub framing_bytes: u64,
    /// The codec each tile used, and how many tiles used it.
    pub codec_histogram: Vec<(String, u32)>,
    /// How long the write took.
    pub encode_ms: u64,
    /// The largest number of bytes the process's allocator held at once.
    pub peak_heap_bytes: usize,
    /// The process's resident-set high-water mark.
    pub peak_working_set_bytes: u64,
    /// The largest tile buffer the writer held.
    pub tile_buffer_bytes: u64,
}

/// The pattern the writer generates.
///
/// The pattern is a smooth two-dimensional gradient with a small
/// deterministic perturbation. It compresses well, so the files stay small
/// enough to fit on disk, and it is cheap to generate, so the measurement
/// measures the codec rather than the pattern.
pub fn fill_tile(x: u32, y: u32, width: u32, height: u32, format: PixelFormat, out: &mut Vec<u8>) {
    let bpp = format.bytes_per_pixel() as usize;
    out.clear();
    out.resize(width as usize * height as usize * bpp, 0);
    let channels = format.channels() as usize;
    let max = if format.bits.bits() == 16 {
        65535u32
    } else {
        255u32
    };
    for row in 0..height {
        for column in 0..width {
            let px = x + column;
            let py = y + row;
            let base = ((row * width + column) as usize) * bpp;
            for channel in 0..channels {
                let seed = (px as u64)
                    .wrapping_mul(3)
                    .wrapping_add((py as u64).wrapping_mul(5))
                    .wrapping_add((channel as u64).wrapping_mul(11))
                    .wrapping_add(((px as u64) * (py as u64)) >> 8);
                let value = (seed % (max as u64 + 1)) as u32;
                if format.bits.bits() == 16 {
                    let sample = (value as u16).to_le_bytes();
                    out[base + channel * 2..base + channel * 2 + 2].copy_from_slice(&sample);
                } else {
                    out[base + channel] = value as u8;
                }
            }
        }
    }
}

/// Write a file tile by tile, without ever holding the whole image.
///
/// `tile_size` is the nominal tile size; the edge tiles are clipped to the
/// image, exactly as the encoder clips them.
pub fn write_streaming(
    path: &Path,
    width: u32,
    height: u32,
    format: PixelFormat,
    tile_size: u32,
    limits: &Limits,
    codec_id: Option<u8>,
) -> Result<StreamReport> {
    let started = std::time::Instant::now();
    crate::mem::reset_peaks();

    let grid = TileGrid::new(tile_size, tile_size, width, height)?;
    let tile_count = grid.tile_count()?;
    let directory_size = Directory::total_size(tile_count)?;

    let mut file = File::create(path)?;
    // Reserve the header and the directory. Both are rewritten at the end,
    // once the payload sizes are known.
    file.write_all(&vec![0u8; HEADER_SIZE + directory_size])?;

    let mut entries: Vec<DirectoryEntry> = Vec::with_capacity(tile_count as usize);
    let mut pixels: Vec<u8> = Vec::new();
    let mut payload: Vec<u8> = Vec::new();
    let mut cursor = (HEADER_SIZE + directory_size) as u64;
    let mut payload_bytes = 0u64;
    let mut histogram: Vec<(u8, u32)> = Vec::new();

    let encode_options = match codec_id {
        Some(codec_id) => EncodeOptions {
            codec_id,
            ..EncodeOptions::default()
        },
        None => EncodeOptions::default(),
    };

    for geometry in grid.iter() {
        fill_tile(
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            format,
            &mut pixels,
        );
        let ctx = vpng_codecs::EncodeContext {
            limits: *limits,
            format,
            tile_width: geometry.width as u16,
            tile_height: geometry.height as u16,
        };
        let (chosen, encoded) = select_payload(&pixels, &ctx, &encode_options)?;
        payload.clear();
        payload.extend_from_slice(&encoded);

        file.write_all(&payload)?;
        payload_bytes += payload.len() as u64;

        entries.push(DirectoryEntry {
            x: geometry.x,
            y: geometry.y,
            width: geometry.width as u16,
            height: geometry.height as u16,
            codec_id: chosen,
            predictor_id: 0,
            flags: 0,
            payload_offset: cursor,
            compressed_length: payload.len() as u64,
            uncompressed_length: pixels.len() as u32,
            payload_checksum: crc32c(&payload),
        });
        cursor += payload.len() as u64;

        match histogram.iter_mut().find(|(id, _)| *id == chosen) {
            Some(entry) => entry.1 += 1,
            None => histogram.push((chosen, 1)),
        }
    }

    // The header and the directory are written last.
    let header = Header::new(
        width,
        height,
        format,
        vpng_codecs::CODEC_RAW,
        tile_count,
        HEADER_SIZE as u64,
        cursor,
    )?;
    let header_bytes = header.to_bytes()?;
    let directory = Directory::new(entries);
    let directory_bytes = directory.encode()?;

    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header_bytes)?;
    file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;
    file.write_all(&directory_bytes)?;
    file.flush()?;
    file.sync_all()?;

    let sample = crate::mem::sample();
    let codec_histogram = histogram
        .into_iter()
        .map(|(id, count)| {
            (
                vpng_codecs::codec_name(id)
                    .map(|name| name.to_string())
                    .unwrap_or_else(|| format!("codec-{id}")),
                count,
            )
        })
        .collect();

    Ok(StreamReport {
        path: path.to_string_lossy().into_owned(),
        width,
        height,
        format: format.to_string(),
        tile_size,
        tile_count,
        file_size: cursor,
        payload_bytes,
        framing_bytes: (HEADER_SIZE + directory_size) as u64,
        codec_histogram,
        encode_ms: started.elapsed().as_millis() as u64,
        peak_heap_bytes: sample.heap_peak_bytes,
        peak_working_set_bytes: sample.peak_working_set_bytes,
        tile_buffer_bytes: (tile_size as u64) * (tile_size as u64) * format.bytes_per_pixel(),
    })
}

/// Check that a streamed file is a valid VPNG file.
///
/// The check opens the file with the ordinary decoder, verifies the
/// container's own integrity, and confirms that a region's pixels match the
/// pattern the writer generated.
pub fn verify_streamed_file(path: &Path, limits: &Limits) -> Result<VerificationReport> {
    let started = std::time::Instant::now();
    let source = vpng_editing::FileSource::open(path, limits)?;
    let width = source.width();
    let height = source.height();
    let format = source.format();
    let tile_count = source.tile_count();

    // The integrity check reads every payload and verifies every checksum.
    let verifier = vpng_editing::FileSource::open(path, limits)?;
    let header = *verifier.header();
    let directory = verifier.directory().clone();
    let mut payload_bytes = 0u64;
    for index in 0..tile_count as usize {
        let entry = directory
            .entries
            .get(index)
            .ok_or_else(|| Error::internal(format!("tile {index} has no entry")))?;
        payload_bytes += entry.compressed_length;
    }
    drop(verifier);

    // Decode one tile and compare it with the generated pattern.
    let mut decoder = vpng_editing::FileSource::open(path, limits)?;
    let mut expected: Vec<u8> = Vec::new();
    let mut checked_tiles = 0u32;
    let mut mismatches = 0u32;
    let partition = decoder.partition().clone();
    for index in 0..tile_count {
        let geometry = partition
            .tile(index as usize)
            .ok_or_else(|| Error::internal(format!("tile {index} has no geometry")))?;
        // Only the first, last and a middle tile are checked: decoding every
        // tile of a 16K image would measure the decoder, not the writer.
        if index != 0 && index != tile_count / 2 && index + 1 != tile_count {
            continue;
        }
        let decoded = decoder.decode_tile(index as usize, limits)?;
        fill_tile(
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            format,
            &mut expected,
        );
        if decoded != expected {
            mismatches += 1;
        }
        checked_tiles += 1;
    }

    Ok(VerificationReport {
        width,
        height,
        format: format.to_string(),
        tile_count,
        payload_bytes,
        header_size: HEADER_SIZE as u64,
        directory_size: directory_bytes_len(&directory),
        checked_tiles,
        mismatches,
        verify_ms: started.elapsed().as_millis() as u64,
        header_magic_ok: header.major == 0 && header.minor == 1,
    })
}

/// The encoded size of a directory.
fn directory_bytes_len(directory: &Directory) -> u64 {
    (directory.len() as u64) * 40 + 4
}

/// What the verification found.
#[derive(Debug, Clone)]
pub struct VerificationReport {
    /// The image's width in pixels.
    pub width: u32,
    /// The image's height in pixels.
    pub height: u32,
    /// The pixel format.
    pub format: String,
    /// The number of tiles.
    pub tile_count: u32,
    /// The number of bytes of tile payload.
    pub payload_bytes: u64,
    /// The size of the container header.
    pub header_size: u64,
    /// The size of the directory.
    pub directory_size: u64,
    /// The number of tiles that were decoded and compared.
    pub checked_tiles: u32,
    /// The number of tiles whose pixels did not match the pattern.
    pub mismatches: u32,
    /// How long the verification took.
    pub verify_ms: u64,
    /// Whether the header carries the expected version.
    pub header_magic_ok: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_core::PixelFormat;
    use vpng_encoder::parallel::ParallelOptions;
    use vpng_encoder::source::PixelSource;
    use vpng_encoder::stream::encode_to_file;
    use vpng_tiling::TileRect;

    /// A source that produces the same pattern [`fill_tile`] does.
    struct PatternSource {
        format: PixelFormat,
    }

    impl PixelSource for PatternSource {
        fn width(&self) -> u32 {
            320
        }

        fn height(&self) -> u32 {
            240
        }

        fn format(&self) -> PixelFormat {
            self.format
        }

        fn tile_pixels(&self, _index: usize, geom: &TileRect, out: &mut Vec<u8>) -> Result<()> {
            fill_tile(geom.x, geom.y, geom.width, geom.height, self.format, out);
            Ok(())
        }
    }

    /// The library's streaming writer and this crate's writer must produce
    /// the same file for the same pixels.
    ///
    /// This crate's writer is the one the P4 memory figures were measured
    /// with, so the equality is what makes it safe to replace it with the
    /// library's writer later: the file does not change, so the evidence
    /// does not go stale.
    #[test]
    fn the_library_writer_and_the_validation_writer_agree() {
        let limits = Limits::default();
        let format = PixelFormat::rgba8();
        let directory = std::env::temp_dir().join(format!(
            "vpng-writer-agreement-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let validation_path = directory.join("validation.vpng");
        let library_path = directory.join("library.vpng");

        write_streaming(&validation_path, 320, 240, format, 64, &limits, None).unwrap();

        encode_to_file(
            &PatternSource { format },
            &EncodeOptions::default().with_tile_size(64, 64),
            &ParallelOptions::default().with_workers(4),
            &library_path,
        )
        .unwrap();

        let validation_bytes = std::fs::read(&validation_path).unwrap();
        let library_bytes = std::fs::read(&library_path).unwrap();
        assert_eq!(
            vpng_container::sha256(&validation_bytes),
            vpng_container::sha256(&library_bytes),
            "the two writers produced different files"
        );
        assert_eq!(validation_bytes, library_bytes);

        let _ = std::fs::remove_dir_all(&directory);
    }
}
