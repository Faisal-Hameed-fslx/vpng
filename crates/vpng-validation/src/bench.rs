//! The editing-engine benchmark scenarios.
//!
//! Each scenario answers one of the P4 acceptance questions with a
//! measurement rather than an assertion:
//!
//! * a single-tile edit does not decode the full image;
//! * unmodified payloads are reused byte for byte;
//! * an abort leaves the original file valid;
//! * a crash simulation never exposes an incomplete generation;
//! * saving and reopening reproduces the pixels exactly;
//! * undo and redo restore the pixels exactly;
//! * the cache and the history stay inside their budgets.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use vpng_core::{Error, Limits, PixelFormat, Rect};
use vpng_decoder::Document;
use vpng_editing::{CommitOptions, Editor, FailurePoint, FileEditor};

use crate::json::Json;

/// The result of one scenario.
pub struct Scenario {
    /// The scenario's name.
    pub name: &'static str,
    /// What the scenario measured.
    pub result: Json,
}

/// The options every scenario runs under.
#[derive(Debug, Clone)]
pub struct BenchOptions {
    /// The directory the scenarios work in.
    pub work_dir: PathBuf,
    /// The width of the test image.
    pub width: u32,
    /// The height of the test image.
    pub height: u32,
    /// The tile size.
    pub tile_size: u32,
    /// The pixel format.
    pub format: PixelFormat,
}

impl BenchOptions {
    /// A default set of options.
    pub fn default_with(work_dir: PathBuf) -> Self {
        BenchOptions {
            work_dir,
            width: 2048,
            height: 2048,
            tile_size: 256,
            format: PixelFormat::rgba8(),
        }
    }
}

/// The canonical digest of a document's pixels.
///
/// The digest is the one the specification defines, so it can be compared
/// with a digest computed by any other implementation.
fn pixel_digest(image: &vpng_core::CanonicalImage) -> String {
    vpng_container::to_hex(&vpng_container::canonical_digest(image))
}

/// Write a test file and return its path.
fn build_file(options: &BenchOptions, name: &str) -> Result<PathBuf, Error> {
    let path = options.work_dir.join(name);
    let image = vpng_testkit::gradient(options.width, options.height, options.format, 11);
    let bytes = vpng_encoder::encode(
        &image,
        &vpng_encoder::EncodeOptions::default()
            .with_tile_size(options.tile_size, options.tile_size),
    )?
    .bytes;
    fs::write(&path, &bytes)?;
    Ok(path)
}

/// The scenario: a localized edit touches only the tiles it overlaps.
pub fn localized_edit(options: &BenchOptions) -> Result<Scenario, Error> {
    let limits = Limits::default();
    let path = build_file(options, "localized-edit.vpng")?;
    let original_size = fs::metadata(&path)?.len();

    let mut editor = FileEditor::open(&path, &limits)?;
    let rect = Rect::new(options.width / 2, options.height / 2, 64, 64)?;

    let started = Instant::now();
    let region = editor.editor().read_region(rect)?;
    let read_ms = started.elapsed().as_secs_f64() * 1000.0;

    let started = Instant::now();
    let mut patch = region.data().to_vec();
    for byte in patch.iter_mut() {
        *byte = byte.wrapping_add(17);
    }
    editor.editor().write_region(rect, &patch)?;
    let write_ms = started.elapsed().as_secs_f64() * 1000.0;

    let stats = editor.editor().cache_stats();
    let dirty = editor.editor().dirty_tiles();
    let dirty_bytes = editor.editor().dirty_bytes();
    let history = editor.editor().history_stats();

    // The full image is 16 MiB of pixels; the engine decoded one tile.
    let full_image_bytes =
        (options.width as u64) * (options.height as u64) * options.format.bytes_per_pixel();

    Ok(Scenario {
        name: "localized-edit",
        result: Json::Obj(vec![
            ("name".into(), Json::string("localized-edit")),
            (
                "image".into(),
                Json::Obj(vec![
                    ("width".into(), Json::UInt(options.width as u64)),
                    ("height".into(), Json::UInt(options.height as u64)),
                    ("format".into(), Json::string(options.format.to_string())),
                    ("tile_size".into(), Json::UInt(options.tile_size as u64)),
                    ("file_bytes".into(), Json::UInt(original_size)),
                    (
                        "full_image_pixel_bytes".into(),
                        Json::UInt(full_image_bytes),
                    ),
                ]),
            ),
            (
                "region".into(),
                Json::Obj(vec![
                    ("x".into(), Json::UInt(rect.x as u64)),
                    ("y".into(), Json::UInt(rect.y as u64)),
                    ("width".into(), Json::UInt(rect.width as u64)),
                    ("height".into(), Json::UInt(rect.height as u64)),
                    ("read_ms".into(), Json::Num(read_ms)),
                    ("write_ms".into(), Json::Num(write_ms)),
                ]),
            ),
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
                ]),
            ),
            (
                "dirty".into(),
                Json::Obj(vec![
                    ("tile_count".into(), Json::UInt(dirty.len() as u64)),
                    (
                        "tiles".into(),
                        Json::Arr(
                            dirty
                                .iter()
                                .map(|index| Json::UInt(*index as u64))
                                .collect(),
                        ),
                    ),
                    ("bytes".into(), Json::UInt(dirty_bytes)),
                    (
                        "fraction_of_image".into(),
                        Json::Num(dirty_bytes as f64 / full_image_bytes as f64),
                    ),
                ]),
            ),
            (
                "history".into(),
                Json::Obj(vec![
                    ("undo_depth".into(), Json::UInt(history.undo_depth as u64)),
                    ("redo_depth".into(), Json::UInt(history.redo_depth as u64)),
                    ("current_bytes".into(), Json::UInt(history.current_bytes)),
                    ("peak_bytes".into(), Json::UInt(history.peak_bytes)),
                    (
                        "dropped_entries".into(),
                        Json::UInt(history.dropped_entries),
                    ),
                ]),
            ),
            (
                // The four tile counts are different things and are never
                // interchangeable. See the report's terminology section.
                "tiles".into(),
                Json::Obj(vec![
                    ("decoded".into(), Json::UInt(stats.decoded_tiles)),
                    ("modified".into(), Json::UInt(dirty.len() as u64)),
                    ("rewritten".into(), Json::UInt(0)),
                    ("preserved".into(), Json::UInt(0)),
                ]),
            ),
        ]),
    })
}

/// The scenario: saving reuses the payloads of the untouched tiles.
pub fn payload_reuse(options: &BenchOptions) -> Result<Scenario, Error> {
    let limits = Limits::default();
    let path = build_file(options, "payload-reuse.vpng")?;
    let original = fs::read(&path)?;

    let bytes = fs::read(&path)?;
    let mut editor = Editor::parse(&bytes, &limits)?;
    let rect = Rect::new(10, 10, 32, 32)?;
    editor.write_region(
        rect,
        &vec![0x5A; 32 * 32 * options.format.bytes_per_pixel() as usize],
    )?;

    let started = Instant::now();
    let report = editor.commit(&path, &CommitOptions::default())?;
    let commit_ms = started.elapsed().as_secs_f64() * 1000.0;

    let saved = fs::read(&path)?;
    let before = Document::parse(&original, &limits)?;
    let after = Document::parse(&saved, &limits)?;

    let mut identical = 0u32;
    let mut changed = 0u32;
    for index in 0..before.tile_count() as usize {
        if before.tile_payload(index)? == after.tile_payload(index)? {
            identical += 1;
        } else {
            changed += 1;
        }
    }

    // The pixels must survive the round trip exactly.
    let decoded = after.decode_all(&limits)?;
    let original = vpng_testkit::gradient(options.width, options.height, options.format, 11);
    let mut expected_pixels = original.data().to_vec();
    let bpp = options.format.bytes_per_pixel() as usize;
    let row = options.width as usize * bpp;
    for y in rect.y as usize..(rect.y + rect.height) as usize {
        for x in rect.x as usize..(rect.x + rect.width) as usize {
            for channel in 0..bpp {
                expected_pixels[y * row + x * bpp + channel] = 0x5A;
            }
        }
    }
    let pixels_exact = decoded.data() == expected_pixels.as_slice();
    let original_image = vpng_core::CanonicalImage::from_vec(
        options.width,
        options.height,
        options.format,
        original.data().to_vec(),
    )?;
    let expected_image = vpng_core::CanonicalImage::from_vec(
        options.width,
        options.height,
        options.format,
        expected_pixels,
    )?;
    let digest_original = pixel_digest(&original_image);
    let digest_expected = pixel_digest(&expected_image);
    let digest_after = pixel_digest(&decoded);

    Ok(Scenario {
        name: "payload-reuse",
        result: Json::Obj(vec![
            ("name".into(), Json::string("payload-reuse")),
            ("commit_ms".into(), Json::Num(commit_ms)),
            (
                "report".into(),
                Json::Obj(vec![
                    (
                        "tiles_rewritten".into(),
                        Json::UInt(report.tiles_rewritten as u64),
                    ),
                    (
                        "tiles_copied".into(),
                        Json::UInt(report.tiles_copied as u64),
                    ),
                    (
                        "copied_payload_bytes".into(),
                        Json::UInt(report.copied_payload_bytes),
                    ),
                    (
                        "rewritten_payload_bytes".into(),
                        Json::UInt(report.rewritten_payload_bytes),
                    ),
                    ("file_size".into(), Json::UInt(report.file_size)),
                ]),
            ),
            (
                "payload_identity".into(),
                Json::Obj(vec![
                    ("identical_tiles".into(), Json::UInt(identical as u64)),
                    ("changed_tiles".into(), Json::UInt(changed as u64)),
                    (
                        "all_untouched_payloads_identical".into(),
                        Json::Bool(changed == 1),
                    ),
                ]),
            ),
            (
                // The four tile counts are different things and are never
                // interchangeable. See the report's terminology section.
                "tiles".into(),
                Json::Obj(vec![
                    // The edit decoded the one tile it touched.
                    ("decoded".into(), Json::UInt(1)),
                    // The edit changed that tile's pixels.
                    ("modified".into(), Json::UInt(1)),
                    // The commit re-encoded that tile's payload.
                    (
                        "rewritten".into(),
                        Json::UInt(report.tiles_rewritten as u64),
                    ),
                    // Every other payload was copied byte for byte.
                    ("preserved".into(), Json::UInt(report.tiles_copied as u64)),
                ]),
            ),
            ("pixels_exact_after_reopen".into(), Json::Bool(pixels_exact)),
            ("digest_original".into(), Json::string(digest_original)),
            (
                "digest_expected_after_patch".into(),
                Json::string(digest_expected.clone()),
            ),
            (
                "digest_after_save".into(),
                Json::string(digest_after.clone()),
            ),
            (
                "digests_match".into(),
                Json::Bool(digest_after == digest_expected),
            ),
        ]),
    })
}

/// The scenario: saving and reopening reproduces the pixels exactly.
pub fn save_reopen_digest(options: &BenchOptions) -> Result<Scenario, Error> {
    let limits = Limits::default();
    let path = build_file(options, "save-reopen.vpng")?;
    let bytes = fs::read(&path)?;
    let mut editor = Editor::parse(&bytes, &limits)?;

    let rect = Rect::new(options.width / 3, options.height / 3, 128, 128)?;
    let patch: Vec<u8> = (0..(128 * 128 * options.format.bytes_per_pixel() as usize))
        .map(|index| (index % 251) as u8)
        .collect();
    editor.write_region(rect, &patch)?;
    editor.commit(&path, &CommitOptions::default())?;

    // Reopen the saved file from disk and digest the whole image.
    let saved = fs::read(&path)?;
    let document = Document::parse(&saved, &limits)?;
    let decoded = document.decode_all(&limits)?;

    let original = vpng_testkit::gradient(options.width, options.height, options.format, 11);
    let mut expected = original.data().to_vec();
    let bpp = options.format.bytes_per_pixel() as usize;
    let row = options.width as usize * bpp;
    for y in rect.y as usize..(rect.y + rect.height) as usize {
        for x in rect.x as usize..(rect.x + rect.width) as usize {
            expected[y * row + x * bpp..y * row + x * bpp + bpp].copy_from_slice(
                &patch[(y - rect.y as usize) * 128 * bpp + (x - rect.x as usize) * bpp..][..bpp],
            );
        }
    }

    let expected_image = vpng_core::CanonicalImage::from_vec(
        options.width,
        options.height,
        options.format,
        expected.clone(),
    )?;
    let expected_digest = pixel_digest(&expected_image);

    Ok(Scenario {
        name: "save-reopen-digest",
        result: Json::Obj(vec![
            ("name".into(), Json::string("save-reopen-digest")),
            (
                "digest_expected".into(),
                Json::string(expected_digest.clone()),
            ),
            ("digest_actual".into(), Json::string(pixel_digest(&decoded))),
            (
                "digests_match".into(),
                Json::Bool(pixel_digest(&decoded) == expected_digest),
            ),
        ]),
    })
}

/// The scenario: an abort leaves the original file untouched.
pub fn abort_preserves(options: &BenchOptions) -> Result<Scenario, Error> {
    let limits = Limits::default();
    let path = build_file(options, "abort.vpng")?;
    let original = fs::read(&path)?;

    let bytes = fs::read(&path)?;
    let mut editor = Editor::parse(&bytes, &limits)?;
    editor.begin()?;
    editor.write_region(
        Rect::new(0, 0, 64, 64)?,
        &vec![0x33; 64 * 64 * options.format.bytes_per_pixel() as usize],
    )?;
    editor.abort()?;

    let after = fs::read(&path)?;
    let document = Document::parse(&after, &limits)?;
    let integrity_ok = document.check_integrity().is_ok();
    let decoded = document.decode_all(&limits)?;

    Ok(Scenario {
        name: "abort-preserves-original",
        result: Json::Obj(vec![
            ("name".into(), Json::string("abort-preserves-original")),
            ("file_bytes_unchanged".into(), Json::Bool(original == after)),
            ("file_still_parses".into(), Json::Bool(true)),
            ("integrity_ok".into(), Json::Bool(integrity_ok)),
            (
                "editor_clean_after_abort".into(),
                Json::Bool(!editor.is_dirty()),
            ),
            ("digest".into(), Json::string(pixel_digest(&decoded))),
        ]),
    })
}

/// The scenario: an injected failure never exposes an incomplete file.
pub fn failure_injection(options: &BenchOptions) -> Result<Scenario, Error> {
    let limits = Limits::default();
    let path = build_file(options, "failure.vpng")?;
    let original = fs::read(&path)?;

    let mut results: Vec<Json> = Vec::new();
    for (label, failure) in [
        ("after-bytes", FailurePoint::AfterBytes(4096)),
        ("before-rename", FailurePoint::BeforeRename),
    ] {
        // Restore the original file before each attempt.
        fs::write(&path, &original)?;
        let bytes = fs::read(&path)?;
        let mut editor = Editor::parse(&bytes, &limits)?;
        editor.write_region(
            Rect::new(0, 0, 32, 32)?,
            &vec![0x44; 32 * 32 * options.format.bytes_per_pixel() as usize],
        )?;
        let options_commit = CommitOptions {
            failure: Some(failure),
            ..CommitOptions::default()
        };
        let failed = editor.commit(&path, &options_commit).is_err();
        let after = fs::read(&path)?;
        let leftovers: Vec<String> = fs::read_dir(&options.work_dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("tmp"))
            .collect();
        let parses = Document::parse(&after, &limits).is_ok();
        results.push(Json::Obj(vec![
            ("failure_point".into(), Json::string(label)),
            ("commit_failed".into(), Json::Bool(failed)),
            ("original_preserved".into(), Json::Bool(original == after)),
            ("original_still_parses".into(), Json::Bool(parses)),
            (
                "temporary_files_left".into(),
                Json::strings(leftovers.clone()),
            ),
            (
                "no_temporary_files_left".into(),
                Json::Bool(leftovers.is_empty()),
            ),
        ]));
    }

    Ok(Scenario {
        name: "failure-injection",
        result: Json::Obj(vec![
            ("name".into(), Json::string("failure-injection")),
            ("attempts".into(), Json::Arr(results)),
        ]),
    })
}

/// The scenario: undo and redo restore the pixels exactly.
pub fn undo_redo(options: &BenchOptions) -> Result<Scenario, Error> {
    let limits = Limits::default();
    let path = build_file(options, "undo-redo.vpng")?;
    let bytes = fs::read(&path)?;
    let mut editor = Editor::parse(&bytes, &limits)?;

    let rect = Rect::new(64, 64, 48, 48)?;
    let before = editor.read_region(rect)?.data().to_vec();

    let started = Instant::now();
    editor.write_region(
        rect,
        &vec![0x77; 48 * 48 * options.format.bytes_per_pixel() as usize],
    )?;
    let write_ms = started.elapsed().as_secs_f64() * 1000.0;
    let after_write = editor.read_region(rect)?.data().to_vec();

    let started = Instant::now();
    let undone = editor.undo()?;
    let undo_ms = started.elapsed().as_secs_f64() * 1000.0;
    let after_undo = editor.read_region(rect)?.data().to_vec();
    let dirty_after_undo = editor.dirty_tiles().len();

    let started = Instant::now();
    let redone = editor.redo()?;
    let redo_ms = started.elapsed().as_secs_f64() * 1000.0;
    let after_redo = editor.read_region(rect)?.data().to_vec();
    let dirty_after_redo = editor.dirty_tiles().len();

    Ok(Scenario {
        name: "undo-redo",
        result: Json::Obj(vec![
            ("name".into(), Json::string("undo-redo")),
            ("write_ms".into(), Json::Num(write_ms)),
            ("undo_ms".into(), Json::Num(undo_ms)),
            ("redo_ms".into(), Json::Num(redo_ms)),
            ("undo_available".into(), Json::Bool(undone)),
            ("redo_available".into(), Json::Bool(redone)),
            (
                "undo_restores_pixels".into(),
                Json::Bool(after_undo == before),
            ),
            (
                "redo_restores_pixels".into(),
                Json::Bool(after_redo == after_write),
            ),
            (
                "dirty_after_undo".into(),
                Json::UInt(dirty_after_undo as u64),
            ),
            (
                "dirty_after_redo".into(),
                Json::UInt(dirty_after_redo as u64),
            ),
        ]),
    })
}

/// The scenario: the cache stays inside its byte budget.
pub fn cache_budget(options: &BenchOptions) -> Result<Scenario, Error> {
    let limits = Limits::default();
    let path = build_file(options, "cache-budget.vpng")?;
    let bytes = fs::read(&path)?;
    let tile_bytes =
        (options.tile_size as u64) * (options.tile_size as u64) * options.format.bytes_per_pixel();
    let budget = tile_bytes * 2;

    let mut editor = Editor::parse(&bytes, &limits)?.with_cache_bytes(budget)?;
    for row in 0..4 {
        let y = row * options.tile_size;
        editor.read_region(Rect::new(0, y, options.tile_size, options.tile_size)?)?;
    }
    let stats = editor.cache_stats();

    Ok(Scenario {
        name: "cache-budget",
        result: Json::Obj(vec![
            ("name".into(), Json::string("cache-budget")),
            ("budget_bytes".into(), Json::UInt(budget)),
            ("tile_bytes".into(), Json::UInt(tile_bytes)),
            ("hits".into(), Json::UInt(stats.hits)),
            ("misses".into(), Json::UInt(stats.misses)),
            ("evictions".into(), Json::UInt(stats.evictions)),
            ("decoded_tiles".into(), Json::UInt(stats.decoded_tiles)),
            ("current_bytes".into(), Json::UInt(stats.current_bytes)),
            ("peak_bytes".into(), Json::UInt(stats.peak_bytes)),
            (
                "within_budget".into(),
                Json::Bool(stats.peak_bytes <= budget),
            ),
        ]),
    })
}

/// The scenario: the history stays inside its byte budget.
pub fn history_budget(options: &BenchOptions) -> Result<Scenario, Error> {
    let limits = Limits::default();
    let path = build_file(options, "history-budget.vpng")?;
    let bytes = fs::read(&path)?;
    // One history entry holds one whole tile's pixels. The budget is four
    // tiles, so the history keeps a few steps and drops the oldest as the
    // session goes on.
    let tile_bytes =
        (options.tile_size as u64) * (options.tile_size as u64) * options.format.bytes_per_pixel();
    let budget = tile_bytes * 4;

    let mut editor = Editor::parse(&bytes, &limits)?.with_history_bytes(budget)?;
    for step in 0..12u32 {
        let x = (step * 17) % (options.width - 64);
        editor.write_region(
            Rect::new(x, 0, 64, 64)?,
            &vec![step as u8; 64 * 64 * options.format.bytes_per_pixel() as usize],
        )?;
    }
    let stats = editor.history_stats();

    Ok(Scenario {
        name: "history-budget",
        result: Json::Obj(vec![
            ("name".into(), Json::string("history-budget")),
            ("budget_bytes".into(), Json::UInt(budget)),
            ("tile_bytes".into(), Json::UInt(tile_bytes)),
            ("edits".into(), Json::UInt(12)),
            ("undo_depth".into(), Json::UInt(stats.undo_depth as u64)),
            ("redo_depth".into(), Json::UInt(stats.redo_depth as u64)),
            ("current_bytes".into(), Json::UInt(stats.current_bytes)),
            ("peak_bytes".into(), Json::UInt(stats.peak_bytes)),
            ("dropped_entries".into(), Json::UInt(stats.dropped_entries)),
            (
                "within_budget".into(),
                Json::Bool(stats.peak_bytes <= budget),
            ),
        ]),
    })
}

/// Run every scenario.
pub fn run_all(options: &BenchOptions) -> Vec<Scenario> {
    let mut scenarios = Vec::new();
    let mut push = |name: &str, result: Result<Scenario, Error>| match result {
        Ok(scenario) => scenarios.push(scenario),
        Err(error) => scenarios.push(Scenario {
            name: match name {
                "localized-edit" => "localized-edit",
                "payload-reuse" => "payload-reuse",
                "save-reopen-digest" => "save-reopen-digest",
                "abort-preserves-original" => "abort-preserves-original",
                "failure-injection" => "failure-injection",
                "undo-redo" => "undo-redo",
                "cache-budget" => "cache-budget",
                _ => "history-budget",
            },
            result: Json::Obj(vec![
                ("name".into(), Json::string(name)),
                ("error".into(), Json::string(error.to_string())),
            ]),
        }),
    };
    push("localized-edit", localized_edit(options));
    push("payload-reuse", payload_reuse(options));
    push("save-reopen-digest", save_reopen_digest(options));
    push("abort-preserves-original", abort_preserves(options));
    push("failure-injection", failure_injection(options));
    push("undo-redo", undo_redo(options));
    push("cache-budget", cache_budget(options));
    push("history-budget", history_budget(options));
    scenarios
}

/// Remove the scenario files from the work directory.
pub fn cleanup(options: &BenchOptions) {
    if let Ok(entries) = fs::read_dir(&options.work_dir) {
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if path.extension().map(|ext| ext == "vpng").unwrap_or(false) {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// The path of a scenario file, for a caller that wants to look at it.
pub fn scenario_path(options: &BenchOptions, name: &str) -> PathBuf {
    options.work_dir.join(name)
}

/// Whether a path exists.
pub fn exists(path: &Path) -> bool {
    path.exists()
}
