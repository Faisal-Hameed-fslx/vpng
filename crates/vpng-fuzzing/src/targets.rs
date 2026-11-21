//! The fuzz targets.
//!
//! Each target takes an arbitrary byte string and checks the invariants that
//! must hold for *every* input, including malformed ones. A target never
//! panics on hostile input; a panic is a bug in the library, and the
//! mutation harness reports it as a finding.

use vpng_codecs::experimental;
use vpng_codecs::{get_codec, DecodeContext, EncodeContext};
use vpng_container::{Directory, Header};
use vpng_core::{Dimensions, Limits, PixelFormat, Rect};
use vpng_decoder::Document;
use vpng_editing::{CommitOptions, Editor};
use vpng_encoder::{encode, EncodeOptions};
use vpng_testkit::Rng;
use vpng_tiling::TileGrid;

/// The result of running one target over one input.
///
/// `Err` carries the invariant that was broken. The harness treats it as a
/// finding and keeps the offending input.
pub type TargetResult = Result<(), String>;

/// The limits every target runs under.
///
/// The limits are deliberately tight: a fuzz target must not be able to ask
/// for a gigabyte of memory, or the harness would run out of memory before
/// it found anything.
pub fn fuzz_limits() -> Limits {
    Limits {
        max_width: 1024,
        max_height: 1024,
        max_pixels: 1024 * 1024,
        max_tile_count: 1024,
        max_tile_payload_bytes: 1 << 20,
        max_decompressed_bytes: 1 << 22,
        max_file_bytes: 1 << 24,
        max_expansion_ratio: 4096,
    }
}

/// The name of a target.
pub type Target = &'static str;

/// Every target, in a fixed order.
pub const ALL_TARGETS: &[Target] = &[
    "header",
    "directory",
    "container",
    "tile-payload",
    "experimental-codec",
    "region",
    "editing",
];

/// Run one target over one input.
pub fn run_target(name: Target, data: &[u8]) -> TargetResult {
    match name {
        "header" => header_target(data),
        "directory" => directory_target(data),
        "container" => container_target(data),
        "tile-payload" => tile_payload_target(data),
        "experimental-codec" => experimental_codec_target(data),
        "region" => region_target(data),
        "editing" => editing_target(data),
        other => Err(format!("unknown target {other}")),
    }
}

/// Parse arbitrary bytes as a container header.
///
/// An accepted header must re-encode to exactly the bytes it was parsed
/// from: the header is a fixed layout, and a parser that accepts a byte it
/// cannot reproduce would let two implementations disagree about a file.
pub fn header_target(data: &[u8]) -> TargetResult {
    let header = match Header::parse(data) {
        Ok(header) => header,
        Err(_) => return Ok(()),
    };
    let encoded = header
        .to_bytes()
        .map_err(|e| format!("accepted header failed to re-encode: {e}"))?;
    // The parser reads the first 64 bytes and ignores whatever follows, so
    // the round-trip is compared against the header's own bytes only.
    if encoded.as_slice() != &data[..encoded.len()] {
        return Err("an accepted header did not re-encode to its own bytes".to_string());
    }
    // The accessors must not panic on an accepted header.
    let _ = header.format();
    let _ = header.dimensions();
    Ok(())
}

/// Parse arbitrary bytes as a tile directory.
///
/// The directory length must match the entry count exactly, so the target
/// only tries lengths that can hold whole entries. An accepted directory must
/// re-encode to exactly the bytes it was parsed from, CRC trailer included.
pub fn directory_target(data: &[u8]) -> TargetResult {
    const ENTRY: usize = 40;
    const TRAILER: usize = 4;
    if data.len() < TRAILER || (data.len() - TRAILER) % ENTRY != 0 {
        return Ok(());
    }
    let count = ((data.len() - TRAILER) / ENTRY) as u32;
    let directory = match Directory::decode(data, count) {
        Ok(directory) => directory,
        Err(_) => return Ok(()),
    };
    let encoded = directory
        .encode()
        .map_err(|e| format!("accepted directory failed to re-encode: {e}"))?;
    if encoded != data {
        return Err("an accepted directory did not re-encode to its own bytes".to_string());
    }
    // The range validator must not panic on an accepted directory.
    for size in [0u64, 1, 1 << 20, u64::MAX] {
        let _ = directory.validate_payload_ranges(size);
    }
    for bpp in [1u64, 2, 3, 4, 6, 8] {
        let _ = directory.validate_tile_sizes(bpp);
    }
    Ok(())
}

/// Parse arbitrary bytes as a whole VPNG file.
///
/// When the file is accepted, the decoder's own promises must hold. The
/// decoder parses in O(1) in the tile count and deliberately does **not**
/// read payload bytes while parsing, so a file with a damaged payload is
/// accepted by the parser and rejected later, by the per-tile checksum. The
/// target therefore checks the contract the decoder actually makes:
///
/// * an integrity failure must be reported as a corrupt file, never as an
///   internal error;
/// * when the full decode succeeds, it must produce exactly the declared
///   dimensions and exactly the declared number of bytes;
/// * when the full decode succeeds, a region decode must equal the
///   corresponding crop of the full decode.
pub fn container_target(data: &[u8]) -> TargetResult {
    let limits = fuzz_limits();
    let document = match Document::parse(data, &limits) {
        Ok(document) => document,
        Err(_) => return Ok(()),
    };

    match document.check_integrity() {
        Ok(_) => {}
        Err(error) => {
            if error.kind() != vpng_core::ErrorKind::CorruptFile {
                return Err(format!(
                    "an integrity check failed with an unexpected kind: {error}"
                ));
            }
        }
    }

    let full = match document.decode_all(&limits) {
        Ok(full) => full,
        Err(error) => {
            // A file the parser accepted may still be corrupt in its
            // payloads. That is a data error, not a bug.
            if error.kind() == vpng_core::ErrorKind::Internal {
                return Err(format!(
                    "the decoder failed with an internal error: {error}"
                ));
            }
            return Ok(());
        }
    };
    if full.width() != document.width() || full.height() != document.height() {
        return Err("the decoded image does not have the declared dimensions".to_string());
    }
    let expected_bytes = document
        .format()
        .frame_bytes(document.width(), document.height())
        .map_err(|e| format!("the declared size could not be computed: {e}"))?;
    if full.len() as u64 != expected_bytes {
        return Err("the decoded image has the wrong byte count".to_string());
    }

    // A region decode must equal the crop of the full decode.
    let rect = pick_rect(data, document.width(), document.height());
    if let Some(rect) = rect {
        let region = document
            .decode_region(rect, &limits)
            .map_err(|e| format!("a region of an accepted file failed to decode: {e}"))?;
        let bpp = document.format().bytes_per_pixel() as usize;
        let full_row = document.width() as usize * bpp;
        let region_row = rect.width as usize * bpp;
        for y in 0..rect.height as usize {
            let full_start = (rect.y as usize + y) * full_row + rect.x as usize * bpp;
            let region_start = y * region_row;
            if full.data()[full_start..full_start + region_row]
                != region.data()[region_start..region_start + region_row]
            {
                return Err("a region decode did not match the crop of the full decode".to_string());
            }
        }
    }
    Ok(())
}

/// Decode arbitrary bytes with every stable codec.
///
/// A decoder that reports success must produce exactly the number of bytes
/// the caller asked for, and it must not panic on any input.
pub fn tile_payload_target(data: &[u8]) -> TargetResult {
    let limits = fuzz_limits();
    let format = PixelFormat::rgba8();
    for id in vpng_codecs::IMPLEMENTED_CODEC_IDS {
        let codec = match get_codec(*id) {
            Ok(codec) => codec,
            Err(_) => continue,
        };
        let ctx = DecodeContext { limits, format };
        // Ask for a few different output sizes: a decoder must not trust the
        // caller's number and must not overflow when the sizes disagree.
        for expected in [0usize, 1, 16, 4096] {
            let mut out = Vec::new();
            if let Ok(()) = codec.decode(data, expected, &ctx, &mut out) {
                if out.len() != expected {
                    return Err(format!(
                        "codec {} reported success but produced {} bytes instead of {expected}",
                        codec.name(),
                        out.len()
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Decode arbitrary bytes with every experimental codec.
///
/// The experimental codecs are research-only, but their decoders are still
/// reachable by the research harness, so they must survive hostile input the
/// same way the stable ones do.
pub fn experimental_codec_target(data: &[u8]) -> TargetResult {
    let limits = fuzz_limits();
    let format = PixelFormat::rgba8();
    for (name, codec) in experimental::all() {
        let ctx = DecodeContext { limits, format };
        for expected in [0usize, 1, 16, 4096] {
            let mut out = Vec::new();
            if let Ok(()) = codec.decode(data, expected, &ctx, &mut out) {
                if out.len() != expected {
                    return Err(format!(
                        "experimental codec {name} reported success but produced {} bytes instead of {expected}",
                        out.len()
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Exercise the region and grid arithmetic with hostile numbers.
///
/// The tile grid and the rectangle helpers are the part of the library that
/// untrusted coordinates reach first. Every arithmetic path must either
/// produce a correct answer or report an error; it must never overflow, and
/// it must never return a tile that does not intersect the requested
/// rectangle.
pub fn region_target(data: &[u8]) -> TargetResult {
    let mut rng = Rng::new(seed_from(data, 0x5245_4749_4F4E));
    // The bounds keep the grid small enough to walk tile by tile: at most
    // 32 columns and 32 rows, so at most 1024 tiles.
    let width = 1 + rng.next_u64() as u32 % 512;
    let height = 1 + rng.next_u64() as u32 % 512;
    let tile_width = 16 + rng.next_u64() as u32 % 256;
    let tile_height = 16 + rng.next_u64() as u32 % 256;

    let grid = match TileGrid::new(tile_width, tile_height, width, height) {
        Ok(grid) => grid,
        Err(_) => return Ok(()),
    };

    // Every tile the grid claims to have must have a geometry, and the
    // geometry must agree with the column/row it came from.
    let count = grid
        .tile_count()
        .map_err(|e| format!("tile count failed: {e}"))? as usize;
    for index in 0..count {
        let geometry = grid
            .geometry_for_index(index)
            .map_err(|e| format!("tile {index} has no geometry: {e}"))?;
        let by_position = grid
            .geometry(geometry.col, geometry.row)
            .map_err(|e| format!("tile {index} has no geometry by position: {e}"))?;
        if geometry != by_position {
            return Err(format!("tile {index} has two different geometries"));
        }
        if geometry.x + geometry.width > width || geometry.y + geometry.height > height {
            return Err(format!("tile {index} lies outside the image"));
        }
    }

    // A rectangle built from hostile numbers must either be rejected or
    // produce tiles that all intersect it.
    for _ in 0..64 {
        let x = rng.next_u64() as u32;
        let y = rng.next_u64() as u32;
        let w = rng.next_u64() as u32;
        let h = rng.next_u64() as u32;
        let rect = match Rect::new(x, y, w, h) {
            Ok(rect) => rect,
            Err(_) => continue,
        };
        if !rect.is_within(Dimensions { width, height }) {
            // A rectangle outside the image is legal to construct; it simply
            // has no tiles. It must not panic when it is asked for.
            let _ = grid.geometries_for_rect(rect);
            continue;
        }
        let geometries = grid
            .geometries_for_rect(rect)
            .map_err(|e| format!("a valid rectangle produced an error: {e}"))?;
        for geometry in &geometries {
            let tx0 = geometry.x.max(rect.x);
            let ty0 = geometry.y.max(rect.y);
            let tx1 = (geometry.x + geometry.width).min(rect.right().unwrap());
            let ty1 = (geometry.y + geometry.height).min(rect.bottom().unwrap());
            if tx0 >= tx1 || ty0 >= ty1 {
                return Err("a tile was returned that does not intersect the rectangle".to_string());
            }
        }
    }
    Ok(())
}

/// Exercise the editing engine with hostile region writes.
///
/// The editing engine reads a rectangle's coordinates and lengths from
/// untrusted input in a GUI session. A write must either be rejected or land
/// exactly where it was asked to land, and the dirty-tile set must contain
/// exactly the tiles the rectangle intersects.
pub fn editing_target(data: &[u8]) -> TargetResult {
    let limits = fuzz_limits();
    // The document is built once and reused: encoding an image on every
    // input would dominate the run time and would not test anything new.
    static DOCUMENT: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    let bytes = DOCUMENT.get_or_init(|| {
        let image = vpng_testkit::gradient(128, 96, PixelFormat::rgba8(), 7);
        encode(&image, &EncodeOptions::default().with_tile_size(32, 32))
            .map(|encoded| encoded.bytes)
            .unwrap_or_default()
    });
    if bytes.is_empty() {
        return Err("the test image could not be encoded".to_string());
    }

    let mut rng = Rng::new(seed_from(data, 0x4544_4954));
    let x = rng.next_u64() as u32;
    let y = rng.next_u64() as u32;
    let w = rng.next_u64() as u32;
    let h = rng.next_u64() as u32;

    let mut editor = match Editor::parse(bytes, &limits) {
        Ok(editor) => editor,
        Err(e) => return Err(format!("a valid file could not be opened for editing: {e}")),
    };

    let rect = match Rect::new(x, y, w, h) {
        Ok(rect) => rect,
        Err(_) => return Ok(()),
    };
    if !rect.is_within(Dimensions {
        width: 128,
        height: 96,
    }) {
        // An out-of-bounds rectangle must be rejected, not applied.
        let payload = vec![0u8; 16];
        if editor.write_region(rect, &payload).is_ok() {
            return Err("a rectangle outside the image was accepted".to_string());
        }
        return Ok(());
    }

    let expected_len = rect.width as usize * rect.height as usize * 4;
    let mut payload = vec![0u8; expected_len];
    if !payload.is_empty() {
        let take = payload.len().min(data.len());
        let head: Vec<u8> = data[..take].to_vec();
        payload[..take].copy_from_slice(&head);
    }

    editor
        .write_region(rect, &payload)
        .map_err(|e| format!("a valid region write failed: {e}"))?;

    // The write must read back exactly.
    let region = editor
        .read_region(rect)
        .map_err(|e| format!("a region read after a write failed: {e}"))?;
    if region.data() != &payload[..] {
        return Err("a region read back different bytes than were written".to_string());
    }

    // The dirty tiles must be exactly the tiles the rectangle touches.
    let expected_tiles: Vec<usize> = editor
        .partition()
        .tiles_for(rect)
        .map_err(|e| format!("the partition could not map the rectangle: {e}"))?;
    let mut actual = editor.dirty_tiles();
    actual.sort_unstable();
    let mut expected_tiles = expected_tiles;
    expected_tiles.sort_unstable();
    if actual != expected_tiles {
        return Err(
            "the dirty-tile set does not match the tiles the rectangle touches".to_string(),
        );
    }

    // An abort must put the document back exactly as it was.
    editor
        .begin()
        .map_err(|e| format!("a transaction could not be opened: {e}"))?;
    editor
        .write_region(rect, &payload)
        .map_err(|e| format!("a write inside a transaction failed: {e}"))?;
    editor
        .abort()
        .map_err(|e| format!("a transaction could not be aborted: {e}"))?;
    if editor.is_dirty() {
        return Err("an abort left the document dirty".to_string());
    }
    let _ = CommitOptions::default();
    Ok(())
}

/// A seed derived from the input bytes, so a failing input always fails the
/// same way.
fn seed_from(data: &[u8], salt: u64) -> u64 {
    let mut hash = salt ^ 0x9E37_79B9_7F4A_7C15;
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0100_0000_01B3);
    }
    if hash == 0 {
        0x9E37_79B9_7F4A_7C15
    } else {
        hash
    }
}

/// A rectangle inside an image, derived from the input bytes.
fn pick_rect(data: &[u8], width: u32, height: u32) -> Option<Rect> {
    if width == 0 || height == 0 || data.len() < 4 {
        return None;
    }
    let mut rng = Rng::new(seed_from(data, 0x5245_4354));
    let x = rng.next_u64() as u32 % width;
    let y = rng.next_u64() as u32 % height;
    let w = 1 + rng.next_u64() as u32 % (width - x).max(1);
    let h = 1 + rng.next_u64() as u32 % (height - y).max(1);
    Rect::new(x, y, w, h).ok()
}

/// The codec ids the research harness reports, for the mutation driver's
/// report.
pub fn experimental_codec_ids() -> Vec<u8> {
    experimental::all()
        .iter()
        .map(|(_, codec)| codec.codec_id())
        .collect()
}

/// The stable codec ids.
pub fn stable_codec_ids() -> Vec<u8> {
    vpng_codecs::IMPLEMENTED_CODEC_IDS.to_vec()
}

/// An encoder context for a small tile, used by the mutation driver's seed
/// builder.
pub fn small_encode_context(format: PixelFormat) -> EncodeContext {
    EncodeContext {
        limits: fuzz_limits(),
        format,
        tile_width: 16,
        tile_height: 16,
    }
}
