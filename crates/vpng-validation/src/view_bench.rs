//! The interactive-viewing benchmark scenarios.
//!
//! Each scenario answers one of the P5 acceptance questions with a
//! measurement rather than an assertion:
//!
//! * the first viewport: how long it takes to open a huge file and decode
//!   the tiles the user is looking at, and how much memory that took;
//! * the pan: how long a new viewport takes after the user moves, and how
//!   much of the old viewport's work was cancelled;
//! * the prefetch: whether prefetching the likely next tiles actually
//!   helps, measured against the same run without it;
//! * the localized edit: that saving a small edit still rewrites only the
//!   tiles it touches, and that the pixels come back exactly.
//!
//! # The rules
//!
//! * **Every figure has a baseline.** The sequential path — one tile at a
//!   time, no cache, no threads — is measured in the same run, on the same
//!   host, against the same file. A latency number without that baseline
//!   means nothing, so there is always one.
//! * **The memory figures say what they include.** The report states the
//!   included and excluded memory next to every number.
//! * **Nothing is claimed that was not measured.** If a case cannot run
//!   on this host, the report says so and the gate is BLOCKED, not PASS.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vpng_core::{Error, Limits, PixelFormat, Rect};
use vpng_editing::TileSource;
use vpng_source::TileReader;
use vpng_view::{Direction, FileDecoderFactory, Pool, PrefetchPolicy, Viewport};

use crate::json::Json;

/// The options the benchmark runs under.
#[derive(Debug, Clone)]
pub struct ViewBenchOptions {
    /// The directory the benchmark works in.
    pub work_dir: PathBuf,
    /// The width of the test image.
    pub width: u32,
    /// The height of the test image.
    pub height: u32,
    /// The tile size.
    pub tile_size: u32,
    /// The pixel format.
    pub format: PixelFormat,
    /// The number of worker threads.
    pub workers: usize,
    /// The width of the viewport, in pixels.
    pub viewport_width: u32,
    /// The height of the viewport, in pixels.
    pub viewport_height: u32,
    /// The prefetch radius, in tiles.
    pub prefetch_radius: u32,
    /// How many tiles ahead of the viewport the direction preference
    /// reaches.
    pub direction_reach: u32,
    /// How much closer a tile in the direction of travel is treated.
    pub direction_bonus: u32,
    /// The number of pans in the pan scenario.
    pub pans: u32,
    /// How far the viewport moves on each pan, in pixels.
    pub pan_step: u32,
    /// The viewing cache's byte budget.
    pub cache_bytes: u64,
    /// The request queue's depth.
    pub queue_capacity: usize,
    /// Whether to reuse an existing file instead of writing a new one.
    pub reuse_file: bool,
}

impl ViewBenchOptions {
    /// A default set of options for a given directory.
    pub fn default_with(work_dir: PathBuf) -> Self {
        ViewBenchOptions {
            work_dir,
            width: 16384,
            height: 16384,
            tile_size: 256,
            format: PixelFormat::rgba8(),
            workers: 8,
            viewport_width: 1024,
            viewport_height: 1024,
            prefetch_radius: 2,
            direction_reach: 2,
            direction_bonus: 4,
            pans: 12,
            pan_step: 512,
            cache_bytes: 32 * 1024 * 1024,
            queue_capacity: 256,
            reuse_file: false,
        }
    }

    /// The path of the file the benchmark uses.
    pub fn file_path(&self) -> PathBuf {
        self.work_dir.join(format!(
            "view-{width}x{height}-{format}.vpng",
            width = self.width,
            height = self.height,
            format = self.format,
        ))
    }

    /// The policy the prefetch uses.
    pub fn policy(&self) -> PrefetchPolicy {
        PrefetchPolicy {
            radius: self.prefetch_radius,
            direction_reach: self.direction_reach,
            direction_bonus: self.direction_bonus,
        }
    }
}

/// The memory scope the report states.
fn memory_scope() -> Json {
    Json::Obj(vec![
        (
            "included".into(),
            Json::string(
                "the process's resident set (working set) and the allocator's tracked heap \
                 during the measured phase, including the decoded tiles held in the viewing \
                 cache and the payload buffers in flight",
            ),
        ),
        (
            "excluded".into(),
            Json::string(
                "the file on disk, the operating system's file cache, the memory of other \
                 processes, and the memory of the benchmark driver itself",
            ),
        ),
    ])
}

/// Run every scenario and return the report.
pub fn run(options: &ViewBenchOptions) -> Result<Json, Error> {
    let limits = Limits::default();
    fs::create_dir_all(&options.work_dir)?;

    let path = options.file_path();
    let file = prepare_file(options, &path, &limits)?;

    let mut scenarios = vec![(
        "first_viewport".into(),
        first_viewport(options, &path, &limits)?,
    )];

    scenarios.push((
        "sequential_baseline".into(),
        sequential_baseline(options, &path, &limits)?,
    ));
    scenarios.push((
        "pan_without_prefetch".into(),
        pan_scenario(options, &path, &limits, 0)?,
    ));
    scenarios.push((
        "pan_with_prefetch".into(),
        pan_scenario(options, &path, &limits, options.prefetch_radius)?,
    ));
    scenarios.push((
        "prefetch_comparison".into(),
        prefetch_comparison(options, &path, &limits)?,
    ));
    scenarios.push((
        "edit_and_save".into(),
        edit_and_save(options, &path, &limits)?,
    ));

    Ok(Json::Obj(vec![
        ("tool".into(), Json::string("vpng-view-bench")),
        (
            "parameters".into(),
            Json::Obj(vec![
                ("width".into(), Json::UInt(options.width as u64)),
                ("height".into(), Json::UInt(options.height as u64)),
                ("format".into(), Json::string(options.format.to_string())),
                ("tile_size".into(), Json::UInt(options.tile_size as u64)),
                ("workers".into(), Json::UInt(options.workers as u64)),
                (
                    "viewport_width".into(),
                    Json::UInt(options.viewport_width as u64),
                ),
                (
                    "viewport_height".into(),
                    Json::UInt(options.viewport_height as u64),
                ),
                (
                    "prefetch_radius".into(),
                    Json::UInt(options.prefetch_radius as u64),
                ),
                (
                    "direction_reach".into(),
                    Json::UInt(options.direction_reach as u64),
                ),
                (
                    "direction_bonus".into(),
                    Json::UInt(options.direction_bonus as u64),
                ),
                ("pans".into(), Json::UInt(options.pans as u64)),
                ("pan_step".into(), Json::UInt(options.pan_step as u64)),
                ("cache_bytes".into(), Json::UInt(options.cache_bytes)),
                (
                    "queue_capacity".into(),
                    Json::UInt(options.queue_capacity as u64),
                ),
            ]),
        ),
        (
            "file".into(),
            Json::Obj(vec![
                ("path".into(), Json::string(path.display().to_string())),
                ("file_size_bytes".into(), Json::UInt(file.size_bytes)),
                ("tile_count".into(), Json::UInt(file.tile_count as u64)),
                (
                    "canonical_pixel_bytes".into(),
                    Json::UInt(options.format.frame_bytes(options.width, options.height)?),
                ),
                ("write_ms".into(), Json::UInt(file.write_ms)),
                ("reused".into(), Json::Bool(file.reused)),
            ]),
        ),
        ("memory_scope".into(), memory_scope()),
        ("scenarios".into(), Json::Obj(scenarios)),
    ]))
}

/// What is known about the file the benchmark uses.
struct FileFacts {
    size_bytes: u64,
    tile_count: u32,
    write_ms: u64,
    reused: bool,
}

/// Write the file if it is not already there.
fn prepare_file(
    options: &ViewBenchOptions,
    path: &Path,
    limits: &Limits,
) -> Result<FileFacts, Error> {
    if options.reuse_file && path.exists() {
        let reader = TileReader::open(path, limits)?;
        return Ok(FileFacts {
            size_bytes: fs::metadata(path)?.len(),
            tile_count: reader.meta().tile_count(),
            write_ms: 0,
            reused: true,
        });
    }

    let started = Instant::now();
    let report = crate::stream::write_streaming(
        path,
        options.width,
        options.height,
        options.format,
        options.tile_size,
        limits,
        None,
    )?;
    Ok(FileFacts {
        size_bytes: report.file_size,
        tile_count: report.tile_count,
        write_ms: started.elapsed().as_millis() as u64,
        reused: false,
    })
}

/// The viewport the benchmark opens on: the centre of the image.
fn centre_viewport(options: &ViewBenchOptions) -> Result<Rect, Error> {
    let width = options.viewport_width.min(options.width);
    let height = options.viewport_height.min(options.height);
    Rect::new(
        (options.width - width) / 2,
        (options.height - height) / 2,
        width,
        height,
    )
}

/// The rectangles the pan scenario visits.
///
/// The sweep moves to the right, one step at a time, starting at the left
/// edge, and stops before the edge of the image. It never wraps around,
/// because a wrap would make the cache hit rate meaningless: the tiles
/// would already be resident.
fn pan_rects(options: &ViewBenchOptions) -> Result<Vec<(Rect, Direction)>, Error> {
    let width = options.viewport_width.min(options.width);
    let height = options.viewport_height.min(options.height);
    let step = options.pan_step.max(1);

    let mut rects = Vec::new();
    for index in 0..options.pans {
        let x = index
            .saturating_mul(step)
            .min(options.width.saturating_sub(width));
        let y = (options.height - height) / 2;
        let rect = Rect::new(x, y, width, height)?;
        let direction = if index == 0 {
            Direction::None
        } else {
            Direction::Right
        };
        rects.push((rect, direction));
    }
    Ok(rects)
}

/// The first viewport: open the file and decode what the user is looking
/// at.
fn first_viewport(options: &ViewBenchOptions, path: &Path, limits: &Limits) -> Result<Json, Error> {
    let baseline = crate::mem::sample();
    crate::mem::reset_peaks();

    // Open the file: the header and the directory are parsed; no payload
    // is read.
    let started = Instant::now();
    let reader = TileReader::open(path, limits)?;
    let open_ms = started.elapsed().as_secs_f64() * 1000.0;
    let grid = *reader.meta().grid();
    let tile_count = reader.meta().tile_count();

    let factory = FileDecoderFactory::from_reader(reader);
    let pool = Pool::new(
        &factory,
        options.workers,
        options.cache_bytes,
        options.queue_capacity,
    )?;

    let rect = centre_viewport(options)?;
    let viewport = Viewport::new(&grid, rect)?;
    let policy = PrefetchPolicy {
        radius: 0,
        direction_reach: 0,
        direction_bonus: 0,
    };
    let plan = viewport.plan(&policy, Direction::None);

    // The first viewport: submit the visible tiles and wait for them.
    let started = Instant::now();
    let report = plan.submit(&pool, pool.generation());
    for entry in &plan.visible {
        pool.wait_for_tile(entry.tile, Duration::from_secs(120))?;
    }
    let first_viewport_ms = started.elapsed().as_secs_f64() * 1000.0;

    let after = crate::mem::sample();
    let stats = pool.stats();
    let cache = stats.cache;

    // The pixels the pool produced must be the pixels the ordinary
    // decoder produces. The same tiles are decoded again, one at a time,
    // with a reader of their own, and compared byte for byte.
    let mut verifier = TileReader::open(path, limits)?;
    let mut identical = 0u64;
    let mut different = 0u64;
    for entry in &plan.visible {
        let sequential = verifier.decode_tile(entry.tile, limits)?;
        let parallel = pool.tile(entry.tile);
        match parallel {
            Some(parallel) if parallel == sequential => identical += 1,
            _ => different += 1,
        }
    }

    Ok(Json::Obj(vec![
        (
            "open".into(),
            Json::Obj(vec![
                ("elapsed_ms".into(), Json::Num(open_ms)),
                ("tile_count".into(), Json::UInt(tile_count as u64)),
            ]),
        ),
        (
            "viewport".into(),
            Json::Obj(vec![
                ("x".into(), Json::UInt(rect.x as u64)),
                ("y".into(), Json::UInt(rect.y as u64)),
                ("width".into(), Json::UInt(rect.width as u64)),
                ("height".into(), Json::UInt(rect.height as u64)),
                (
                    "visible_tiles".into(),
                    Json::UInt(plan.visible.len() as u64),
                ),
                (
                    "prefetch_tiles".into(),
                    Json::UInt(plan.prefetch.len() as u64),
                ),
            ]),
        ),
        ("first_viewport_ms".into(), Json::Num(first_viewport_ms)),
        (
            "submitted_visible".into(),
            Json::UInt(report.visible_queued),
        ),
        ("tiles_decoded".into(), Json::UInt(stats.completed)),
        (
            "only_viewport_tiles_decoded".into(),
            Json::Bool(stats.completed as usize == plan.visible.len()),
        ),
        (
            "pixel_identity".into(),
            Json::Obj(vec![
                (
                    "tiles_compared".into(),
                    Json::UInt(plan.visible.len() as u64),
                ),
                ("identical_to_sequential".into(), Json::UInt(identical)),
                ("different_from_sequential".into(), Json::UInt(different)),
                (
                    "pixels_identical_to_sequential".into(),
                    Json::Bool(different == 0),
                ),
            ]),
        ),
        ("bytes_read".into(), Json::UInt(stats.bytes_read)),
        (
            "scheduler".into(),
            Json::Obj(vec![
                ("submitted".into(), Json::UInt(stats.scheduler.submitted)),
                ("accepted".into(), Json::UInt(stats.scheduler.accepted)),
                ("dispatched".into(), Json::UInt(stats.scheduler.dispatched)),
                ("cancelled".into(), Json::UInt(stats.scheduler.cancelled)),
                (
                    "discarded_stale".into(),
                    Json::UInt(stats.scheduler.discarded_stale),
                ),
            ]),
        ),
        (
            "cache".into(),
            Json::Obj(vec![
                ("hits".into(), Json::UInt(cache.hits)),
                ("misses".into(), Json::UInt(cache.misses)),
                ("evictions".into(), Json::UInt(cache.evictions)),
                ("decoded_tiles".into(), Json::UInt(cache.decoded_tiles)),
                ("resident_tiles".into(), Json::UInt(cache.resident_tiles)),
                ("current_bytes".into(), Json::UInt(cache.current_bytes)),
                ("peak_bytes".into(), Json::UInt(cache.peak_bytes)),
                ("budget_bytes".into(), Json::UInt(options.cache_bytes)),
                (
                    "within_budget".into(),
                    Json::Bool(cache.peak_bytes <= options.cache_bytes),
                ),
            ]),
        ),
        (
            "memory".into(),
            Json::Obj(vec![
                (
                    "heap_peak_bytes".into(),
                    Json::UInt(after.heap_peak_bytes as u64),
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
                            .saturating_sub(baseline.working_set_bytes),
                    ),
                ),
                (
                    "heap_delta_bytes".into(),
                    Json::UInt(
                        after
                            .heap_peak_bytes
                            .saturating_sub(baseline.heap_peak_bytes)
                            as u64,
                    ),
                ),
            ]),
        ),
    ]))
}

/// The sequential baseline: the same viewport, decoded one tile at a time
/// with no cache and no threads.
fn sequential_baseline(
    options: &ViewBenchOptions,
    path: &Path,
    limits: &Limits,
) -> Result<Json, Error> {
    let baseline = crate::mem::sample();
    crate::mem::reset_peaks();

    let mut reader = TileReader::open(path, limits)?;
    let grid = *reader.meta().grid();
    let rect = centre_viewport(options)?;
    let viewport = Viewport::new(&grid, rect)?;
    let plan = viewport.plan(
        &PrefetchPolicy {
            radius: 0,
            direction_reach: 0,
            direction_bonus: 0,
        },
        Direction::None,
    );

    let started = Instant::now();
    let mut decoded = 0u64;
    let mut bytes = 0u64;
    for entry in &plan.visible {
        let pixels = reader.decode_tile(entry.tile, limits)?;
        decoded += 1;
        bytes += pixels.len() as u64;
        // The pixels are dropped as soon as they have been counted, which
        // is what the sequential path does: it never keeps them.
        drop(pixels);
    }
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

    let after = crate::mem::sample();

    Ok(Json::Obj(vec![
        ("elapsed_ms".into(), Json::Num(elapsed_ms)),
        ("tiles_decoded".into(), Json::UInt(decoded)),
        ("pixel_bytes".into(), Json::UInt(bytes)),
        (
            "bytes_read_from_file".into(),
            Json::UInt(reader.bytes_read()),
        ),
        ("payloads_read".into(), Json::UInt(reader.payloads_read())),
        (
            "memory".into(),
            Json::Obj(vec![
                (
                    "heap_peak_bytes".into(),
                    Json::UInt(after.heap_peak_bytes as u64),
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
                            .saturating_sub(baseline.working_set_bytes),
                    ),
                ),
            ]),
        ),
    ]))
}

/// The pan scenario: move the viewport and measure what it costs.
fn pan_scenario(
    options: &ViewBenchOptions,
    path: &Path,
    limits: &Limits,
    prefetch_radius: u32,
) -> Result<Json, Error> {
    let baseline = crate::mem::sample();
    crate::mem::reset_peaks();

    let reader = TileReader::open(path, limits)?;
    let grid = *reader.meta().grid();
    let factory = FileDecoderFactory::from_reader(reader);
    let pool = Pool::new(
        &factory,
        options.workers,
        options.cache_bytes,
        options.queue_capacity,
    )?;

    let policy = PrefetchPolicy {
        radius: prefetch_radius,
        direction_reach: options.direction_reach,
        direction_bonus: options.direction_bonus,
    };

    let rects = pan_rects(options)?;
    let mut latencies = Vec::with_capacity(rects.len());
    let mut visible_tiles = 0u64;

    for (index, (rect, direction)) in rects.iter().enumerate() {
        let viewport = Viewport::new(&grid, *rect)?;
        let plan = viewport.plan(&policy, *direction);

        // A pan: the generation advances, and everything queued for the
        // old viewport is dropped.
        let generation = if index == 0 {
            pool.generation()
        } else {
            pool.advance_generation()
        };
        plan.submit(&pool, generation);

        let started = Instant::now();
        for entry in &plan.visible {
            pool.wait_for_tile(entry.tile, Duration::from_secs(120))?;
        }
        latencies.push(started.elapsed().as_secs_f64() * 1000.0);
        visible_tiles += plan.visible.len() as u64;
    }

    let after = crate::mem::sample();
    let stats = pool.stats();
    let cache = stats.cache;
    let total: f64 = latencies.iter().sum();
    let mean = if latencies.is_empty() {
        0.0
    } else {
        total / latencies.len() as f64
    };
    let max = latencies.iter().copied().fold(0.0f64, f64::max);
    let cancellation_rate = if stats.scheduler.submitted == 0 {
        0.0
    } else {
        stats.scheduler.cancelled as f64 / stats.scheduler.submitted as f64
    };

    Ok(Json::Obj(vec![
        ("prefetch_radius".into(), Json::UInt(prefetch_radius as u64)),
        ("pans".into(), Json::UInt(latencies.len() as u64)),
        ("visible_tiles_requested".into(), Json::UInt(visible_tiles)),
        ("total_ms".into(), Json::Num(total)),
        ("mean_ms".into(), Json::Num(mean)),
        ("max_ms".into(), Json::Num(max)),
        (
            "latencies_ms".into(),
            Json::Arr(latencies.iter().map(|ms| Json::Num(*ms)).collect()),
        ),
        ("tiles_decoded".into(), Json::UInt(stats.completed)),
        ("bytes_read".into(), Json::UInt(stats.bytes_read)),
        (
            "scheduler".into(),
            Json::Obj(vec![
                ("submitted".into(), Json::UInt(stats.scheduler.submitted)),
                ("accepted".into(), Json::UInt(stats.scheduler.accepted)),
                ("dispatched".into(), Json::UInt(stats.scheduler.dispatched)),
                ("cancelled".into(), Json::UInt(stats.scheduler.cancelled)),
                (
                    "discarded_stale".into(),
                    Json::UInt(stats.scheduler.discarded_stale),
                ),
                (
                    "rejected_full".into(),
                    Json::UInt(stats.scheduler.rejected_full),
                ),
                ("cancellation_rate".into(), Json::Num(cancellation_rate)),
            ]),
        ),
        ("discarded_results".into(), Json::UInt(stats.discarded)),
        ("recovered_results".into(), Json::UInt(stats.recovered)),
        (
            "cache".into(),
            Json::Obj(vec![
                ("hits".into(), Json::UInt(cache.hits)),
                ("misses".into(), Json::UInt(cache.misses)),
                ("evictions".into(), Json::UInt(cache.evictions)),
                ("decoded_tiles".into(), Json::UInt(cache.decoded_tiles)),
                ("peak_bytes".into(), Json::UInt(cache.peak_bytes)),
                (
                    "hit_rate".into(),
                    match cache.hit_rate() {
                        Some(rate) => Json::Num(rate),
                        None => Json::Null,
                    },
                ),
                (
                    "visible_hit_rate".into(),
                    match stats.visible_hit_rate() {
                        Some(rate) => Json::Num(rate),
                        None => Json::Null,
                    },
                ),
                (
                    "speculative_hit_rate".into(),
                    match stats.speculative_hit_rate() {
                        Some(rate) => Json::Num(rate),
                        None => Json::Null,
                    },
                ),
                ("visible_hits".into(), Json::UInt(stats.visible_hits)),
                ("visible_misses".into(), Json::UInt(stats.visible_misses)),
            ]),
        ),
        (
            "memory".into(),
            Json::Obj(vec![
                (
                    "heap_peak_bytes".into(),
                    Json::UInt(after.heap_peak_bytes as u64),
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
                            .saturating_sub(baseline.working_set_bytes),
                    ),
                ),
            ]),
        ),
    ]))
}

/// The prefetch comparison: the same pan, with several prefetch radii.
///
/// The policy's parameters are not a constant: the ring's width decides
/// how much work is done speculatively, and the right width depends on the
/// tile size, the viewport and how fast the user pans. So the benchmark
/// measures several radii and reports them all, including the one that
/// lost.
fn prefetch_comparison(
    options: &ViewBenchOptions,
    path: &Path,
    limits: &Limits,
) -> Result<Json, Error> {
    let mut radii = vec![0u32];
    for radius in 1..=options.prefetch_radius.max(1) {
        radii.push(radius);
    }
    if options.prefetch_radius > 1 {
        radii.push(options.prefetch_radius + 1);
    }

    let mut runs = Vec::new();
    let mut baseline_total: Option<f64> = None;
    let mut best: Option<(u32, f64)> = None;
    let mut helped = false;

    for radius in &radii {
        let run = pan_scenario(options, path, limits, *radius)?;
        let total = number(&run, "total_ms").unwrap_or(0.0);
        let visible_hit_rate = optional_number(&run, "cache", "visible_hit_rate");
        let overall_hit_rate = optional_number(&run, "cache", "hit_rate");

        if *radius == 0 {
            baseline_total = Some(total);
        } else if let Some(baseline) = baseline_total {
            if total < baseline {
                helped = true;
            }
        }
        if best.map_or(true, |(_, best_total)| total < best_total) {
            best = Some((*radius, total));
        }

        runs.push((
            radius.to_string(),
            Json::Obj(vec![
                ("prefetch_radius".into(), Json::UInt(*radius as u64)),
                ("total_ms".into(), Json::Num(total)),
                (
                    "mean_ms".into(),
                    Json::Num(number(&run, "mean_ms").unwrap_or(0.0)),
                ),
                (
                    "max_ms".into(),
                    Json::Num(number(&run, "max_ms").unwrap_or(0.0)),
                ),
                (
                    "tiles_decoded".into(),
                    Json::UInt(number(&run, "tiles_decoded").unwrap_or(0.0) as u64),
                ),
                (
                    "cancelled".into(),
                    Json::UInt(
                        optional_number(&run, "scheduler", "cancelled").unwrap_or(0.0) as u64,
                    ),
                ),
                (
                    "visible_hit_rate".into(),
                    match visible_hit_rate {
                        Some(rate) => Json::Num(rate),
                        None => Json::Null,
                    },
                ),
                (
                    "overall_hit_rate".into(),
                    match overall_hit_rate {
                        Some(rate) => Json::Num(rate),
                        None => Json::Null,
                    },
                ),
                ("run".into(), run),
            ]),
        ));
    }

    Ok(Json::Obj(vec![
        (
            "radii".into(),
            Json::Arr(radii.iter().map(|r| Json::UInt(*r as u64)).collect()),
        ),
        ("runs".into(), Json::Obj(runs)),
        ("prefetch_reduced_total_latency".into(), Json::Bool(helped)),
        (
            "best_radius".into(),
            match best {
                Some((radius, _)) => Json::UInt(radius as u64),
                None => Json::Null,
            },
        ),
        (
            "note".into(),
            Json::string(
                "each radius is the same pan sequence over the same file in the same process; \
                 the visible hit rate counts only the tiles the user was looking at, because a \
                 prefetch that misses is a guess that has not come true, not a cache failure",
            ),
        ),
    ]))
}

/// The localized edit: read a small region, write it back, save, reopen.
fn edit_and_save(options: &ViewBenchOptions, path: &Path, limits: &Limits) -> Result<Json, Error> {
    use vpng_editing::{CommitOptions, FileEditor};

    let before_payloads = payload_digests(path, limits)?;

    let mut editor = FileEditor::open(path, limits)?
        .with_cache_bytes(options.cache_bytes)?
        .with_history_bytes(16 * 1024 * 1024)?;

    let width = 64.min(options.width);
    let height = 64.min(options.height);
    let rect = Rect::new(
        (options.width / 2).min(options.width - width),
        (options.height / 2).min(options.height - height),
        width,
        height,
    )?;

    let started = Instant::now();
    let region = editor.editor().read_region(rect)?;
    let read_ms = started.elapsed().as_secs_f64() * 1000.0;

    let mut patch = region.data().to_vec();
    for byte in patch.iter_mut() {
        *byte = byte.wrapping_add(29);
    }
    editor.editor().write_region(rect, &patch)?;

    let dirty = editor.editor().dirty_tiles();
    let started = Instant::now();
    let commit = editor.editor().commit(path, &CommitOptions::default())?;
    let commit_ms = started.elapsed().as_secs_f64() * 1000.0;

    let after_payloads = payload_digests(path, limits)?;
    let mut identical = 0u64;
    let mut changed = 0u64;
    for (index, digest) in after_payloads.iter().enumerate() {
        if before_payloads.get(index) == Some(digest) {
            identical += 1;
        } else {
            changed += 1;
        }
    }

    // Reopen the saved file and read the region back.
    let mut verifier = FileEditor::open(path, limits)?;
    let read_back = verifier.editor().read_region(rect)?;

    Ok(Json::Obj(vec![
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
        ("commit_ms".into(), Json::Num(commit_ms)),
        ("dirty_tiles".into(), Json::UInt(dirty.len() as u64)),
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
                ("file_size".into(), Json::UInt(commit.file_size)),
            ]),
        ),
        (
            "payload_identity".into(),
            Json::Obj(vec![
                ("identical_tiles".into(), Json::UInt(identical)),
                ("changed_tiles".into(), Json::UInt(changed)),
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
    ]))
}

/// The CRC32C of every tile payload, so two files can be compared tile by
/// tile without holding either file in memory.
fn payload_digests(path: &Path, limits: &Limits) -> Result<Vec<u32>, Error> {
    let mut source = vpng_editing::FileSource::open(path, limits)?;
    let count = source.tile_count() as usize;
    let mut digests = Vec::with_capacity(count);
    for index in 0..count {
        let payload = source.tile_payload(index)?;
        digests.push(vpng_container::crc32c(&payload));
    }
    Ok(digests)
}

/// Read a number out of a report.
fn number(value: &Json, key: &str) -> Option<f64> {
    match value {
        Json::Obj(fields) => {
            fields
                .iter()
                .find(|(name, _)| name == key)
                .and_then(|(_, v)| match v {
                    Json::Num(number) => Some(*number),
                    Json::UInt(number) => Some(*number as f64),
                    _ => None,
                })
        }
        _ => None,
    }
}

/// Read a number out of a nested report.
fn optional_number(value: &Json, outer: &str, inner: &str) -> Option<f64> {
    match value {
        Json::Obj(fields) => fields
            .iter()
            .find(|(name, _)| name == outer)
            .and_then(|(_, v)| number(v, inner)),
        _ => None,
    }
}
