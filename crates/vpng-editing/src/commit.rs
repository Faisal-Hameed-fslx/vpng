//! The copy-on-write writer.
//!
//! Saving an edited document writes a **new** file and then replaces the old
//! one atomically. The original file is never modified in place, so a failure
//! at any point — a full disk, a failed checksum, an injected error — leaves
//! the original exactly as it was.
//!
//! # Untouched tiles
//!
//! A tile that the edit did not touch keeps its compressed payload **byte for
//! byte**. The writer copies the original bytes into the new file rather than
//! decoding and re-encoding the tile, so an untouched tile cannot drift: a
//! save that changes one tile leaves every other tile's payload identical.
//!
//! The payload's *offset* can move, because the container lays the payloads
//! out contiguously after the directory and a rewritten tile may be a
//! different size. The bytes themselves do not change.
//!
//! # Atomicity
//!
//! The new file is written to a temporary file in the **same directory** as
//! the target, so the replacement is a rename within one volume. The rename
//! replaces the target in one step: a reader either sees the old file or the
//! new one, never a half-written file.
//!
//! # Failure injection
//!
//! [`FailurePoint`] lets a test make the writer fail on purpose, either part
//! way through the write or immediately before the rename. The point of the
//! exercise is that the original file survives both, and that no partial file
//! is left behind.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vpng_container::{
    crc32c, Directory, DirectoryEntry, Header, FEATURE_PYRAMID, HEADER_SIZE, MINOR_V0_3,
};
use vpng_core::{Error, Limits, PixelFormat, Result};
use vpng_encoder::{select_payload, EncodeOptions};

use crate::source::TileSource;

/// Where an injected failure happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailurePoint {
    /// Fail once this many bytes have been written to the temporary file.
    AfterBytes(u64),
    /// Fail after the temporary file is complete but before it replaces the
    /// target.
    BeforeRename,
}

/// How a save is performed.
#[derive(Debug, Clone)]
pub struct CommitOptions {
    /// Force a codec for the rewritten tiles.
    ///
    /// `None` selects the smallest payload the encoder would choose, using
    /// the encoder's own selection function so a re-encoded tile is exactly
    /// the tile the encoder would have produced.
    pub codec_id: Option<u8>,
    /// Which codecs the per-tile selector may choose from.
    ///
    /// The default is [`vpng_codecs::CodecSet::V0_1`], the same set the
    /// encoder uses by default, so a commit that does not ask for anything
    /// produces the same bytes the encoder would have produced.
    pub codec_set: vpng_codecs::CodecSet,
    /// Make the write fail on purpose.
    pub failure: Option<FailurePoint>,
    /// The suffix of the temporary file.
    pub temp_suffix: String,
}

impl Default for CommitOptions {
    fn default() -> Self {
        CommitOptions {
            codec_id: None,
            codec_set: vpng_codecs::CodecSet::V0_1,
            failure: None,
            temp_suffix: ".vpng-edit.tmp".to_string(),
        }
    }
}

/// What a save did.
#[derive(Debug, Clone)]
pub struct CommitReport {
    /// The path that was written.
    pub path: PathBuf,
    /// The size of the new file.
    pub file_size: u64,
    /// The number of tiles that were re-encoded.
    pub tiles_rewritten: u32,
    /// The number of tiles whose payload was copied verbatim.
    pub tiles_copied: u32,
    /// The bytes copied verbatim from the original file.
    pub copied_payload_bytes: u64,
    /// The bytes of newly encoded payloads.
    pub rewritten_payload_bytes: u64,
    /// Whether the session's baseline moved to the committed file.
    ///
    /// The writer itself does not know: it only writes a file. The editing
    /// engine sets the flag after it has asked the source to re-read its
    /// baseline, and it is `false` when the commit went to a path other
    /// than the source's own file, or when the source cannot re-read its
    /// baseline (the borrowed in-memory source).
    pub baseline_advanced: bool,
    /// The number of derived preview levels the commit dropped.
    ///
    /// A preview is derived from the original's pixels, so an edit that
    /// changes those pixels leaves the preview stale. The editor cannot
    /// rebuild a preview without re-reading the whole original, which its
    /// memory model does not allow, so the commit writes the original level
    /// only and says how many previews it left out.
    pub preview_levels_dropped: u32,
    /// How long the save took.
    pub elapsed: Duration,
}

/// The new payload of one rewritten tile.
struct Rewritten {
    codec_id: u8,
    payload: Vec<u8>,
}

/// Write the edited document to `target`, replacing it atomically.
///
/// `dirty` maps a tile index to the tile's new pixels. Every tile that is
/// not in the map keeps the payload the source stores for it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_document<S: TileSource>(
    source: &mut S,
    dirty: &BTreeMap<usize, Vec<u8>>,
    limits: &Limits,
    format: PixelFormat,
    width: u32,
    height: u32,
    template: &Header,
    target: &Path,
    options: &CommitOptions,
) -> Result<CommitReport> {
    let started = Instant::now();

    // The original level is the only level the editor writes. A file may hold
    // derived preview levels; those are dropped, because an edit invalidates
    // them and the editor cannot rebuild them without re-reading the whole
    // original.
    let base = source.base_level();
    let tile_count = base.tile_count();
    let preview_levels_dropped = (source.level_count() - 1) as u32;

    // Step one: encode every dirty tile. The payloads are held in memory
    // only for the tiles the edit touched, which is bounded by the edited
    // area rather than by the file.
    let mut rewritten: BTreeMap<usize, Rewritten> = BTreeMap::new();
    for (index, pixels) in dirty {
        // The lookup is the bounds check: a dirty tile must exist in the
        // directory.
        let original = source
            .tile_entry(*index)
            .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
        let expected =
            original.width as usize * original.height as usize * format.bytes_per_pixel() as usize;
        if pixels.len() != expected {
            return Err(Error::internal(format!(
                "tile {index} holds {} pixels, expected {expected}",
                pixels.len()
            )));
        }

        let ctx = vpng_codecs::EncodeContext {
            limits: *limits,
            format,
            tile_width: original.width,
            tile_height: original.height,
        };
        let encode_options = match options.codec_id {
            Some(codec_id) => EncodeOptions {
                codec_id,
                codec_set: options.codec_set,
                ..EncodeOptions::default()
            },
            None => EncodeOptions {
                codec_set: options.codec_set,
                ..EncodeOptions::default()
            },
        };
        let (codec_id, payload) = select_payload(pixels, &ctx, &encode_options)?;
        rewritten.insert(*index, Rewritten { codec_id, payload });
    }

    // Step two: lay out the new directory. The offsets depend on the sizes
    // of the rewritten payloads, so they are computed after the payloads are
    // known.
    let directory_size = Directory::total_size(tile_count as u32)?;
    let mut cursor = vpng_core::checked_add(HEADER_SIZE as u64, directory_size as u64)?;

    let mut entries: Vec<DirectoryEntry> = Vec::with_capacity(tile_count);
    let mut copied_payload_bytes = 0u64;
    let mut rewritten_payload_bytes = 0u64;

    for index in 0..tile_count {
        let original = source
            .tile_entry(index)
            .ok_or_else(|| Error::internal(format!("tile {index} has no directory entry")))?;

        let entry = match rewritten.get(&index) {
            Some(new) => {
                rewritten_payload_bytes += new.payload.len() as u64;
                DirectoryEntry {
                    x: original.x,
                    y: original.y,
                    width: original.width,
                    height: original.height,
                    codec_id: new.codec_id,
                    // The metadata describes the payload that was just
                    // written, not the one it replaced. The previous tile's
                    // predictor id and flags describe an encoding that is no
                    // longer in the file, so carrying them over would leave
                    // the directory entry describing the wrong thing.
                    predictor_id: 0,
                    flags: 0,
                    payload_offset: cursor,
                    compressed_length: new.payload.len() as u64,
                    // The uncompressed length is a property of the tile's
                    // geometry, not of its encoding: an edit never changes
                    // the tile's size, so the length is the same one the
                    // source declared.
                    uncompressed_length: original.uncompressed_length,
                    payload_checksum: crc32c(&new.payload),
                }
            }
            None => {
                copied_payload_bytes += original.compressed_length;
                DirectoryEntry {
                    x: original.x,
                    y: original.y,
                    width: original.width,
                    height: original.height,
                    codec_id: original.codec_id,
                    predictor_id: original.predictor_id,
                    flags: original.flags,
                    payload_offset: cursor,
                    compressed_length: original.compressed_length,
                    uncompressed_length: original.uncompressed_length,
                    payload_checksum: original.payload_checksum,
                }
            }
        };
        cursor = vpng_core::checked_add(cursor, entry.compressed_length)?;
        entries.push(entry);
    }

    let directory = Directory::new(entries);
    let directory_bytes = directory.encode()?;

    // The header keeps every field of the original except the size, the
    // directory offset and the tile count. The default codec id is
    // informational — the per-tile entry is authoritative — so it is carried
    // over unchanged. The pyramid flag is cleared, because the committed file
    // holds the original level only.
    //
    // The minor version moves up if a rewritten tile names a codec that only
    // a newer format defines: a v0.1 file edited into a file that uses the
    // dictionary codec becomes a v0.3 file. It never moves down.
    let uses_v0_3_codec = rewritten
        .values()
        .any(|tile| vpng_codecs::is_v0_3_codec(tile.codec_id));
    let minor = if uses_v0_3_codec {
        template.minor.max(MINOR_V0_3)
    } else {
        template.minor
    };
    let header = Header {
        major: template.major,
        minor,
        width,
        height,
        color_model: template.color_model,
        bits: template.bits,
        default_codec: template.default_codec,
        tile_count: tile_count as u32,
        directory_offset: HEADER_SIZE as u64,
        declared_file_size: cursor,
        feature_flags: template.feature_flags & !FEATURE_PYRAMID,
    };
    let header_bytes = header.to_bytes()?;

    // Step three: write the temporary file.
    let temp_path = temporary_path(target, &options.temp_suffix);
    let write_result = write_temp_file(
        source,
        &rewritten,
        tile_count,
        &header_bytes,
        &directory_bytes,
        &temp_path,
        options,
    );

    if let Err(error) = write_result {
        // The original file has not been touched, so all that is left is to
        // remove the partial temporary file.
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }

    // Step four: replace the target in one step.
    if options.failure == Some(FailurePoint::BeforeRename) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(Error::internal(
            "injected failure before the atomic replacement",
        ));
    }

    std::fs::rename(&temp_path, target)?;

    Ok(CommitReport {
        path: target.to_path_buf(),
        file_size: cursor,
        tiles_rewritten: rewritten.len() as u32,
        tiles_copied: (tile_count - rewritten.len()) as u32,
        copied_payload_bytes,
        rewritten_payload_bytes,
        baseline_advanced: false,
        preview_levels_dropped,
        elapsed: started.elapsed(),
    })
}

/// Write the temporary file.
fn write_temp_file<S: TileSource>(
    source: &mut S,
    rewritten: &BTreeMap<usize, Rewritten>,
    tile_count: usize,
    header_bytes: &[u8],
    directory_bytes: &[u8],
    temp_path: &Path,
    options: &CommitOptions,
) -> Result<()> {
    let mut file = File::create(temp_path)?;
    let mut written = 0u64;

    let write = |file: &mut File, bytes: &[u8], written: &mut u64| -> Result<()> {
        if let Some(FailurePoint::AfterBytes(limit)) = options.failure {
            if *written >= limit {
                return Err(Error::internal(format!(
                    "injected failure after {written} bytes"
                )));
            }
            if *written + bytes.len() as u64 > limit {
                let allowed = (limit - *written) as usize;
                file.write_all(&bytes[..allowed])?;
                *written = limit;
                return Err(Error::internal(format!(
                    "injected failure after {written} bytes"
                )));
            }
        }
        file.write_all(bytes)?;
        *written += bytes.len() as u64;
        Ok(())
    };

    write(&mut file, header_bytes, &mut written)?;
    write(&mut file, directory_bytes, &mut written)?;

    for index in 0..tile_count {
        match rewritten.get(&index) {
            Some(new) => {
                write(&mut file, &new.payload, &mut written)?;
            }
            None => {
                // Copy the original payload byte for byte.
                let payload = source.tile_payload(index)?;
                write(&mut file, &payload, &mut written)?;
            }
        }
    }

    file.flush()?;
    file.sync_all()?;
    Ok(())
}

/// The path of the temporary file used while saving `target`.
fn temporary_path(target: &Path, suffix: &str) -> PathBuf {
    let directory = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".to_string());
    // The process id keeps two editors working on different files from
    // colliding; the counter keeps two saves in the same process apart.
    let unique = next_temp_id();
    directory.join(format!(".{name}.{unique}{suffix}"))
}

/// A process-wide counter for temporary file names.
fn next_temp_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    (std::process::id() as u64) << 24 | (count & 0x00FF_FFFF)
}
