//! The memory validation driver.
//!
//! One validation case runs per process, so the process's resident-set
//! high-water mark belongs to that case alone. A driver script runs the
//! cases and aggregates their reports.
//!
//! ```text
//! vpng-memory --width 16384 --height 16384 --format rgba8 \
//!     --tile-size 512 --work-dir D:\work --output case.json
//! ```
//!
//! The case does four things, in order, and measures each one:
//!
//! 1. writes the image to a file, tile by tile, without holding it in
//!    memory;
//! 2. verifies the file with the ordinary decoder;
//! 3. opens the file with the file-backed editor, reads a region, writes a
//!    region back, and commits the result;
//! 4. verifies that the untouched tiles kept their payloads byte for byte
//!    and that the edited region has the right pixels.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use vpng_core::{Error, Limits, PixelFormat, Rect};
use vpng_editing::{CommitOptions, FileEditor, TileSource};
use vpng_validation::json::Json;
use vpng_validation::mem;

/// The tracking allocator, so the tool can report the heap it actually
/// used rather than guessing from the operating system's figures.
#[global_allocator]
static ALLOCATOR: mem::Tracking = mem::Tracking;
use vpng_validation::stream;

fn main() -> ExitCode {
    let mut width = 4096u32;
    let mut height = 4096u32;
    let mut format = PixelFormat::rgba8();
    let mut tile_size = 512u32;
    let mut work_dir = PathBuf::from(".");
    let mut output: Option<PathBuf> = None;
    let mut edit = true;
    let mut cache_bytes = 16 * 1024 * 1024u64;
    let mut history_bytes = 32 * 1024 * 1024u64;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--width" => width = parse_u32(args.next()),
            "--height" => height = parse_u32(args.next()),
            "--format" => {
                let value = args.next().unwrap_or_default();
                format = match value.as_str() {
                    "gray8" => PixelFormat::gray8(),
                    "gray16" => PixelFormat::gray16(),
                    "rgb8" => PixelFormat::rgb8(),
                    "rgb16" => PixelFormat::rgb16(),
                    "rgba8" => PixelFormat::rgba8(),
                    "rgba16" => PixelFormat::rgba16(),
                    other => {
                        eprintln!("unknown format {other}");
                        return ExitCode::from(2);
                    }
                };
            }
            "--tile-size" => tile_size = parse_u32(args.next()),
            "--work-dir" => work_dir = PathBuf::from(args.next().unwrap_or_default()),
            "--output" => output = args.next().map(PathBuf::from),
            "--no-edit" => edit = false,
            "--cache-bytes" => cache_bytes = parse_u64(args.next()),
            "--history-bytes" => history_bytes = parse_u64(args.next()),
            "--help" | "-h" => {
                println!(
                    "usage: vpng-memory --width N --height N --format FMT --tile-size N \
                     --work-dir DIR [--output FILE] [--no-edit]"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument {other}");
                return ExitCode::from(2);
            }
        }
    }

    if let Err(error) = fs::create_dir_all(&work_dir) {
        eprintln!("could not create {}: {error}", work_dir.display());
        return ExitCode::from(2);
    }

    let limits = Limits::default();
    let path = work_dir.join(format!("validation-{width}x{height}-{}.vpng", format));

    let baseline = mem::sample();

    // Step one: write the file, tile by tile.
    mem::reset_peaks();
    let write_started = Instant::now();
    let written =
        match stream::write_streaming(&path, width, height, format, tile_size, &limits, None) {
            Ok(report) => report,
            Err(error) => {
                eprintln!("the streaming write failed: {error}");
                return ExitCode::FAILURE;
            }
        };
    let write_elapsed = write_started.elapsed();
    let after_write = mem::sample();

    let mut fields = vec![
        (
            "case".into(),
            Json::Obj(vec![
                ("width".into(), Json::UInt(width as u64)),
                ("height".into(), Json::UInt(height as u64)),
                ("format".into(), Json::string(format.to_string())),
                ("tile_size".into(), Json::UInt(tile_size as u64)),
                (
                    "canonical_bytes".into(),
                    Json::UInt(format.frame_bytes(width, height).unwrap_or(0)),
                ),
                ("cache_budget_bytes".into(), Json::UInt(cache_bytes)),
                ("history_budget_bytes".into(), Json::UInt(history_bytes)),
            ]),
        ),
        (
            "baseline".into(),
            Json::Obj(vec![
                (
                    "working_set_bytes".into(),
                    Json::UInt(baseline.working_set_bytes),
                ),
                (
                    "heap_bytes".into(),
                    Json::UInt(baseline.heap_current_bytes as u64),
                ),
            ]),
        ),
        (
            "write".into(),
            Json::Obj(vec![
                ("file_size_bytes".into(), Json::UInt(written.file_size)),
                ("payload_bytes".into(), Json::UInt(written.payload_bytes)),
                ("framing_bytes".into(), Json::UInt(written.framing_bytes)),
                ("tile_count".into(), Json::UInt(written.tile_count as u64)),
                (
                    "codec_histogram".into(),
                    Json::Obj(
                        written
                            .codec_histogram
                            .iter()
                            .map(|(name, count)| (name.clone(), Json::UInt(*count as u64)))
                            .collect(),
                    ),
                ),
                ("elapsed_ms".into(), Json::UInt(written.encode_ms)),
                (
                    "peak_heap_bytes".into(),
                    Json::UInt(written.peak_heap_bytes as u64),
                ),
                (
                    "peak_working_set_bytes".into(),
                    Json::UInt(written.peak_working_set_bytes),
                ),
                (
                    "tile_buffer_bytes".into(),
                    Json::UInt(written.tile_buffer_bytes),
                ),
            ]),
        ),
        (
            "write_memory".into(),
            Json::Obj(vec![
                (
                    "heap_peak_bytes".into(),
                    Json::UInt(after_write.heap_peak_bytes as u64),
                ),
                (
                    "working_set_at_end_bytes".into(),
                    Json::UInt(after_write.working_set_bytes),
                ),
                (
                    "peak_working_set_bytes".into(),
                    Json::UInt(after_write.peak_working_set_bytes),
                ),
                (
                    "working_set_delta_bytes".into(),
                    Json::UInt(
                        after_write
                            .peak_working_set_bytes
                            .saturating_sub(baseline.working_set_bytes),
                    ),
                ),
            ]),
        ),
    ];

    // Step two: verify the file.
    let verification = match stream::verify_streamed_file(&path, &limits) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("the streamed file did not verify: {error}");
            return ExitCode::FAILURE;
        }
    };
    fields.push((
        "verification".into(),
        Json::Obj(vec![
            (
                "tile_count".into(),
                Json::UInt(verification.tile_count as u64),
            ),
            (
                "checked_tiles".into(),
                Json::UInt(verification.checked_tiles as u64),
            ),
            (
                "mismatches".into(),
                Json::UInt(verification.mismatches as u64),
            ),
            ("elapsed_ms".into(), Json::UInt(verification.verify_ms)),
            ("valid".into(), Json::Bool(verification.mismatches == 0)),
        ]),
    ));

    // Step three: the localized edit.
    if edit {
        match localized_edit(
            &path,
            width,
            height,
            format,
            tile_size,
            &limits,
            cache_bytes,
            history_bytes,
        ) {
            Ok(value) => fields.push(("edit".into(), value)),
            Err(error) => {
                eprintln!("the localized edit failed: {error}");
                fields.push((
                    "edit".into(),
                    Json::Obj(vec![("error".into(), Json::string(error.to_string()))]),
                ));
            }
        }
    }

    fields.push((
        "process_peak".into(),
        Json::Obj(vec![
            (
                "peak_working_set_bytes".into(),
                Json::UInt(mem::sample().peak_working_set_bytes),
            ),
            (
                "peak_heap_bytes".into(),
                Json::UInt(mem::peak_heap_bytes() as u64),
            ),
            (
                "allocations".into(),
                Json::UInt(mem::allocation_count() as u64),
            ),
        ]),
    ));

    let document = Json::Obj(fields).to_string_pretty();
    match output {
        Some(path) => {
            if let Err(error) = fs::write(&path, &document) {
                eprintln!("could not write {}: {error}", path.display());
                return ExitCode::from(2);
            }
            println!("wrote {}", path.display());
        }
        None => print!("{document}"),
    }

    let _ = fs::remove_file(&path);
    let _ = write_elapsed;
    ExitCode::SUCCESS
}

/// The localized edit: read a region, write it back, commit, verify.
#[allow(clippy::too_many_arguments)]
fn localized_edit(
    path: &std::path::Path,
    width: u32,
    height: u32,
    format: PixelFormat,
    tile_size: u32,
    limits: &Limits,
    cache_bytes: u64,
    history_bytes: u64,
) -> Result<Json, Error> {
    let before = mem::sample();
    mem::reset_peaks();

    let original_payloads = payload_digests(path, limits)?;

    let mut editor = FileEditor::open(path, limits)?
        .with_cache_bytes(cache_bytes)?
        .with_history_bytes(history_bytes)?;

    let rect = Rect::new(
        (width / 2).min(width.saturating_sub(64)),
        (height / 2).min(height.saturating_sub(64)),
        64.min(width),
        64.min(height),
    )?;

    let started = Instant::now();
    let region = editor.editor().read_region(rect)?;
    let read_ms = started.elapsed().as_secs_f64() * 1000.0;

    let started = Instant::now();
    let mut patch = region.data().to_vec();
    for byte in patch.iter_mut() {
        *byte = byte.wrapping_add(13);
    }
    editor.editor().write_region(rect, &patch)?;
    let write_ms = started.elapsed().as_secs_f64() * 1000.0;

    let stats = editor.editor().cache_stats();
    let history = editor.editor().history_stats();
    let dirty = editor.editor().dirty_tiles();

    let started = Instant::now();
    let commit = editor.editor().commit(path, &CommitOptions::default())?;
    let commit_ms = started.elapsed().as_secs_f64() * 1000.0;

    let after = mem::sample();
    let new_payloads = payload_digests(path, limits)?;
    let mut identical = 0u32;
    let mut changed = 0u32;
    for (index, digest) in new_payloads.iter().enumerate() {
        if original_payloads.get(index) == Some(digest) {
            identical += 1;
        } else {
            changed += 1;
        }
    }

    // The edited region must read back exactly.
    let mut verifier = FileEditor::open(path, limits)?;
    let read_back = verifier.editor().read_region(rect)?;

    let full_image_bytes = (width as u64) * (height as u64) * format.bytes_per_pixel();

    Ok(Json::Obj(vec![
        (
            "full_image_pixel_bytes".into(),
            Json::UInt(full_image_bytes),
        ),
        (
            "region".into(),
            Json::Obj(vec![
                ("x".into(), Json::UInt(rect.x as u64)),
                ("y".into(), Json::UInt(rect.y as u64)),
                ("width".into(), Json::UInt(rect.width as u64)),
                ("height".into(), Json::UInt(rect.height as u64)),
            ]),
        ),
        ("read_ms".into(), Json::Num(read_ms)),
        ("write_ms".into(), Json::Num(write_ms)),
        ("commit_ms".into(), Json::Num(commit_ms)),
        (
            "cache".into(),
            Json::Obj(vec![
                ("hits".into(), Json::UInt(stats.hits)),
                ("misses".into(), Json::UInt(stats.misses)),
                ("evictions".into(), Json::UInt(stats.evictions)),
                ("decoded_tiles".into(), Json::UInt(stats.decoded_tiles)),
                ("rejected_tiles".into(), Json::UInt(stats.rejected_tiles)),
                ("resident_tiles".into(), Json::UInt(stats.resident_tiles)),
                ("current_bytes".into(), Json::UInt(stats.current_bytes)),
                ("peak_bytes".into(), Json::UInt(stats.peak_bytes)),
                ("decode_time_ms".into(), Json::Num(stats.decode_time_ms())),
                ("budget_bytes".into(), Json::UInt(cache_bytes)),
                (
                    "within_budget".into(),
                    Json::Bool(stats.peak_bytes <= cache_bytes),
                ),
            ]),
        ),
        (
            "history".into(),
            Json::Obj(vec![
                ("undo_depth".into(), Json::UInt(history.undo_depth as u64)),
                ("current_bytes".into(), Json::UInt(history.current_bytes)),
                ("peak_bytes".into(), Json::UInt(history.peak_bytes)),
                (
                    "dropped_entries".into(),
                    Json::UInt(history.dropped_entries),
                ),
                ("budget_bytes".into(), Json::UInt(history_bytes)),
                (
                    "within_budget".into(),
                    Json::Bool(history.peak_bytes <= history_bytes),
                ),
            ]),
        ),
        (
            "dirty".into(),
            Json::Obj(vec![
                ("tile_count".into(), Json::UInt(dirty.len() as u64)),
                ("bytes".into(), Json::UInt(editor.editor().dirty_bytes())),
            ]),
        ),
        (
            "commit".into(),
            Json::Obj(vec![
                (
                    "tiles_rewritten".into(),
                    Json::UInt(commit.tiles_rewritten as u64),
                ),
                (
                    "tiles_copied".into(),
                    Json::UInt(commit.tiles_copied as u64),
                ),
                (
                    "copied_payload_bytes".into(),
                    Json::UInt(commit.copied_payload_bytes),
                ),
                (
                    "rewritten_payload_bytes".into(),
                    Json::UInt(commit.rewritten_payload_bytes),
                ),
                ("file_size".into(), Json::UInt(commit.file_size)),
            ]),
        ),
        (
            "payload_identity".into(),
            Json::Obj(vec![
                ("identical_tiles".into(), Json::UInt(identical as u64)),
                ("changed_tiles".into(), Json::UInt(changed as u64)),
                (
                    "untouched_payloads_byte_identical".into(),
                    Json::Bool(changed as usize == dirty.len().max(1)),
                ),
            ]),
        ),
        (
            "pixels_read_back_exact".into(),
            Json::Bool(read_back.data() == patch.as_slice()),
        ),
        (
            "memory".into(),
            Json::Obj(vec![
                (
                    "working_set_before_bytes".into(),
                    Json::UInt(before.working_set_bytes),
                ),
                (
                    "working_set_peak_bytes".into(),
                    Json::UInt(after.peak_working_set_bytes),
                ),
                (
                    "working_set_delta_bytes".into(),
                    Json::UInt(
                        after
                            .peak_working_set_bytes
                            .saturating_sub(before.working_set_bytes),
                    ),
                ),
                (
                    "heap_peak_bytes".into(),
                    Json::UInt(after.heap_peak_bytes as u64),
                ),
                (
                    "heap_delta_bytes".into(),
                    Json::UInt(after.heap_peak_bytes.saturating_sub(before.heap_peak_bytes) as u64),
                ),
                (
                    "pagefile_peak_bytes".into(),
                    Json::UInt(after.peak_pagefile_bytes),
                ),
            ]),
        ),
        ("tile_size".into(), Json::UInt(tile_size as u64)),
    ]))
}

/// The CRC32C of every tile payload, so two files can be compared tile by
/// tile without holding either file in memory.
fn payload_digests(path: &std::path::Path, limits: &Limits) -> Result<Vec<u32>, Error> {
    let mut source = vpng_editing::FileSource::open(path, limits)?;
    let count = source.tile_count() as usize;
    let mut digests = Vec::with_capacity(count);
    for index in 0..count {
        let payload = source.tile_payload(index)?;
        digests.push(vpng_container::crc32c(&payload));
    }
    Ok(digests)
}

/// Parse an unsigned 32-bit argument.
fn parse_u32(value: Option<String>) -> u32 {
    value.and_then(|text| text.parse::<u32>().ok()).unwrap_or(0)
}

/// Parse an unsigned 64-bit argument.
fn parse_u64(value: Option<String>) -> u64 {
    value.and_then(|text| text.parse::<u64>().ok()).unwrap_or(0)
}
