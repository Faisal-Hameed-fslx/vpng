//! `vpng-research` — the measurement harness for the P3.5 compression study.
//!
//! The harness reads a **canonical raw pixel file** — the exact bytes the
//! corpus runner hands to the CLI — tiles it with the same grid the encoder
//! uses, and measures every stable and experimental codec on the *same*
//! pixels and the *same* tile layout.
//!
//! It writes a JSON document. The corpus runner aggregates that document
//! into the per-category, per-resolution, aggregate and worst-case reports
//! that the milestone asks for.
//!
//! # What is measured
//!
//! For every tile and every candidate codec:
//!
//! * the **complete payload size**, framing bytes included;
//! * the bits per pixel the payload costs;
//! * the encode and decode wall-clock time;
//! * whether the decoded bytes are **exactly** the original bytes;
//! * the framing bytes the codec's own header costs;
//! * the peak heap bytes the operation needed, measured with a tracking
//!   global allocator.
//!
//! The memory figure is *heap bytes tracked by this harness*, not the
//! process's resident set size. The report says so explicitly.
//!
//! # Why a separate binary
//!
//! The canonical pixels and the PNG/WebP baselines come from the existing
//! Python corpus runner, so the research numbers are directly comparable
//! with the numbers already in `benchmarks/results/`. Nothing here changes
//! the file format, and no experimental codec id is ever written to a file.

#![allow(clippy::too_many_arguments)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::time::Instant;

use vpng_codecs::experimental;
use vpng_codecs::{DecodeContext, EncodeContext, TileCodec};
use vpng_core::{Error, Limits, PixelFormat, Result};
use vpng_tiling::TileGrid;

// ---------------------------------------------------------------------------
// Tracking allocator
// ---------------------------------------------------------------------------

/// A global allocator that tracks the bytes currently allocated and the
/// highest total it has ever reached.
///
/// The unsafe code is confined to this one place: forwarding to the system
/// allocator and updating two counters.
struct Tracking;

static CURRENT_BYTES: AtomicIsize = AtomicIsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Add (or subtract) bytes from the current total, never going below zero.
///
/// The counter is signed so a subtraction cannot wrap around; the clamp to
/// zero keeps a mismatch between the allocation and deallocation sizes from
/// producing a nonsensical total.
fn add_bytes(delta: isize) -> usize {
    let mut current = CURRENT_BYTES.load(Ordering::Relaxed);
    loop {
        let next = (current + delta).max(0) as usize;
        match CURRENT_BYTES.compare_exchange_weak(
            current,
            next as isize,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return next,
            Err(actual) => current = actual,
        }
    }
}

/// Record a new high-water mark for the heap total.
fn update_peak(current: usize) {
    let mut peak = PEAK_BYTES.load(Ordering::Relaxed);
    while current > peak {
        match PEAK_BYTES.compare_exchange_weak(peak, current, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => break,
            Err(actual) => peak = actual,
        }
    }
}

unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let current = add_bytes(layout.size() as isize);
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            update_peak(current);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        add_bytes(-(layout.size() as isize));
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            let current = add_bytes(new_size as isize - layout.size() as isize);
            update_peak(current);
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: Tracking = Tracking;

/// The bytes currently allocated on the heap.
fn current_bytes() -> usize {
    CURRENT_BYTES.load(Ordering::Relaxed).max(0) as usize
}

/// The highest heap total reached since the last reset.
fn peak_bytes() -> usize {
    PEAK_BYTES.load(Ordering::Relaxed)
}

/// Reset the peak tracker.
fn reset_peak() {
    PEAK_BYTES.store(current_bytes(), Ordering::Relaxed);
}

/// The number of allocations made since the process started.
fn allocation_count() -> usize {
    ALLOC_COUNT.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

/// The command line the harness was given.
struct Args {
    input: String,
    width: u32,
    height: u32,
    format: PixelFormat,
    tile_size: u32,
    output: Option<String>,
}

fn parse_format(text: &str) -> Result<PixelFormat> {
    let lowered = text.to_ascii_lowercase();
    Ok(match lowered.as_str() {
        "gray8" | "gray" | "g8" | "l8" => PixelFormat::gray8(),
        "gray16" | "g16" | "l16" => PixelFormat::gray16(),
        "graya8" | "gray-alpha8" | "ga8" | "la8" => PixelFormat::gray_alpha8(),
        "graya16" | "gray-alpha16" | "ga16" | "la16" => PixelFormat::gray_alpha16(),
        "rgb8" | "rgb" => PixelFormat::rgb8(),
        "rgb16" => PixelFormat::rgb16(),
        "rgba8" | "rgba" => PixelFormat::rgba8(),
        "rgba16" => PixelFormat::rgba16(),
        other => {
            return Err(Error::invalid(format!(
                "unknown pixel format `{other}` (try rgba8, rgb16, gray8, graya16, ...)"
            )))
        }
    })
}

fn parse_args() -> Result<Args> {
    let mut input = None;
    let mut width = None;
    let mut height = None;
    let mut format = None;
    let mut tile_size = 256u32;
    let mut output = None;

    let mut argv = std::env::args().skip(1);
    while let Some(flag) = argv.next() {
        let mut value = || {
            argv.next()
                .ok_or_else(|| Error::invalid(format!("`{flag}` expects a value")))
        };
        match flag.as_str() {
            "--input" => input = Some(value()?),
            "--width" => {
                width = Some(
                    value()?
                        .parse()
                        .map_err(|_| Error::invalid("--width expects an integer"))?,
                )
            }
            "--height" => {
                height = Some(
                    value()?
                        .parse()
                        .map_err(|_| Error::invalid("--height expects an integer"))?,
                )
            }
            "--format" => format = Some(parse_format(&value()?)?),
            "--tile-size" => {
                tile_size = value()?
                    .parse()
                    .map_err(|_| Error::invalid("--tile-size expects an integer"))?
            }
            "--output" => output = Some(value()?),
            other => {
                return Err(Error::invalid(format!(
                    "unknown argument `{other}` (try --input, --width, --height, --format)"
                )))
            }
        }
    }

    Ok(Args {
        input: input.ok_or_else(|| Error::invalid("--input is required"))?,
        width: width.ok_or_else(|| Error::invalid("--width is required"))?,
        height: height.ok_or_else(|| Error::invalid("--height is required"))?,
        format: format.ok_or_else(|| Error::invalid("--format is required"))?,
        tile_size,
        output,
    })
}

// ---------------------------------------------------------------------------
// Minimal JSON writer
// ---------------------------------------------------------------------------

/// Escape a string for a JSON document.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Format a floating point number for JSON, or `null` when it is not finite.
fn number(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.6}")
    } else {
        "null".to_string()
    }
}

// ---------------------------------------------------------------------------
// Framing and model state
// ---------------------------------------------------------------------------

/// The size of the VPNG container header.
pub const HEADER_SIZE: u64 = 64;

/// The size of one directory entry.
pub const DIRECTORY_ENTRY_SIZE: u64 = 40;

/// The size of the directory's CRC trailer.
pub const DIRECTORY_CRC_SIZE: u64 = 4;

/// The container framing a file of `tile_count` tiles carries, on top of the
/// tile payloads.
///
/// The header, the directory and the directory's CRC trailer are not image
/// data, but they *are* bytes in the file. Comparing a payload-only total
/// with a whole PNG file would understate VPNG by exactly this much, so the
/// harness reports both numbers.
pub fn container_framing_bytes(tile_count: u64) -> u64 {
    HEADER_SIZE + tile_count * (DIRECTORY_ENTRY_SIZE + DIRECTORY_CRC_SIZE)
}

/// The number of bytes a LEB128 value occupies.
fn leb_len(mut value: u64) -> usize {
    let mut len = 1;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

/// The number of probability contexts the stable predictor uses.
fn predictor_contexts(format: PixelFormat) -> usize {
    let bits = format.bytes_per_channel() as usize * 8;
    bits * 2 + 1
}

/// A stable, machine-friendly name for a pixel format.
fn format_name(format: PixelFormat) -> String {
    format!("{}-{}bit", format.color_model.name(), format.bits.bits())
}

/// The framing bytes a candidate's payload carries outside its compressed
/// data.
///
/// These are the bytes that do not encode image data: headers, a predictor
/// plan, a palette table, and so on. They are part of the payload, so they
/// count towards the size comparison — this function only reports how much
/// of the payload they are.
fn framing_bytes(
    name: &str,
    format: PixelFormat,
    width: usize,
    height: usize,
    total_samples: usize,
) -> usize {
    let bpp = format.bytes_per_pixel() as usize;
    match name {
        "raw" | "rle" => 0,
        "palette" => {
            // Two header bytes plus the palette table. The table's size is
            // not known before the tile is encoded, so the caller overrides
            // this value from the payload it measured.
            2 + bpp
        }
        "predictor" | "carc-a" => {
            1 + leb_len(width as u64) + height + leb_len(total_samples as u64)
        }
        "carc-b" | "carc-c" | "carc-d" | "context-entropy" => {
            3 + leb_len(width as u64) + height + leb_len(total_samples as u64)
        }
        "lz77" => {
            // The LZ77 header plus its declared output length.
            9 + leb_len((total_samples * bpp) as u64)
        }
        "pred-lz77" | "predictor+dictionary" => {
            2 + (1 + leb_len(width as u64) + height) + (9 + leb_len((total_samples * bpp) as u64))
        }
        "lz77-entropy" => 2 + (9 + leb_len((total_samples * bpp) as u64)),
        "pred-lz77-entropy" => {
            2 + (1 + leb_len(width as u64) + height) + (9 + leb_len((total_samples * bpp) as u64))
        }
        _ => 0,
    }
}

/// The bytes of in-memory model state a candidate keeps while it works.
///
/// This is the probability table or the dictionary a codec holds; it is not
/// part of the payload, but it is part of the codec's working set.
fn model_state_bytes(name: &str, format: PixelFormat) -> usize {
    let contexts = predictor_contexts(format);
    match name {
        "raw" | "rle" => 0,
        "palette" => 256 * format.bytes_per_pixel() as usize,
        "predictor" | "carc-a" => contexts * 4,
        "carc-b" | "carc-c" | "carc-d" | "context-entropy" => 256 * contexts * 4,
        "lz77" | "pred-lz77" | "predictor+dictionary" | "lz77-entropy" | "pred-lz77-entropy" => {
            // The hash table and the match chain of the default parameters.
            (1usize << 14) * 4 + (1usize << 15) * 4
        }
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Measurement
// ---------------------------------------------------------------------------

/// One codec's result on one tile.
struct TileMeasurement {
    name: &'static str,
    codec_id: u8,
    status: &'static str,
    payload_bytes: usize,
    bpp: f64,
    ratio: f64,
    encode_ms: f64,
    decode_ms: f64,
    exact: bool,
    framing_bytes: usize,
    model_state_bytes: usize,
    peak_encode_alloc_bytes: usize,
    peak_decode_alloc_bytes: usize,
    error: Option<String>,
}

/// Measure one codec on one tile.
fn measure_codec(
    codec: &dyn TileCodec,
    name: &'static str,
    tile: &[u8],
    encode_ctx: &EncodeContext,
    decode_ctx: &DecodeContext,
    width: usize,
    height: usize,
    total_samples: usize,
    format: PixelFormat,
) -> TileMeasurement {
    let canonical = tile.len() as f64;

    let mut payload = Vec::new();
    reset_peak();
    let baseline = current_bytes();
    let started = Instant::now();
    let encoded = codec.encode(tile, encode_ctx, &mut payload);
    let encode_ms = started.elapsed().as_secs_f64() * 1000.0;
    let peak_encode = peak_bytes().saturating_sub(baseline);

    let encoded = match encoded {
        Ok(()) => Ok(()),
        Err(error) => Err(error),
    };

    if let Err(error) = encoded {
        let unsupported = error.kind() == vpng_core::ErrorKind::UnsupportedFeature;
        return TileMeasurement {
            name,
            codec_id: codec.codec_id(),
            status: if unsupported { "unsupported" } else { "error" },
            payload_bytes: 0,
            bpp: 0.0,
            ratio: 0.0,
            encode_ms,
            decode_ms: 0.0,
            exact: false,
            framing_bytes: 0,
            model_state_bytes: model_state_bytes(name, format),
            peak_encode_alloc_bytes: peak_encode,
            peak_decode_alloc_bytes: 0,
            error: Some(error.to_string()),
        };
    }

    let mut decoded = Vec::new();
    reset_peak();
    let baseline = current_bytes();
    let started = Instant::now();
    let result = codec.decode(&payload, tile.len(), decode_ctx, &mut decoded);
    let decode_ms = started.elapsed().as_secs_f64() * 1000.0;
    let peak_decode = peak_bytes().saturating_sub(baseline);

    let mut framing = framing_bytes(name, format, width, height, total_samples);
    if name == "palette" && payload.len() >= 2 {
        // The palette table's real size is only known from the payload.
        let entries = usize::from(payload[1]) + 1;
        framing = 2 + entries * format.bytes_per_pixel() as usize;
    }

    match result {
        Ok(()) => {
            let exact = decoded == tile;
            let payload_bytes = payload.len();
            TileMeasurement {
                name,
                codec_id: codec.codec_id(),
                status: if exact { "ok" } else { "inexact" },
                payload_bytes,
                bpp: payload_bytes as f64 * 8.0 / canonical,
                ratio: payload_bytes as f64 / canonical,
                encode_ms,
                decode_ms,
                exact,
                framing_bytes: framing,
                model_state_bytes: model_state_bytes(name, format),
                peak_encode_alloc_bytes: peak_encode,
                peak_decode_alloc_bytes: peak_decode,
                error: None,
            }
        }
        Err(error) => TileMeasurement {
            name,
            codec_id: codec.codec_id(),
            status: "error",
            payload_bytes: payload.len(),
            bpp: 0.0,
            ratio: 0.0,
            encode_ms,
            decode_ms,
            exact: false,
            framing_bytes: framing,
            model_state_bytes: model_state_bytes(name, format),
            peak_encode_alloc_bytes: peak_encode,
            peak_decode_alloc_bytes: peak_decode,
            error: Some(error.to_string()),
        },
    }
}

/// Read one tile's bytes straight out of the canonical raw file.
///
/// The harness never holds the whole image: it reads one row segment per
/// tile row, so its memory is bounded by the tile size.
fn read_tile(
    file: &mut File,
    image_width: u32,
    geometry: &vpng_tiling::TileGeometry,
    format: PixelFormat,
) -> Result<Vec<u8>> {
    let bpp = format.bytes_per_pixel() as usize;
    let row_bytes = geometry.width as usize * bpp;
    let mut tile = Vec::with_capacity(row_bytes * geometry.height as usize);
    let mut buffer = vec![0u8; row_bytes];
    for row in 0..geometry.height {
        let offset = (u64::from(geometry.y + row) * u64::from(image_width) + u64::from(geometry.x))
            * bpp as u64;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut buffer)?;
        tile.extend_from_slice(&buffer);
    }
    Ok(tile)
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

/// The aggregate of one candidate over a whole image.
struct Aggregate {
    name: &'static str,
    payload_bytes: u64,
    canonical_bytes: u64,
    tiles_applicable: u64,
    tiles_failed: u64,
    encode_ms: f64,
    decode_ms: f64,
    framing_bytes: u64,
    model_state_bytes: usize,
    peak_encode_alloc_bytes: usize,
    peak_decode_alloc_bytes: usize,
    exact: bool,
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(error) => {
            eprintln!("vpng-research: {error}");
            std::process::exit(error.exit_code());
        }
    }
}

fn run() -> Result<()> {
    let args = parse_args()?;
    let limits = Limits::default();
    let format = args.format;
    let bpp = format.bytes_per_pixel() as usize;

    let grid = TileGrid::new(args.tile_size, args.tile_size, args.width, args.height)?;
    let tile_count = grid.tile_count()?;
    let canonical_bytes = u64::from(args.width) * u64::from(args.height) * bpp as u64;

    let mut file = File::open(&args.input)?;
    let file_len = file.metadata()?.len();
    if file_len != canonical_bytes {
        return Err(Error::invalid(format!(
            "`{}` holds {file_len} bytes but {}x{} {} needs {canonical_bytes}",
            args.input,
            args.width,
            args.height,
            format_name(format)
        )));
    }

    let codecs = experimental::candidate_codecs();
    let decode_ctx = DecodeContext { limits, format };

    // Per-candidate accumulators.
    let mut aggregates: Vec<Aggregate> = codecs
        .iter()
        .map(|(name, _)| Aggregate {
            name,
            payload_bytes: 0,
            canonical_bytes: 0,
            tiles_applicable: 0,
            tiles_failed: 0,
            encode_ms: 0.0,
            decode_ms: 0.0,
            framing_bytes: 0,
            model_state_bytes: 0,
            peak_encode_alloc_bytes: 0,
            peak_decode_alloc_bytes: 0,
            exact: true,
        })
        .collect();

    // The JSON document, built by hand.
    let mut tiles_json = String::new();
    let mut first_tile = true;
    let mut selection_counts: Vec<(&'static str, u64)> =
        codecs.iter().map(|(name, _)| (*name, 0)).collect();
    let mut selection_payload_bytes = 0u64;
    let mut selection_canonical_bytes = 0u64;

    for index in 0..tile_count as usize {
        let geometry =
            grid.geometry(index as u32 % grid.columns(), index as u32 / grid.columns())?;
        let tile = read_tile(&mut file, args.width, &geometry, format)?;
        let width = geometry.width as usize;
        let height = geometry.height as usize;
        let total_samples = width * height * format.channels() as usize;

        let encode_ctx = EncodeContext {
            limits,
            format,
            tile_width: geometry.width as u16,
            tile_height: geometry.height as u16,
        };

        let mut candidates: Vec<(&'static str, usize)> = Vec::new();
        let mut results_json = String::new();
        let mut first_result = true;

        for (slot, (name, codec)) in codecs.iter().enumerate() {
            let measurement = measure_codec(
                *codec,
                name,
                &tile,
                &encode_ctx,
                &decode_ctx,
                width,
                height,
                total_samples,
                format,
            );

            let aggregate = &mut aggregates[slot];
            aggregate.encode_ms += measurement.encode_ms;
            aggregate.decode_ms += measurement.decode_ms;
            aggregate.model_state_bytes = measurement.model_state_bytes;
            aggregate.peak_encode_alloc_bytes = aggregate
                .peak_encode_alloc_bytes
                .max(measurement.peak_encode_alloc_bytes);
            aggregate.peak_decode_alloc_bytes = aggregate
                .peak_decode_alloc_bytes
                .max(measurement.peak_decode_alloc_bytes);
            match measurement.status {
                "ok" => {
                    aggregate.payload_bytes += measurement.payload_bytes as u64;
                    aggregate.canonical_bytes += tile.len() as u64;
                    aggregate.tiles_applicable += 1;
                    aggregate.framing_bytes += measurement.framing_bytes as u64;
                    if !measurement.exact {
                        aggregate.exact = false;
                    }
                }
                "unsupported" => {}
                _ => {
                    aggregate.tiles_failed += 1;
                    aggregate.exact = false;
                }
            }

            if !first_result {
                results_json.push(',');
            }
            first_result = false;
            results_json.push_str(&format!(
                "{{\"name\":\"{}\",\"codec_id\":{},\"status\":\"{}\",\"payload_bytes\":{},\"bpp\":{},\"ratio\":{},\"encode_ms\":{},\"decode_ms\":{},\"exact\":{},\"framing_bytes\":{},\"model_state_bytes\":{},\"peak_encode_alloc_bytes\":{},\"peak_decode_alloc_bytes\":{},\"error\":{}}}",
                measurement.name,
                measurement.codec_id,
                measurement.status,
                measurement.payload_bytes,
                number(measurement.bpp),
                number(measurement.ratio),
                number(measurement.encode_ms),
                number(measurement.decode_ms),
                measurement.exact,
                measurement.framing_bytes,
                measurement.model_state_bytes,
                measurement.peak_encode_alloc_bytes,
                measurement.peak_decode_alloc_bytes,
                match &measurement.error {
                    Some(text) => format!("\"{}\"", escape(text)),
                    None => "null".to_string(),
                }
            ));

            if measurement.status == "ok" {
                candidates.push((measurement.name, measurement.payload_bytes));
            }
        }

        // The selection: the smallest complete payload, with the candidate
        // order as the tie-break.
        if let Some((name, bytes)) = candidates
            .iter()
            .min_by_key(|(_, payload_bytes)| *payload_bytes)
            .copied()
        {
            if let Some(slot) = selection_counts.iter_mut().find(|(n, _)| *n == name) {
                slot.1 += 1;
            }
            selection_payload_bytes += bytes as u64;
            selection_canonical_bytes += tile.len() as u64;
        }

        if !first_tile {
            tiles_json.push(',');
        }
        first_tile = false;
        tiles_json.push_str(&format!(
            "{{\"index\":{},\"x\":{},\"y\":{},\"width\":{},\"height\":{},\"canonical_bytes\":{},\"results\":[{}]}}",
            index,
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            tile.len(),
            results_json
        ));
    }

    // The per-image aggregate, as an array.
    let mut per_image = String::from("[");
    let framing = container_framing_bytes(u64::from(tile_count));
    for (slot, aggregate) in aggregates.iter().enumerate() {
        if slot > 0 {
            per_image.push(',');
        }
        let bpp = if aggregate.canonical_bytes > 0 {
            aggregate.payload_bytes as f64 * 8.0 / aggregate.canonical_bytes as f64
        } else {
            0.0
        };
        let ratio = if aggregate.canonical_bytes > 0 {
            aggregate.payload_bytes as f64 / aggregate.canonical_bytes as f64
        } else {
            0.0
        };
        // The whole-file size the same payloads would occupy in a real VPNG
        // container: the payloads plus the header, the directory and the
        // directory's CRC trailer.
        let file_bytes = aggregate.payload_bytes + framing;
        let file_ratio = if aggregate.canonical_bytes > 0 {
            file_bytes as f64 / aggregate.canonical_bytes as f64
        } else {
            0.0
        };
        let file_bpp = if aggregate.canonical_bytes > 0 {
            file_bytes as f64 * 8.0 / aggregate.canonical_bytes as f64
        } else {
            0.0
        };
        per_image.push_str(&format!(
            "{{\"name\":\"{}\",\"payload_bytes\":{},\"canonical_bytes\":{},\"bpp\":{},\"ratio\":{},\"file_bytes\":{},\"file_ratio\":{},\"file_bpp\":{},\"encode_ms\":{},\"decode_ms\":{},\"tiles_applicable\":{},\"tiles_failed\":{},\"framing_bytes\":{},\"model_state_bytes\":{},\"peak_encode_alloc_bytes\":{},\"peak_decode_alloc_bytes\":{},\"exact\":{}}}",
            aggregate.name,
            aggregate.payload_bytes,
            aggregate.canonical_bytes,
            number(bpp),
            number(ratio),
            file_bytes,
            number(file_ratio),
            number(file_bpp),
            number(aggregate.encode_ms),
            number(aggregate.decode_ms),
            aggregate.tiles_applicable,
            aggregate.tiles_failed,
            aggregate.framing_bytes,
            aggregate.model_state_bytes,
            aggregate.peak_encode_alloc_bytes,
            aggregate.peak_decode_alloc_bytes,
            aggregate.exact
        ));
    }
    per_image.push(']');

    let mut selection_json = String::from("{");
    for (slot, (name, count)) in selection_counts.iter().enumerate() {
        if slot > 0 {
            selection_json.push(',');
        }
        selection_json.push_str(&format!("\"{name}\":{count}"));
    }
    selection_json.push('}');

    // The document is assembled by concatenation: nested format strings with
    // literal braces are easy to get subtly wrong.
    let mut document = String::new();
    document.push_str("{\"tool\":\"vpng-research\",\"schema\":1,\"input\":{");
    document.push_str(&format!("\"path\":\"{}\"", escape(&args.input)));
    document.push_str(&format!(",\"width\":{}", args.width));
    document.push_str(&format!(",\"height\":{}", args.height));
    document.push_str(&format!(",\"format\":\"{}\"", escape(&format_name(format))));
    document.push_str(&format!(",\"tile_width\":{}", args.tile_size));
    document.push_str(&format!(",\"tile_height\":{}", args.tile_size));
    document.push_str(&format!(",\"tile_count\":{tile_count}"));
    document.push_str(&format!(",\"canonical_bytes\":{canonical_bytes}}}"));
    document.push_str(",\"candidates\":[");
    for (slot, (name, _)) in codecs.iter().enumerate() {
        if slot > 0 {
            document.push(',');
        }
        document.push_str(&format!("\"{}\"", escape(name)));
    }
    document.push_str("],\"tiles\":[");
    document.push_str(&tiles_json);
    document.push_str("],\"per_image\":");
    document.push_str(&per_image);
    document.push_str(",\"selection\":{");
    document.push_str(&format!(
        "\"payload_bytes\":{selection_payload_bytes},\"canonical_bytes\":{selection_canonical_bytes},\"ratio\":{}",
        number(if selection_canonical_bytes > 0 {
            selection_payload_bytes as f64 / selection_canonical_bytes as f64
        } else {
            0.0
        })
    ));
    document.push_str(&format!(
        ",\"file_bytes\":{},\"file_ratio\":{}",
        selection_payload_bytes + container_framing_bytes(u64::from(tile_count)),
        number(if selection_canonical_bytes > 0 {
            (selection_payload_bytes + container_framing_bytes(u64::from(tile_count))) as f64
                / selection_canonical_bytes as f64
        } else {
            0.0
        })
    ));
    document.push_str(",\"tiles\":");
    document.push_str(&selection_json);
    document.push_str("},\"process\":{");
    document.push_str(&format!("\"peak_heap_bytes\":{}", peak_bytes()));
    document.push_str(&format!(",\"current_heap_bytes\":{}", current_bytes()));
    document.push_str(&format!(",\"allocations\":{}}}", allocation_count()));
    document.push('}');

    match &args.output {
        Some(path) => std::fs::write(path, format!("{document}\n"))?,
        None => println!("{document}"),
    }
    Ok(())
}
