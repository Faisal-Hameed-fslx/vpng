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
//! # v0.4 files
//!
//! A file with layers holds several grids on one canvas, and the writer writes
//! them one after the other: the base layer first, then the layers above it.
//! The layer table sits between the directory and the payloads. Its **length**
//! is known before the payloads are written — it depends only on the layer
//! names — so the table's space is reserved with the header and the
//! directory. Its **contents**, which carry each layer's byte accounting, are
//! known only once every payload exists, so the table is written back after
//! the payloads, exactly like the directory.
//!
//! The adaptive planner and the pyramid builder need the picture's content,
//! so a source that cannot be asked for "some pixels" is read into a whole
//! image first. The default path — a uniform grid and no previews — never
//! does that, and keeps its bounded memory.

use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use vpng_codecs::{CODEC_AUTO, CODEC_RAW};
use vpng_container::{
    crc32c, Directory, DirectoryEntry, GpuRecord, GpuTable, LayerDescriptor, LayerTable,
    FEATURE_GPU, FEATURE_LAYERS, FEATURE_PYRAMID, GPU_RECORD_SIZE, GPU_TABLE_HEADER_SIZE,
    HEADER_SIZE, HEADER_SIZE_V0_5, LAYER_DESCRIPTOR_SIZE, LAYER_TABLE_HEADER_SIZE,
};
use vpng_core::{Dimensions, Error, Result};
use vpng_tiling::Partition;

use crate::layers::LayerInput;
use crate::parallel::{encode_tiles_in_order, ParallelOptions};
use crate::source::PixelSource;
use crate::{EncodeOptions, LayerMeta};

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
    /// The number of layers in the file. One for every file written before
    /// v0.4.
    pub layer_count: u32,
    /// The number of tiles in each layer, in layer order.
    pub layer_tile_counts: Vec<u32>,
    /// The uncompressed pixel bytes of each layer, in layer order.
    pub layer_uncompressed_bytes: Vec<u64>,
    /// The compressed payload bytes of each layer, in layer order.
    pub layer_compressed_bytes: Vec<u64>,
    /// The number of bytes the file's GPU payloads occupy.
    ///
    /// Zero for every file written before v0.5.
    pub gpu_payload_bytes: u64,
    /// The number of bytes the file's GPU payload table occupies.
    pub gpu_table_bytes: u64,
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
        (None, crate::plan_uniform(dims, source.format(), options)?)
    };

    let base_source: &dyn PixelSource = match &materialized {
        Some(image) => image,
        None => source,
    };

    // The levels, in the order they are written: the original first, then the
    // derived previews.
    let mut levels: Vec<StreamLevel<'_>> = vec![StreamLevel {
        partition: &plan.base,
        source: base_source,
        index: 0,
        layer: 0,
    }];
    for (offset, preview) in plan.previews.iter().enumerate() {
        levels.push(StreamLevel {
            partition: &preview.partition,
            source: &preview.image as &dyn PixelSource,
            index: (offset + 1) as u16,
            layer: 0,
        });
    }

    write_layers_to_file(levels, None, options, parallel, path)
}

/// One level of a file being streamed out: its partition, its pixels, its
/// level index and its layer index.
struct StreamLevel<'a> {
    partition: &'a Partition,
    source: &'a dyn PixelSource,
    index: u16,
    layer: u16,
}

/// Encode several layers straight into a `.vpng` file.
///
/// The file is a **v0.4** file: it carries a layer table, and its directory
/// entries name the layer each tile belongs to. The layers are encoded
/// independently of one another, and the result is byte-identical to
/// [`crate::encode_layers`].
pub fn encode_layers_to_file(
    layers: &[LayerInput<'_>],
    options: &EncodeOptions,
    parallel: &ParallelOptions,
    path: &Path,
) -> Result<StreamEncodeReport> {
    let (planned, _dims, _format) = crate::layers::plan_layers(layers, options)?;

    // Each layer's tiles carry canvas coordinates; the pixels live in the
    // layer's own rectangle, so the coordinates are translated on the way in.
    let sources: Vec<crate::layers::LayerSource<'_>> = planned
        .iter()
        .map(|layer| crate::layers::LayerSource::new(layer.source, layer.rect, layer.kind))
        .collect();

    let levels: Vec<StreamLevel<'_>> = planned
        .iter()
        .enumerate()
        .map(|(index, layer)| StreamLevel {
            partition: &layer.partition,
            source: &sources[index] as &dyn PixelSource,
            index: 0,
            layer: index as u16,
        })
        .collect();

    let meta: Vec<LayerMeta> = planned.iter().map(|layer| layer.meta()).collect();

    write_layers_to_file(levels, Some(&meta), options, parallel, path)
}

/// Write a file out of its levels, with or without a layer table.
fn write_layers_to_file(
    levels: Vec<StreamLevel<'_>>,
    layer_meta: Option<&[LayerMeta]>,
    options: &EncodeOptions,
    parallel: &ParallelOptions,
    path: &Path,
) -> Result<StreamEncodeReport> {
    let started = std::time::Instant::now();
    let dims = Dimensions {
        width: levels[0].source.width(),
        height: levels[0].source.height(),
    };

    let mut tile_count = 0u32;
    for level in levels.iter() {
        tile_count = tile_count
            .checked_add(level.partition.len() as u32)
            .ok_or_else(|| Error::resource_limit("the file's tile count exceeds u32"))?;
    }
    let directory_size = Directory::total_size(tile_count)?;

    // The layer table's length is known before the payloads are written: it
    // depends only on the number of layers and their names.
    let layer_table_size = match layer_meta {
        None => 0usize,
        Some(meta) => {
            let mut size = LAYER_TABLE_HEADER_SIZE;
            for layer in meta {
                size += LAYER_DESCRIPTOR_SIZE + layer.name.len();
            }
            size
        }
    };

    // The GPU table holds one fixed-size record per tile, so its length is
    // known before the payloads are written too.
    let has_gpu = options.gpu;
    let gpu_table_size = if has_gpu {
        GPU_TABLE_HEADER_SIZE + GPU_RECORD_SIZE * tile_count as usize
    } else {
        0
    };

    let temp = temporary_path(path);
    let report = match write_stream(
        source_format(&levels),
        options,
        parallel,
        &levels,
        tile_count,
        directory_size,
        layer_table_size,
        gpu_table_size,
        layer_meta,
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

/// The pixel format the levels share.
fn source_format(levels: &[StreamLevel<'_>]) -> vpng_core::PixelFormat {
    levels[0].source.format()
}

/// Write the temporary file: placeholders, then the payloads, then the real
/// header, directory and layer table.
#[allow(clippy::too_many_arguments)]
fn write_stream(
    format: vpng_core::PixelFormat,
    options: &EncodeOptions,
    parallel: &ParallelOptions,
    levels: &[StreamLevel<'_>],
    tile_count: u32,
    directory_size: usize,
    layer_table_size: usize,
    gpu_table_size: usize,
    layer_meta: Option<&[LayerMeta]>,
    dims: Dimensions,
    path: &Path,
) -> Result<StreamEncodeReport> {
    let mut file = File::create(path)?;
    let has_gpu = options.gpu;
    let header_size = if has_gpu {
        HEADER_SIZE_V0_5
    } else {
        HEADER_SIZE
    };
    let reserved = header_size + directory_size + layer_table_size + gpu_table_size;
    file.write_all(&vec![0u8; reserved])?;

    let mut entries: Vec<DirectoryEntry> = Vec::with_capacity(tile_count as usize);
    let mut codec_ids: Vec<u8> = Vec::with_capacity(tile_count as usize);
    let mut compressed_lengths: Vec<u64> = Vec::with_capacity(tile_count as usize);
    let mut uncompressed_lengths: Vec<u32> = Vec::with_capacity(tile_count as usize);
    let mut gpu_records: Vec<GpuRecord> = Vec::with_capacity(tile_count as usize);
    let mut gpu_payload_bytes = 0u64;
    let mut level_tile_counts: Vec<u32> = Vec::with_capacity(levels.len());
    let mut level_payload_bytes: Vec<u64> = Vec::with_capacity(levels.len());
    let mut cursor = reserved as u64;

    for level in levels {
        let flags = DirectoryEntry::flags_for_level_and_layer(level.index, level.layer)?;
        let mut level_bytes = 0u64;
        {
            let file = &mut file;
            encode_tiles_in_order(
                level.source,
                level.partition,
                options,
                parallel,
                &mut |position, payload| {
                    let geom = level.partition.tile(position).ok_or_else(|| {
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

                    // The tile's GPU payload follows its lossless payload.
                    let record = match payload.gpu.as_ref() {
                        Some(gpu) => {
                            let bytes = gpu.bytes();
                            let length = u32::try_from(bytes.len()).map_err(|_| {
                                Error::resource_limit("a tile's GPU payload length exceeds u32")
                            })?;
                            let record = GpuRecord {
                                payload_offset: cursor,
                                payload_length: length,
                                colour_codec: gpu.colour_codec().map_or(0, |codec| codec.id()),
                                alpha_codec: gpu.alpha_codec().map_or(0, |codec| codec.id()),
                                payload_checksum: crc32c(&bytes),
                            };
                            file.write_all(&bytes)?;
                            cursor = vpng_core::checked_add(cursor, bytes.len() as u64)?;
                            gpu_payload_bytes =
                                vpng_core::checked_add(gpu_payload_bytes, bytes.len() as u64)?;
                            record
                        }
                        None => GpuRecord {
                            payload_offset: 0,
                            payload_length: 0,
                            colour_codec: 0,
                            alpha_codec: 0,
                            payload_checksum: 0,
                        },
                    };
                    gpu_records.push(record);
                    Ok(())
                },
            )?;
        }
        level_tile_counts.push(level.partition.len() as u32);
        level_payload_bytes.push(level_bytes);
    }

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
    //
    // The rule is shared with the batch assembler, so the streaming writer
    // and `encode` cannot disagree about a file's version.
    let has_layers = layer_meta.is_some();
    let minor = crate::content_minor(&codec_ids, options.is_v2(), has_layers, has_gpu);

    // The pyramid flag and the levels must agree. A layered file has exactly
    // one level per layer, so it never has a pyramid, and the two flags are
    // mutually exclusive in v0.4. The GPU flag is orthogonal to both.
    let mut feature_flags = 0u32;
    if has_layers {
        feature_flags |= FEATURE_LAYERS;
    } else if levels.len() > 1 {
        feature_flags |= FEATURE_PYRAMID;
    }
    if has_gpu {
        feature_flags |= FEATURE_GPU;
    }

    // The layer table, whose contents are the per-layer byte accounting. The
    // accounting is known only now, because the payloads are.
    let layer_table_range = match layer_meta {
        None => None,
        Some(meta) => {
            let descriptors =
                layer_descriptors(meta, levels, &uncompressed_lengths, &compressed_lengths)?;
            let table = LayerTable::new(descriptors);
            table.validate()?;
            let bytes = table.encode()?;
            if bytes.len() != layer_table_size {
                return Err(Error::internal(format!(
                    "the layer table is {} bytes, the reservation was {layer_table_size}",
                    bytes.len()
                )));
            }
            let offset = (header_size + directory_size) as u64;
            file.seek(SeekFrom::Start(offset))?;
            file.write_all(&bytes)?;
            Some((offset as u32, bytes.len() as u32))
        }
    };

    // The GPU table, whose records are the per-tile GPU payload locators.
    let gpu_table_range = if has_gpu {
        let table = GpuTable::new(gpu_records);
        let bytes = table.encode()?;
        if bytes.len() != gpu_table_size {
            return Err(Error::internal(format!(
                "the GPU table is {} bytes, the reservation was {gpu_table_size}",
                bytes.len()
            )));
        }
        let offset = (header_size + directory_size + layer_table_size) as u64;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&bytes)?;
        Some((offset as u32, bytes.len() as u32))
    } else {
        None
    };

    let header = crate::build_header(
        minor,
        dims,
        format,
        file_codec,
        tile_count,
        cursor,
        feature_flags,
        layer_table_range,
        gpu_table_range,
    )?;
    let header_bytes = header.to_bytes()?;
    let directory_bytes = Directory::new(entries).encode()?;

    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header_bytes)?;
    file.seek(SeekFrom::Start(header_size as u64))?;
    file.write_all(&directory_bytes)?;
    file.flush()?;
    file.sync_all()?;

    let base_uncompressed_bytes = levels
        .first()
        .map(|_| {
            uncompressed_lengths
                .iter()
                .take(levels[0].partition.len())
                .map(|&length| length as u64)
                .sum()
        })
        .unwrap_or(0);

    let (layer_tile_counts, layer_uncompressed_bytes, layer_compressed_bytes) = layer_accounting(
        levels,
        has_layers,
        &uncompressed_lengths,
        &compressed_lengths,
    );

    Ok(StreamEncodeReport {
        file_size: cursor,
        tile_count,
        payload_bytes: compressed_lengths.iter().sum(),
        framing_bytes: (header_size + directory_size + layer_table_size + gpu_table_size) as u64,
        codec_ids,
        compressed_lengths,
        uncompressed_lengths,
        level_count: levels.len() as u32,
        level_tile_counts,
        level_payload_bytes,
        base_uncompressed_bytes,
        layer_count: layer_tile_counts.len() as u32,
        layer_tile_counts,
        layer_uncompressed_bytes,
        layer_compressed_bytes,
        gpu_payload_bytes,
        gpu_table_bytes: gpu_table_size as u64,
        encode_ms: 0,
        workers: parallel.workers,
    })
}

/// Build the layer table's descriptors, with the byte accounting filled in.
fn layer_descriptors(
    meta: &[LayerMeta],
    levels: &[StreamLevel<'_>],
    uncompressed_lengths: &[u32],
    compressed_lengths: &[u64],
) -> Result<Vec<LayerDescriptor>> {
    let mut descriptors = Vec::with_capacity(meta.len());
    let mut cursor = 0usize;
    for (index, layer) in meta.iter().enumerate() {
        let tiles: usize = levels
            .iter()
            .filter(|level| level.layer == index as u16)
            .map(|level| level.partition.len())
            .sum();

        let mut uncompressed = 0u64;
        let mut compressed = 0u64;
        for offset in cursor..cursor + tiles {
            uncompressed += uncompressed_lengths[offset] as u64;
            compressed += compressed_lengths[offset];
        }
        cursor += tiles;

        descriptors.push(LayerDescriptor {
            name: layer.name.clone(),
            kind: layer.kind,
            rect: layer.rect,
            opacity: layer.opacity,
            mask: layer.mask,
            uncompressed_bytes: uncompressed,
            compressed_bytes: compressed,
        });
    }
    Ok(descriptors)
}

/// The per-layer tile counts and byte counts.
fn layer_accounting(
    levels: &[StreamLevel<'_>],
    has_layers: bool,
    uncompressed_lengths: &[u32],
    compressed_lengths: &[u64],
) -> (Vec<u32>, Vec<u64>, Vec<u64>) {
    if !has_layers {
        let mut tiles = Vec::new();
        let mut uncompressed = Vec::new();
        let mut compressed = Vec::new();
        if let Some(level) = levels.first() {
            let count = level.partition.len();
            tiles.push(count as u32);
            uncompressed.push(
                uncompressed_lengths
                    .iter()
                    .take(count)
                    .map(|&length| length as u64)
                    .sum(),
            );
            compressed.push(compressed_lengths.iter().take(count).sum());
        } else {
            tiles.push(0);
            uncompressed.push(0);
            compressed.push(0);
        }
        return (tiles, uncompressed, compressed);
    }

    let count = levels
        .iter()
        .map(|level| level.layer)
        .max()
        .map(|max| max as usize + 1)
        .unwrap_or(0);
    let mut tiles = vec![0u32; count];
    let mut uncompressed = vec![0u64; count];
    let mut compressed = vec![0u64; count];
    let mut cursor = 0usize;
    for level in levels {
        let index = level.layer as usize;
        let count = level.partition.len();
        tiles[index] += count as u32;
        for offset in cursor..cursor + count {
            uncompressed[index] += uncompressed_lengths[offset] as u64;
            compressed[index] += compressed_lengths[offset];
        }
        cursor += count;
    }
    (tiles, uncompressed, compressed)
}

/// The temporary file's name, in the target's own directory so the rename
/// stays within one volume.
fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".partial-{}", std::process::id()));
    PathBuf::from(name)
}
