//! The preview-pyramid benchmark: what a pyramid buys, and what it costs.
//!
//! A pyramid is a promise: a viewer that opens the file can show a
//! recognisable picture long before it can show the original at full
//! resolution. This module measures that promise and its price, on the same
//! photographic source the P5.2 latency matrix used, so the numbers can be
//! compared with the baseline that is already in the report.
//!
//! # The scenarios
//!
//! * **The encode.** The same picture is encoded twice: once as an ordinary
//!   **v0.1** file, and once as a **v0.2** file with a preview pyramid. Both
//!   are repeated, and both record the SHA-256 of the file they wrote, so the
//!   report can say that the pyramid is deterministic and can say what it
//!   costs in bytes and in time.
//! * **The first viewport.** The P5.2 baseline: open the file and decode the
//!   tiles of a 1024 x 1024 viewport in the middle of the picture. It is
//!   measured on the v0.1 file and on the pyramid's level 0, so the report can
//!   say whether the pyramid slowed the full-resolution path down.
//! * **The levels.** Every level of the pyramid is decoded on its own, and
//!   the time is recorded. The last level is the **first preview**: the
//!   earliest moment at which the whole picture can be shown.
//! * **The whole picture.** The v0.1 file is decoded in full, for the
//!   like-for-like comparison with the smallest preview.
//!
//! # The rules
//!
//! * Every case is repeated and the **median** and the **p95** are reported,
//!   with the same percentile rules the P5.2 matrix uses.
//! * The bytes are checked: the SHA-256 of every file the benchmark writes is
//!   recorded, and the report says whether the runs agreed.
//! * The memory figures state what they include and what they exclude. The
//!   pyramid encoder has to see the whole picture — that is what building a
//!   preview means — so its working set is at least the size of the original.
//!   The default path does not do that, and keeps its bounded memory.
//! * Nothing is claimed that was not measured. A case that cannot run on this
//!   host is reported as BLOCKED, not PASS.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use vpng_core::{Error, Limits, PixelFormat, Rect, Result};
use vpng_encoder::parallel::ParallelOptions;
use vpng_encoder::stream::encode_to_file;
use vpng_encoder::EncodeOptions;
use vpng_source::TileReader;
use vpng_view::{Direction, FileDecoderFactory, Pool, PrefetchPolicy, Viewport};

use crate::json::Json;
use crate::photo_bench::{hash_file, numbers, stats, stats_json, RawSource};

/// The options the preview benchmark runs under.
#[derive(Debug, Clone)]
pub struct PreviewBenchOptions {
    /// The raw canonical pixel file the photograph was converted to.
    pub source_path: PathBuf,
    /// The image's width in pixels.
    pub width: u32,
    /// The image's height in pixels.
    pub height: u32,
    /// The pixel format.
    pub format: PixelFormat,
    /// The nominal tile size.
    pub tile_size: u32,
    /// How many workers the encode and the viewing pool use.
    pub workers: usize,
    /// How many times each case is repeated.
    pub runs: usize,
    /// How many derived preview levels the pyramid holds.
    pub pyramid_levels: u32,
    /// The width of the viewport, in pixels.
    pub viewport_width: u32,
    /// The height of the viewport, in pixels.
    pub viewport_height: u32,
    /// The viewing cache's byte budget.
    pub cache_bytes: u64,
    /// The request queue's depth.
    pub queue_capacity: usize,
    /// Where the benchmark's files are written.
    pub work_dir: PathBuf,
    /// Keep the files after the run.
    pub keep_file: bool,
}

impl PreviewBenchOptions {
    /// A default set of options for a given directory.
    pub fn default_with(work_dir: PathBuf) -> Self {
        PreviewBenchOptions {
            source_path: PathBuf::new(),
            width: 15360,
            height: 8640,
            format: PixelFormat::rgb8(),
            tile_size: 256,
            workers: 8,
            runs: 5,
            pyramid_levels: 3,
            viewport_width: 1024,
            viewport_height: 1024,
            cache_bytes: 32 * 1024 * 1024,
            queue_capacity: 256,
            work_dir,
            keep_file: false,
        }
    }

    /// The path the benchmark's v0.1 file is written to.
    pub fn base_path(&self) -> PathBuf {
        self.work_dir.join(format!(
            "preview-base-{width}x{height}-{format}-t{tile}.vpng",
            width = self.width,
            height = self.height,
            format = self.format,
            tile = self.tile_size,
        ))
    }

    /// The path the benchmark's pyramid file is written to.
    pub fn pyramid_path(&self) -> PathBuf {
        self.work_dir.join(format!(
            "preview-pyramid-{width}x{height}-{format}-t{tile}-l{levels}.vpng",
            width = self.width,
            height = self.height,
            format = self.format,
            tile = self.tile_size,
            levels = self.pyramid_levels,
        ))
    }
}

/// Run every scenario and return the report.
pub fn run(options: &PreviewBenchOptions) -> Result<Json> {
    let limits = Limits::default();
    fs::create_dir_all(&options.work_dir)?;

    let canonical_bytes = options.format.frame_bytes(options.width, options.height)?;
    let source_size = fs::metadata(&options.source_path)?.len();
    if source_size != canonical_bytes {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("the canonical file is {source_size} bytes but the image is {canonical_bytes}"),
        )));
    }

    let source = RawSource::open(
        &options.source_path,
        options.width,
        options.height,
        options.format,
    )?;

    let parallel = ParallelOptions::default().with_workers(options.workers);
    let base_options = EncodeOptions::default()
        .with_tile_size(options.tile_size, options.tile_size)
        .with_limits(limits);
    let pyramid_options = base_options.with_pyramid(options.pyramid_levels);

    // ---- the encodes ----------------------------------------------------
    let base_path = options.base_path();
    let pyramid_path = options.pyramid_path();

    let base_encode = encode_repeatedly(
        options,
        &source,
        &base_options,
        &parallel,
        &base_path,
        "v0.1",
    )?;
    let pyramid_encode = encode_repeatedly(
        options,
        &source,
        &pyramid_options,
        &parallel,
        &pyramid_path,
        "v0.2 pyramid",
    )?;

    // ---- the first viewport ---------------------------------------------
    // The P5.2 baseline, measured on the same files in the same run, so the
    // comparison is like for like.
    let base_viewport = viewport_repeatedly(options, &base_path, &limits, "v0.1")?;
    let pyramid_viewport = viewport_repeatedly(options, &pyramid_path, &limits, "v0.2 level 0")?;

    // ---- the levels ------------------------------------------------------
    let level_rows = level_decode_rows(options, &pyramid_path, &limits)?;

    // ---- the whole picture ----------------------------------------------
    let full_decode = full_decode_row(options, &base_path, &limits)?;

    if !options.keep_file {
        let _ = fs::remove_file(&base_path);
        let _ = fs::remove_file(&pyramid_path);
    }

    Ok(Json::Obj(vec![
        ("tool".into(), Json::string("vpng-preview-bench")),
        (
            "parameters".into(),
            Json::Obj(vec![
                ("width".into(), Json::UInt(options.width as u64)),
                ("height".into(), Json::UInt(options.height as u64)),
                ("format".into(), Json::string(options.format.to_string())),
                ("tile_size".into(), Json::UInt(options.tile_size as u64)),
                ("workers".into(), Json::UInt(options.workers as u64)),
                ("runs".into(), Json::UInt(options.runs as u64)),
                (
                    "pyramid_levels".into(),
                    Json::UInt(options.pyramid_levels as u64),
                ),
                (
                    "viewport_width".into(),
                    Json::UInt(options.viewport_width as u64),
                ),
                (
                    "viewport_height".into(),
                    Json::UInt(options.viewport_height as u64),
                ),
                ("cache_bytes".into(), Json::UInt(options.cache_bytes)),
                (
                    "queue_capacity".into(),
                    Json::UInt(options.queue_capacity as u64),
                ),
            ]),
        ),
        (
            "source".into(),
            Json::Obj(vec![
                (
                    "path".into(),
                    Json::string(options.source_path.display().to_string()),
                ),
                ("width".into(), Json::UInt(options.width as u64)),
                ("height".into(), Json::UInt(options.height as u64)),
                ("format".into(), Json::string(options.format.to_string())),
                ("canonical_bytes".into(), Json::UInt(canonical_bytes)),
                ("file_bytes".into(), Json::UInt(source_size)),
            ]),
        ),
        (
            "statistics".into(),
            Json::Obj(vec![
                (
                    "method".into(),
                    Json::string(
                        "each case is repeated and the median and the p95 are reported; the \
                         mean and the sample standard deviation are reported beside them",
                    ),
                ),
                (
                    "p95_rule".into(),
                    Json::string(
                        "nearest rank: the smallest sample at or above the 95th percentile, so \
                         with ten samples the p95 is the largest of the ten",
                    ),
                ),
                (
                    "median_rule".into(),
                    Json::string(
                        "the middle sample, or the mean of the two middle samples when the \
                         count is even",
                    ),
                ),
                (
                    "stdev_rule".into(),
                    Json::string("the sample standard deviation, with n - 1 in the denominator"),
                ),
            ]),
        ),
        (
            "memory_scope".into(),
            Json::Obj(vec![
                (
                    "included".into(),
                    Json::string(
                        "the pyramid encode holds the whole original picture and every derived \
                         level at once, because a preview is built from the pixels below it; \
                         the figures quoted for it are the allocator's own high-water mark and \
                         the process's resident-set high-water mark",
                    ),
                ),
                (
                    "excluded".into(),
                    Json::string(
                        "the canonical pixel file on disk, the operating system's page cache, \
                         the memory of other processes, and the benchmark driver's own memory",
                    ),
                ),
                (
                    "default_path".into(),
                    Json::string(
                        "the v0.1 path never materialises the picture: the source is read tile \
                         by tile, so its working set is bounded by the number of tiles in \
                         flight and not by the size of the image",
                    ),
                ),
            ]),
        ),
        (
            "scenarios".into(),
            Json::Obj(vec![
                ("encode_base".into(), base_encode),
                ("encode_pyramid".into(), pyramid_encode),
                ("first_viewport_base".into(), base_viewport),
                ("first_viewport_pyramid".into(), pyramid_viewport),
                ("level_decode".into(), Json::Arr(level_rows)),
                ("full_decode_base".into(), full_decode),
            ]),
        ),
    ]))
}

/// Encode the same picture `runs` times and record what came out.
fn encode_repeatedly(
    options: &PreviewBenchOptions,
    source: &RawSource,
    encode_options: &EncodeOptions,
    parallel: &ParallelOptions,
    path: &Path,
    label: &str,
) -> Result<Json> {
    let mut samples = Vec::with_capacity(options.runs);
    let mut digests = Vec::with_capacity(options.runs);
    let mut file_size = 0u64;
    let mut level_count = 0u32;
    let mut level_tile_counts: Vec<u32> = Vec::new();
    let mut level_payload_bytes: Vec<u64> = Vec::new();
    let mut base_only_file_size = 0u64;
    let mut tile_count = 0u32;

    for _ in 0..options.runs {
        let started = Instant::now();
        let report = encode_to_file(source, encode_options, parallel, path)?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
        digests.push(hash_file(path)?);
        file_size = report.file_size;
        level_count = report.level_count;
        level_tile_counts = report.level_tile_counts.clone();
        level_payload_bytes = report.level_payload_bytes.clone();
        base_only_file_size = report.base_only_file_size();
        tile_count = report.tile_count;
    }

    let digests_agree = digests.windows(2).all(|pair| pair[0] == pair[1]);
    let digest = digests.last().cloned().unwrap_or_default();

    Ok(Json::Obj(vec![
        ("label".into(), Json::string(label)),
        ("runs".into(), Json::UInt(options.runs as u64)),
        ("samples_ms".into(), numbers(&samples)),
        ("stats".into(), stats_json(&stats(&samples))),
        ("file_size_bytes".into(), Json::UInt(file_size)),
        ("tile_count".into(), Json::UInt(tile_count as u64)),
        ("level_count".into(), Json::UInt(level_count as u64)),
        (
            "level_tile_counts".into(),
            Json::Arr(
                level_tile_counts
                    .iter()
                    .map(|&count| Json::UInt(count as u64))
                    .collect(),
            ),
        ),
        (
            "level_payload_bytes".into(),
            Json::Arr(
                level_payload_bytes
                    .iter()
                    .map(|&bytes| Json::UInt(bytes))
                    .collect(),
            ),
        ),
        (
            "base_only_file_size_bytes".into(),
            Json::UInt(base_only_file_size),
        ),
        ("sha256".into(), Json::string(digest)),
        ("all_runs_identical".into(), Json::Bool(digests_agree)),
    ]))
}

/// Open the file and show the first viewport, `runs` times.
fn viewport_repeatedly(
    options: &PreviewBenchOptions,
    path: &Path,
    limits: &Limits,
    label: &str,
) -> Result<Json> {
    let mut open_samples = Vec::with_capacity(options.runs);
    let mut decode_samples = Vec::with_capacity(options.runs);
    let mut total_samples = Vec::with_capacity(options.runs);
    let mut visible_tiles = 0usize;

    for _ in 0..options.runs {
        let (open_ms, decode_ms, total_ms, tiles) = viewport_once(options, path, limits)?;
        open_samples.push(open_ms);
        decode_samples.push(decode_ms);
        total_samples.push(total_ms);
        visible_tiles = tiles;
    }

    Ok(Json::Obj(vec![
        ("label".into(), Json::string(label)),
        ("runs".into(), Json::UInt(options.runs as u64)),
        ("visible_tiles".into(), Json::UInt(visible_tiles as u64)),
        ("open_stats".into(), stats_json(&stats(&open_samples))),
        ("decode_stats".into(), stats_json(&stats(&decode_samples))),
        (
            "time_to_pixels_stats".into(),
            stats_json(&stats(&total_samples)),
        ),
    ]))
}

/// Open the file and decode the tiles of the centre viewport.
fn viewport_once(
    options: &PreviewBenchOptions,
    path: &Path,
    limits: &Limits,
) -> Result<(f64, f64, f64, usize)> {
    let started = Instant::now();
    let reader = TileReader::open(path, limits)?;
    let partition = reader.meta().partition().clone();
    let factory = FileDecoderFactory::from_reader(reader);
    let pool = Pool::new(
        &factory,
        options.workers,
        options.cache_bytes,
        options.queue_capacity,
    )?;

    let rect = centre_viewport(options)?;
    let viewport = Viewport::new(&partition, rect)?;
    let policy = PrefetchPolicy {
        radius: 0,
        direction_reach: 0,
        direction_bonus: 0,
    };
    let plan = viewport.plan(&policy, Direction::None);
    let opened = Instant::now();

    plan.submit(&pool, pool.generation());
    for entry in &plan.visible {
        pool.wait_for_tile(entry.tile, std::time::Duration::from_secs(120))?;
    }
    let done = Instant::now();

    let open_ms = opened.duration_since(started).as_secs_f64() * 1000.0;
    let decode_ms = done.duration_since(opened).as_secs_f64() * 1000.0;
    let total_ms = done.duration_since(started).as_secs_f64() * 1000.0;
    Ok((open_ms, decode_ms, total_ms, plan.visible.len()))
}

/// The viewport the benchmark opens on: the centre of the image.
fn centre_viewport(options: &PreviewBenchOptions) -> Result<Rect> {
    let width = options.viewport_width.min(options.width);
    let height = options.viewport_height.min(options.height);
    Rect::new(
        (options.width - width) / 2,
        (options.height - height) / 2,
        width,
        height,
    )
}

/// Decode every level of the pyramid, one level at a time.
fn level_decode_rows(
    options: &PreviewBenchOptions,
    path: &Path,
    limits: &Limits,
) -> Result<Vec<Json>> {
    let reader = TileReader::open(path, limits)?;
    let levels: Vec<(u16, u32, u32, bool, usize)> = reader
        .meta()
        .levels()
        .iter()
        .map(|level| {
            let dims = level.dimensions();
            (
                level.index(),
                dims.width,
                dims.height,
                level.is_derived(),
                level.tile_count(),
            )
        })
        .collect();

    let mut rows = Vec::with_capacity(levels.len());
    for (index, width, height, derived, tile_count) in levels {
        let mut samples = Vec::with_capacity(options.runs);
        let mut bytes_decoded = 0u64;
        for _ in 0..options.runs {
            let (elapsed_ms, bytes) = decode_level_once(path, limits, index as usize)?;
            samples.push(elapsed_ms);
            bytes_decoded = bytes;
        }

        rows.push(Json::Obj(vec![
            ("level".into(), Json::UInt(index as u64)),
            ("derived".into(), Json::Bool(derived)),
            ("width".into(), Json::UInt(width as u64)),
            ("height".into(), Json::UInt(height as u64)),
            ("tiles".into(), Json::UInt(tile_count as u64)),
            ("runs".into(), Json::UInt(options.runs as u64)),
            ("samples_ms".into(), numbers(&samples)),
            ("stats".into(), stats_json(&stats(&samples))),
            ("pixel_bytes".into(), Json::UInt(bytes_decoded)),
        ]));
    }
    Ok(rows)
}

/// Decode every tile of one level and drop the pixels as they are decoded.
fn decode_level_once(path: &Path, limits: &Limits, level_index: usize) -> Result<(f64, u64)> {
    let mut reader = TileReader::open(path, limits)?;
    let tile_count = reader
        .meta()
        .level(level_index)
        .ok_or_else(|| Error::invalid(format!("the file has no level {level_index}")))?
        .tile_count();

    let started = Instant::now();
    let mut bytes = 0u64;
    for position in 0..tile_count {
        let index = reader
            .meta()
            .level(level_index)
            .and_then(|level| level.entry_index(position))
            .ok_or_else(|| {
                Error::internal(format!("level {level_index} has no entry {position}"))
            })?;
        let pixels = reader.decode_tile(index, limits)?;
        bytes += pixels.len() as u64;
    }
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((elapsed_ms, bytes))
}

/// Decode the whole v0.1 file, for the like-for-like comparison with the
/// smallest preview.
fn full_decode_row(options: &PreviewBenchOptions, path: &Path, limits: &Limits) -> Result<Json> {
    let mut samples = Vec::with_capacity(options.runs);
    let mut bytes_decoded = 0u64;
    for _ in 0..options.runs {
        let (elapsed_ms, bytes) = decode_level_once(path, limits, 0)?;
        samples.push(elapsed_ms);
        bytes_decoded = bytes;
    }

    Ok(Json::Obj(vec![
        ("label".into(), Json::string("v0.1 whole picture")),
        ("runs".into(), Json::UInt(options.runs as u64)),
        ("samples_ms".into(), numbers(&samples)),
        ("stats".into(), stats_json(&stats(&samples))),
        ("pixel_bytes".into(), Json::UInt(bytes_decoded)),
    ]))
}
