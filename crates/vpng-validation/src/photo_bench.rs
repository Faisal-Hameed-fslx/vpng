//! The multi-run photographic benchmark: a latency matrix with statistics.
//!
//! Every earlier benchmark in this repository measured **one** run and
//! reported the number it got. That is enough to say "this works" and not
//! enough to say "this is faster than that": a single run on a shared
//! machine has a run-to-run variance that is often larger than the effect
//! being measured. This module fixes that.
//!
//! # What it does
//!
//! * **It repeats.** Every case runs five to ten times by default, and the
//!   report gives the **median** and the **p95**, not just a mean.
//! * **It uses a photograph.** The synthetic gradient the other benchmarks
//!   use compresses trivially and decodes trivially. A real photograph is
//!   what the engine will actually meet, and its cost profile is different.
//! * **It sweeps a matrix.** The tile size and the worker count are both
//!   varied, because the interesting question is not "how fast is this
//!   configuration" but "how does the configuration change the answer".
//! * **It keeps a baseline.** The sequential decoder is measured in the
//!   same run, on the same file, so every parallel figure has something to
//!   be compared against.
//!
//! # The rules
//!
//! * **The percentiles are defined.** The p95 is the *nearest-rank*
//!   percentile: with ten samples it is the largest of the ten. The median
//!   of an even number of samples is the mean of the two middle ones. Both
//!   rules are recorded in the report next to the numbers.
//! * **The bytes are checked.** Every encode records the SHA-256 of the
//!   file it wrote. If the tile size or the worker count changed a single
//!   byte, the digests would differ and the report would say so.
//! * **Nothing is claimed that was not measured.** If a case cannot run on
//!   this host, the gate is BLOCKED, not PASS.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use vpng_core::{Error, Limits, PixelFormat, Rect, Result};
use vpng_encoder::parallel::ParallelOptions;
use vpng_encoder::source::PixelSource;
use vpng_encoder::stream::encode_to_file;
use vpng_encoder::EncodeOptions;
use vpng_source::TileReader;
use vpng_tiling::TileRect;
use vpng_view::{Direction, FileDecoderFactory, Pool, PrefetchPolicy, Viewport};

use crate::json::Json;

/// The summary of a set of latency samples.
#[derive(Debug, Clone, Copy)]
pub struct Stats {
    /// How many samples the summary is over.
    pub count: usize,
    /// The smallest sample.
    pub min: f64,
    /// The largest sample.
    pub max: f64,
    /// The arithmetic mean.
    pub mean: f64,
    /// The median: the middle sample, or the mean of the two middle ones.
    pub median: f64,
    /// The 95th percentile, by the nearest-rank rule.
    pub p95: f64,
    /// The sample standard deviation.
    pub stdev: f64,
}

/// The options the benchmark runs under.
#[derive(Debug, Clone)]
pub struct PhotoBenchOptions {
    /// The raw canonical pixel file the photograph was converted to.
    pub source_path: PathBuf,
    /// The image's width in pixels.
    pub width: u32,
    /// The image's height in pixels.
    pub height: u32,
    /// The pixel format.
    pub format: PixelFormat,
    /// The tile sizes to sweep.
    pub tile_sizes: Vec<u32>,
    /// The worker counts to sweep.
    pub worker_counts: Vec<usize>,
    /// How many times each case is repeated.
    pub runs: usize,
    /// The width of the viewport, in pixels.
    pub viewport_width: u32,
    /// The height of the viewport, in pixels.
    pub viewport_height: u32,
    /// How many tiles one work item covers during the encode.
    pub chunk_tiles: usize,
    /// The viewing cache's byte budget.
    pub cache_bytes: u64,
    /// The request queue's depth.
    pub queue_capacity: usize,
    /// Where the benchmark's files are written.
    pub work_dir: PathBuf,
    /// Keep the files after the run.
    pub keep_file: bool,
}

impl PhotoBenchOptions {
    /// A default set of options for a given directory.
    pub fn default_with(work_dir: PathBuf) -> Self {
        PhotoBenchOptions {
            source_path: PathBuf::new(),
            width: 15360,
            height: 8640,
            format: PixelFormat::rgb8(),
            tile_sizes: vec![256, 512, 1024],
            worker_counts: vec![1, 2, 4, 8],
            runs: 10,
            viewport_width: 1024,
            viewport_height: 1024,
            chunk_tiles: 4,
            cache_bytes: 32 * 1024 * 1024,
            queue_capacity: 256,
            work_dir,
            keep_file: false,
        }
    }

    /// The path the benchmark's file is written to for one tile size.
    pub fn file_path(&self, tile_size: u32) -> PathBuf {
        self.work_dir.join(format!(
            "photo-{width}x{height}-{format}-t{tile}.vpng",
            width = self.width,
            height = self.height,
            format = self.format,
            tile = tile_size,
        ))
    }
}

/// Run every scenario and return the report.
pub fn run(options: &PhotoBenchOptions) -> Result<Json> {
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

    let mut encode_rows = Vec::new();
    let mut compression_rows = Vec::new();
    let mut viewport_rows = Vec::new();
    let mut pan_rows = Vec::new();
    let mut baseline_rows = Vec::new();

    for &tile_size in &options.tile_sizes {
        let path = options.file_path(tile_size);
        let encode_options = EncodeOptions::default()
            .with_tile_size(tile_size, tile_size)
            .with_limits(limits);

        // ---- the encode matrix ------------------------------------------
        let mut tile_digests: Vec<String> = Vec::new();
        let mut file_size = 0u64;
        let mut tile_count = 0u64;
        let mut payload_bytes = 0u64;

        for &workers in &options.worker_counts {
            let mut samples = Vec::with_capacity(options.runs);
            for _ in 0..options.runs {
                let started = Instant::now();
                let report = encode_to_file(
                    &source,
                    &encode_options,
                    &ParallelOptions::default()
                        .with_workers(workers)
                        .with_chunk_tiles(options.chunk_tiles),
                    &path,
                )?;
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
                tile_digests.push(hash_file(&path)?);
                file_size = report.file_size;
                tile_count = report.tile_count as u64;
                payload_bytes = report.payload_bytes;
            }

            let digest = tile_digests.last().cloned().unwrap_or_default();
            encode_rows.push(Json::Obj(vec![
                ("tile_size".into(), Json::UInt(tile_size as u64)),
                ("workers".into(), Json::UInt(workers as u64)),
                ("runs".into(), Json::UInt(options.runs as u64)),
                ("samples_ms".into(), numbers(&samples)),
                ("stats".into(), stats_json(&stats(&samples))),
                ("file_size_bytes".into(), Json::UInt(file_size)),
                ("sha256".into(), Json::string(digest)),
            ]));
        }

        let digests_agree = tile_digests.windows(2).all(|pair| pair[0] == pair[1]);
        let tile_digest = tile_digests.first().cloned().unwrap_or_default();

        compression_rows.push(Json::Obj(vec![
            ("tile_size".into(), Json::UInt(tile_size as u64)),
            ("tiles".into(), Json::UInt(tile_count)),
            ("canonical_bytes".into(), Json::UInt(canonical_bytes)),
            ("file_size_bytes".into(), Json::UInt(file_size)),
            ("payload_bytes".into(), Json::UInt(payload_bytes)),
            (
                "ratio".into(),
                Json::Num(canonical_bytes as f64 / file_size.max(1) as f64),
            ),
            ("sha256".into(), Json::string(tile_digest.clone())),
            ("all_runs_identical".into(), Json::Bool(digests_agree)),
        ]));

        // ---- the viewing matrix ----------------------------------------
        for &workers in &options.worker_counts {
            let mut open_samples = Vec::with_capacity(options.runs);
            let mut decode_samples = Vec::with_capacity(options.runs);
            let mut total_samples = Vec::with_capacity(options.runs);
            let mut visible_tiles = 0usize;

            for _ in 0..options.runs {
                let (open_ms, decode_ms, total_ms, tiles) =
                    first_viewport_once(options, &path, &limits, workers)?;
                open_samples.push(open_ms);
                decode_samples.push(decode_ms);
                total_samples.push(total_ms);
                visible_tiles = tiles;
            }

            viewport_rows.push(Json::Obj(vec![
                ("tile_size".into(), Json::UInt(tile_size as u64)),
                ("workers".into(), Json::UInt(workers as u64)),
                ("runs".into(), Json::UInt(options.runs as u64)),
                ("visible_tiles".into(), Json::UInt(visible_tiles as u64)),
                ("open_stats".into(), stats_json(&stats(&open_samples))),
                ("decode_stats".into(), stats_json(&stats(&decode_samples))),
                (
                    "time_to_pixels_stats".into(),
                    stats_json(&stats(&total_samples)),
                ),
            ]));

            // The pan: what a move costs once the file is open.
            let mut pan_samples = Vec::new();
            let mut pan_totals = Vec::new();
            for _ in 0..options.runs {
                let (latencies, total_ms) = pan_once(options, &path, &limits, workers)?;
                pan_samples.extend(latencies);
                pan_totals.push(total_ms);
            }

            pan_rows.push(Json::Obj(vec![
                ("tile_size".into(), Json::UInt(tile_size as u64)),
                ("workers".into(), Json::UInt(workers as u64)),
                ("runs".into(), Json::UInt(options.runs as u64)),
                ("pans_per_run".into(), Json::UInt(PANS as u64)),
                ("pan_stats".into(), stats_json(&stats(&pan_samples))),
                ("sweep_total_stats".into(), stats_json(&stats(&pan_totals))),
            ]));
        }

        // ---- the sequential baseline ------------------------------------
        let mut baseline_samples = Vec::with_capacity(options.runs);
        let mut baseline_visible = 0usize;
        for _ in 0..options.runs {
            let (elapsed_ms, tiles) = sequential_once(options, &path, &limits)?;
            baseline_samples.push(elapsed_ms);
            baseline_visible = tiles;
        }
        baseline_rows.push(Json::Obj(vec![
            ("tile_size".into(), Json::UInt(tile_size as u64)),
            ("runs".into(), Json::UInt(options.runs as u64)),
            ("visible_tiles".into(), Json::UInt(baseline_visible as u64)),
            ("stats".into(), stats_json(&stats(&baseline_samples))),
        ]));

        if !options.keep_file {
            let _ = fs::remove_file(&path);
        }
    }

    Ok(Json::Obj(vec![
        ("tool".into(), Json::string("vpng-photo-bench")),
        (
            "parameters".into(),
            Json::Obj(vec![
                ("width".into(), Json::UInt(options.width as u64)),
                ("height".into(), Json::UInt(options.height as u64)),
                ("format".into(), Json::string(options.format.to_string())),
                (
                    "tile_sizes".into(),
                    Json::Arr(
                        options
                            .tile_sizes
                            .iter()
                            .map(|&size| Json::UInt(size as u64))
                            .collect(),
                    ),
                ),
                (
                    "worker_counts".into(),
                    Json::Arr(
                        options
                            .worker_counts
                            .iter()
                            .map(|&count| Json::UInt(count as u64))
                            .collect(),
                    ),
                ),
                ("runs".into(), Json::UInt(options.runs as u64)),
                (
                    "viewport_width".into(),
                    Json::UInt(options.viewport_width as u64),
                ),
                (
                    "viewport_height".into(),
                    Json::UInt(options.viewport_height as u64),
                ),
                ("chunk_tiles".into(), Json::UInt(options.chunk_tiles as u64)),
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
                        "the canonical pixel file is read from disk, tile by tile, and is never \
                         held in memory; the figures in this report are latencies, and the \
                         working-set bound of the encoder is measured separately in \
                         docs/P5_ENCODER_REPORT.md with a source that generates tiles on demand",
                    ),
                ),
                (
                    "excluded".into(),
                    Json::string(
                        "the file on disk, the operating system's page cache, the memory of \
                         other processes, and the benchmark driver's own memory",
                    ),
                ),
            ]),
        ),
        (
            "scenarios".into(),
            Json::Obj(vec![
                ("encode_matrix".into(), Json::Arr(encode_rows)),
                ("compression".into(), Json::Arr(compression_rows)),
                ("first_viewport_matrix".into(), Json::Arr(viewport_rows)),
                ("pan_matrix".into(), Json::Arr(pan_rows)),
                ("sequential_baseline".into(), Json::Arr(baseline_rows)),
            ]),
        ),
    ]))
}

/// How many pans each pan run makes.
const PANS: usize = 12;

/// How far the viewport moves on each pan, in pixels.
const PAN_STEP: u32 = 512;

/// A source of canonical pixels read from a raw file on disk.
///
/// The file holds the canonical buffer: tightly packed rows, the whole image
/// in order. A tile is therefore not contiguous in the file — it is one run
/// of bytes per row — and each row is read at its own offset. The reads are
/// positional and lock-free, so the workers do not serialise on the file.
pub struct RawSource {
    file: File,
    width: u32,
    height: u32,
    format: PixelFormat,
}

impl RawSource {
    /// Open the raw file.
    pub fn open(path: &Path, width: u32, height: u32, format: PixelFormat) -> Result<Self> {
        Ok(RawSource {
            file: File::open(path)?,
            width,
            height,
            format,
        })
    }
}

impl PixelSource for RawSource {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn format(&self) -> PixelFormat {
        self.format
    }

    fn tile_pixels(&self, _index: usize, geom: &TileRect, out: &mut Vec<u8>) -> Result<()> {
        let bpp = self.format.bytes_per_pixel() as usize;
        let row_bytes = geom.width as usize * bpp;
        out.clear();
        out.resize(row_bytes * geom.height as usize, 0);
        let stride = self.width as usize * bpp;
        for row in 0..geom.height as usize {
            let offset = ((geom.y as usize + row) * stride + geom.x as usize * bpp) as u64;
            read_exact_at(
                &self.file,
                &mut out[row * row_bytes..(row + 1) * row_bytes],
                offset,
            )?;
        }
        Ok(())
    }
}

/// Read exactly `buffer.len()` bytes at `offset`, without moving the file
/// position.
#[cfg(unix)]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buffer, offset)?;
    Ok(())
}

/// Read exactly `buffer.len()` bytes at `offset`, without moving the file
/// position.
#[cfg(windows)]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> Result<()> {
    use std::os::windows::fs::FileExt;
    let mut read = 0;
    while read < buffer.len() {
        let count = file.seek_read(&mut buffer[read..], offset + read as u64)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "the canonical pixel file ended early",
            )
            .into());
        }
        read += count;
    }
    Ok(())
}

/// The viewport the benchmark opens on: the centre of the image.
fn centre_viewport(options: &PhotoBenchOptions) -> Result<Rect> {
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
fn pan_rects(options: &PhotoBenchOptions) -> Result<Vec<(Rect, Direction)>> {
    let width = options.viewport_width.min(options.width);
    let height = options.viewport_height.min(options.height);
    let step = PAN_STEP.max(1);

    let mut rects = Vec::new();
    for index in 0..PANS {
        let x = (index as u32)
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

/// Open the file and show the first viewport.
///
/// Returns the open time, the decode time, the total time to pixels, and the
/// number of visible tiles.
fn first_viewport_once(
    options: &PhotoBenchOptions,
    path: &Path,
    limits: &Limits,
    workers: usize,
) -> Result<(f64, f64, f64, usize)> {
    let started = Instant::now();
    let reader = TileReader::open(path, limits)?;
    let partition = reader.meta().partition().clone();
    let factory = FileDecoderFactory::from_reader(reader);
    let pool = Pool::new(
        &factory,
        workers,
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
        pool.wait_for_tile(entry.tile, Duration::from_secs(120))?;
    }
    let done = Instant::now();

    let open_ms = opened.duration_since(started).as_secs_f64() * 1000.0;
    let decode_ms = done.duration_since(opened).as_secs_f64() * 1000.0;
    let total_ms = done.duration_since(started).as_secs_f64() * 1000.0;
    Ok((open_ms, decode_ms, total_ms, plan.visible.len()))
}

/// The sequential baseline: the same viewport, decoded one tile at a time
/// with no cache and no threads.
fn sequential_once(
    options: &PhotoBenchOptions,
    path: &Path,
    limits: &Limits,
) -> Result<(f64, usize)> {
    let mut reader = TileReader::open(path, limits)?;
    let partition = reader.meta().partition().clone();
    let rect = centre_viewport(options)?;
    let viewport = Viewport::new(&partition, rect)?;
    let plan = viewport.plan(
        &PrefetchPolicy {
            radius: 0,
            direction_reach: 0,
            direction_bonus: 0,
        },
        Direction::None,
    );

    let started = Instant::now();
    for entry in &plan.visible {
        let pixels = reader.decode_tile(entry.tile, limits)?;
        drop(pixels);
    }
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((elapsed_ms, plan.visible.len()))
}

/// Move the viewport across the image and measure what each move costs.
fn pan_once(
    options: &PhotoBenchOptions,
    path: &Path,
    limits: &Limits,
    workers: usize,
) -> Result<(Vec<f64>, f64)> {
    let reader = TileReader::open(path, limits)?;
    let partition = reader.meta().partition().clone();
    let factory = FileDecoderFactory::from_reader(reader);
    let pool = Pool::new(
        &factory,
        workers,
        options.cache_bytes,
        options.queue_capacity,
    )?;

    let policy = PrefetchPolicy {
        radius: 0,
        direction_reach: 0,
        direction_bonus: 0,
    };

    let rects = pan_rects(options)?;
    let mut latencies = Vec::with_capacity(rects.len());
    let started = Instant::now();

    for (index, (rect, direction)) in rects.iter().enumerate() {
        let viewport = Viewport::new(&partition, *rect)?;
        let plan = viewport.plan(&policy, *direction);

        // A pan: the generation advances, and everything queued for the
        // old viewport is dropped.
        let generation = if index == 0 {
            pool.generation()
        } else {
            pool.advance_generation()
        };
        plan.submit(&pool, generation);

        let pan_started = Instant::now();
        for entry in &plan.visible {
            pool.wait_for_tile(entry.tile, Duration::from_secs(120))?;
        }
        latencies.push(pan_started.elapsed().as_secs_f64() * 1000.0);
    }

    let total_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((latencies, total_ms))
}

/// The summary of a set of samples.
///
/// The rules are the ones the report quotes: the median is the middle sample
/// (or the mean of the two middle ones), the p95 is the nearest-rank
/// percentile, and the standard deviation is the sample one.
pub fn stats(samples: &[f64]) -> Stats {
    if samples.is_empty() {
        return Stats {
            count: 0,
            min: 0.0,
            max: 0.0,
            mean: 0.0,
            median: 0.0,
            p95: 0.0,
            stdev: 0.0,
        };
    }

    let mut sorted = samples.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let count = sorted.len();
    let mean = sorted.iter().sum::<f64>() / count as f64;
    let median = if count % 2 == 1 {
        sorted[count / 2]
    } else {
        (sorted[count / 2 - 1] + sorted[count / 2]) / 2.0
    };
    // Nearest-rank p95: with n samples it is the ceil(0.95 * n)-th smallest.
    let rank = ((0.95f64 * count as f64).ceil() as usize).clamp(1, count);
    let p95 = sorted[rank - 1];
    let stdev = if count > 1 {
        let sum: f64 = sorted.iter().map(|value| (value - mean).powi(2)).sum();
        (sum / (count as f64 - 1.0)).sqrt()
    } else {
        0.0
    };

    Stats {
        count,
        min: sorted[0],
        max: sorted[count - 1],
        mean,
        median,
        p95,
        stdev,
    }
}

/// The JSON form of a summary.
pub fn stats_json(summary: &Stats) -> Json {
    Json::Obj(vec![
        ("count".into(), Json::UInt(summary.count as u64)),
        ("min_ms".into(), Json::Num(summary.min)),
        ("max_ms".into(), Json::Num(summary.max)),
        ("mean_ms".into(), Json::Num(summary.mean)),
        ("median_ms".into(), Json::Num(summary.median)),
        ("p95_ms".into(), Json::Num(summary.p95)),
        ("stdev_ms".into(), Json::Num(summary.stdev)),
    ])
}

/// The JSON form of a list of numbers.
pub fn numbers(values: &[f64]) -> Json {
    Json::Arr(values.iter().map(|&value| Json::Num(value)).collect())
}

/// The SHA-256 of a file, read in chunks so the whole file is never in memory.
pub fn hash_file(path: &Path) -> Result<String> {
    use std::io::Read;

    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize().into()))
}

/// Format a digest as lower-case hex.
fn hex(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The median of an even number of samples is the mean of the two
    /// middle ones.
    #[test]
    fn the_median_of_an_even_count_is_the_mean_of_the_middle_two() {
        let summary = stats(&[4.0, 1.0, 3.0, 2.0]);
        assert_eq!(summary.count, 4);
        assert_eq!(summary.min, 1.0);
        assert_eq!(summary.max, 4.0);
        assert_eq!(summary.median, 2.5);
        assert_eq!(summary.mean, 2.5);
    }

    /// The p95 is the nearest-rank percentile: with ten samples it is the
    /// largest of the ten, and with twenty it is the nineteenth.
    #[test]
    fn the_p95_is_the_nearest_rank_percentile() {
        let ten: Vec<f64> = (1..=10).map(|value| value as f64).collect();
        assert_eq!(stats(&ten).p95, 10.0);

        let twenty: Vec<f64> = (1..=20).map(|value| value as f64).collect();
        assert_eq!(stats(&twenty).p95, 19.0);

        let hundred: Vec<f64> = (1..=100).map(|value| value as f64).collect();
        assert_eq!(stats(&hundred).p95, 95.0);
    }

    /// The standard deviation is the sample one, with n - 1 in the
    /// denominator — the textbook example's 2.138…, not its 2.0.
    #[test]
    fn the_standard_deviation_is_the_sample_one() {
        let summary = stats(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
        assert!(
            (summary.stdev - 2.138_089_935_299_395).abs() < 1e-9,
            "{}",
            summary.stdev
        );
    }

    /// The order of the samples must not matter.
    #[test]
    fn the_summary_does_not_depend_on_the_order() {
        let ordered = stats(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let shuffled = stats(&[5.0, 3.0, 1.0, 4.0, 2.0]);
        assert_eq!(ordered.median, shuffled.median);
        assert_eq!(ordered.p95, shuffled.p95);
        assert_eq!(ordered.mean, shuffled.mean);
        assert_eq!(ordered.stdev, shuffled.stdev);
    }

    /// The source reads the right bytes out of a tightly packed raw file:
    /// a tile is one run of bytes per row, not one contiguous run.
    #[test]
    fn the_source_reads_the_right_tile_out_of_the_raw_file() {
        let mut pixels = Vec::new();
        for row in 0..4u8 {
            for column in 0..4u8 {
                pixels.push(row * 10 + column);
                pixels.push(0);
                pixels.push(0);
            }
        }

        let directory = std::env::temp_dir().join(format!(
            "vpng-raw-source-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("pixels.raw");
        std::fs::write(&path, &pixels).unwrap();

        let source = RawSource::open(&path, 4, 4, PixelFormat::rgb8()).unwrap();
        let geometry = TileRect {
            x: 1,
            y: 2,
            width: 2,
            height: 2,
        };
        let mut out = Vec::new();
        source.tile_pixels(0, &geometry, &mut out).unwrap();

        // Rows 2 and 3, columns 1 and 2: 21, 22, 31, 32, each with two
        // zero bytes after it.
        assert_eq!(
            out,
            vec![21, 0, 0, 22, 0, 0, 31, 0, 0, 32, 0, 0],
            "the tile bytes are wrong"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A source that is asked for a tile past the end of the file must fail
    /// rather than return short bytes.
    #[test]
    fn a_short_file_is_an_error() {
        let directory = std::env::temp_dir().join(format!(
            "vpng-raw-source-short-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("pixels.raw");
        std::fs::write(&path, [0u8; 16]).unwrap();

        let source = RawSource::open(&path, 4, 4, PixelFormat::rgb8()).unwrap();
        let geometry = TileRect {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        };
        let mut out = Vec::new();
        assert!(source.tile_pixels(0, &geometry, &mut out).is_err());

        let _ = std::fs::remove_dir_all(&directory);
    }
}
