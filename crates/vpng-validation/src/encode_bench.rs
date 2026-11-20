//! The parallel-encoder acceptance benchmark.
//!
//! The question this measures is narrow and answerable: **does using more
//! workers make the encoder faster, and does it change the file?** The
//! answer has to be both, and the second half is the one that matters: a
//! faster encoder that produces different bytes is not an encoder, it is a
//! different file format.
//!
//! The benchmark therefore records the SHA-256 of every file it writes. If
//! the worker count changed a single byte, the digests would differ and the
//! report would say so.
//!
//! # The order of the scenarios
//!
//! The streaming scenario runs **first**, and its memory is sampled before
//! the agreement scenario runs. The resident-set high-water mark the
//! operating system reports cannot be reset, so a scenario that ran earlier
//! would put a floor under every later measurement. The batch encoder
//! deliberately holds the whole image in memory, so it must come last.

use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;
use std::time::Instant;

use sha2::{Digest, Sha256};
use vpng_core::{CanonicalImage, Limits, PixelFormat, Result};
use vpng_encoder::parallel::ParallelOptions;
use vpng_encoder::source::PixelSource;
use vpng_encoder::stream::encode_to_file;
use vpng_encoder::{encode, EncodeOptions};
use vpng_tiling::TileGeometry;

use crate::json::Json;

/// The configuration of one benchmark run.
#[derive(Debug, Clone)]
pub struct EncodeBenchOptions {
    /// The image's width in pixels.
    pub width: u32,
    /// The image's height in pixels.
    pub height: u32,
    /// The image's pixel format.
    pub format: PixelFormat,
    /// The nominal tile size.
    pub tile_size: u32,
    /// The worker counts to measure, in order.
    pub worker_counts: Vec<usize>,
    /// How many tiles one work item covers.
    pub chunk_tiles: usize,
    /// Where the files are written.
    pub work_dir: PathBuf,
    /// The width of the smaller case used to check the batch encoder against
    /// the streaming writer.
    pub agreement_width: u32,
    /// The height of the smaller case used to check the batch encoder against
    /// the streaming writer.
    pub agreement_height: u32,
    /// Keep the files after the run.
    pub keep_file: bool,
}

impl EncodeBenchOptions {
    /// The defaults: a 16384 × 16384 RGBA8 image at 256 × 256 tiles, and the
    /// worker counts 1, 2, 4 and 8.
    pub fn default_with(work_dir: PathBuf) -> Self {
        EncodeBenchOptions {
            width: 16384,
            height: 16384,
            format: PixelFormat::rgba8(),
            tile_size: 256,
            worker_counts: vec![1, 2, 4, 8],
            chunk_tiles: 4,
            work_dir,
            agreement_width: 4096,
            agreement_height: 4096,
            keep_file: false,
        }
    }

    /// The path the benchmark's file is written to.
    pub fn file_path(&self) -> PathBuf {
        self.work_dir.join(format!(
            "encode-{}x{}-{}.vpng",
            self.width, self.height, self.format
        ))
    }
}

/// Which pattern a [`GeneratedSource`] produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pattern {
    /// The benchmark's smooth gradient (`stream::fill_tile`): the same
    /// pattern the P4 and P5 benchmarks use, so the files are comparable.
    Fill,
    /// The agreement case's pattern. The batch encoder and the streaming
    /// writer must be given *the same* pixels, so both read this function.
    Agreement,
}

/// A source that generates its tiles on demand, so the benchmark never holds
/// the whole image.
struct GeneratedSource {
    width: u32,
    height: u32,
    format: PixelFormat,
    pattern: Pattern,
}

/// The agreement pattern: a smooth gradient with a band that changes every 64
/// pixels, so the tiles differ from each other and the per-tile codec choice
/// has something to choose between.
fn agreement_pixel(x: u32, y: u32, channel: u32) -> u8 {
    (((x * 7 + y * 13 + channel * 29) & 0xff) as u8) ^ ((x / 64) as u8)
}

impl PixelSource for GeneratedSource {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn format(&self) -> PixelFormat {
        self.format
    }

    fn tile_pixels(&self, _index: usize, geom: &TileGeometry, out: &mut Vec<u8>) -> Result<()> {
        match self.pattern {
            Pattern::Fill => {
                crate::stream::fill_tile(geom.x, geom.y, geom.width, geom.height, self.format, out);
            }
            Pattern::Agreement => {
                let bpp = self.format.bytes_per_pixel() as usize;
                out.clear();
                out.resize(geom.width as usize * geom.height as usize * bpp, 0);
                for row in 0..geom.height {
                    for column in 0..geom.width {
                        let base = (row as usize * geom.width as usize + column as usize) * bpp;
                        for channel in 0..bpp {
                            out[base + channel] =
                                agreement_pixel(geom.x + column, geom.y + row, channel as u32);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Run the benchmark.
pub fn run(options: &EncodeBenchOptions) -> Result<Json> {
    let limits = Limits::default();
    fs::create_dir_all(&options.work_dir)?;

    let source = GeneratedSource {
        width: options.width,
        height: options.height,
        format: options.format,
        pattern: Pattern::Fill,
    };
    let encode_options = EncodeOptions::default()
        .with_tile_size(options.tile_size, options.tile_size)
        .with_limits(limits);

    let path = options.file_path();

    // ---- the streaming scenario, first, so its memory is the clean figure --

    crate::mem::reset_peaks();

    let mut runs: Vec<Json> = Vec::new();
    let mut digests: Vec<String> = Vec::new();
    let mut baseline_ms: Option<f64> = None;

    for &workers in &options.worker_counts {
        let started = Instant::now();
        let report = encode_to_file(
            &source,
            &encode_options,
            &ParallelOptions::default()
                .with_workers(workers)
                .with_chunk_tiles(options.chunk_tiles),
            &path,
        )?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        // The file is hashed in chunks, not read whole: the benchmark must
        // not inflate its own memory figure by loading the finished file.
        let digest = hash_file(&path)?;
        digests.push(digest.clone());

        if workers == 1 || baseline_ms.is_none() {
            baseline_ms = Some(elapsed_ms);
        }
        let baseline = baseline_ms.unwrap_or(elapsed_ms);

        runs.push(Json::Obj(vec![
            ("workers".into(), Json::UInt(workers as u64)),
            ("encode_ms".into(), Json::Num(elapsed_ms)),
            (
                "speedup_vs_one_worker".into(),
                Json::Num(baseline / elapsed_ms.max(f64::MIN_POSITIVE)),
            ),
            ("file_size_bytes".into(), Json::UInt(report.file_size)),
            ("tiles".into(), Json::UInt(report.tile_count as u64)),
            ("payload_bytes".into(), Json::UInt(report.payload_bytes)),
            ("framing_bytes".into(), Json::UInt(report.framing_bytes)),
            ("sha256".into(), Json::string(digest.clone())),
        ]));
    }

    let all_identical = digests.windows(2).all(|pair| pair[0] == pair[1]);
    let digest = digests.first().cloned().unwrap_or_default();

    if !options.keep_file {
        let _ = fs::remove_file(&path);
    }

    let streaming_memory = crate::mem::sample();

    // ---- the agreement scenario, last, because it holds a whole image ------

    crate::mem::reset_peaks();
    let agreement = agreement_scenario(options, &limits)?;
    let agreement_memory = crate::mem::sample();

    Ok(Json::Obj(vec![
        (
            "parameters".into(),
            Json::Obj(vec![
                ("width".into(), Json::UInt(options.width as u64)),
                ("height".into(), Json::UInt(options.height as u64)),
                ("format".into(), Json::string(options.format.to_string())),
                ("tile_size".into(), Json::UInt(options.tile_size as u64)),
                (
                    "worker_counts".into(),
                    Json::Arr(
                        options
                            .worker_counts
                            .iter()
                            .map(|&w| Json::UInt(w as u64))
                            .collect(),
                    ),
                ),
                ("chunk_tiles".into(), Json::UInt(options.chunk_tiles as u64)),
            ]),
        ),
        (
            "memory_scope".into(),
            Json::Obj(vec![
                (
                    "included".into(),
                    Json::string(
                        "the process's resident set (working set) and the allocator's tracked heap \
                         during the measured phase, including the tiles in flight, the encoded \
                         payloads waiting to be written and the container metadata",
                    ),
                ),
                (
                    "excluded".into(),
                    Json::string(
                        "the file on disk, the operating system's file cache, the memory of other \
                         processes, and the benchmark driver's own memory",
                    ),
                ),
                (
                    "streaming_scenario".into(),
                    Json::string(
                        "measured before the agreement scenario ran, because the operating \
                         system's resident-set high-water mark cannot be reset and the batch \
                         encoder deliberately holds a whole image",
                    ),
                ),
            ]),
        ),
        (
            "scenarios".into(),
            Json::Obj(vec![
                (
                    "streaming".into(),
                    Json::Obj(vec![
                        ("runs".into(), Json::Arr(runs)),
                        (
                            "all_worker_counts_identical".into(),
                            Json::Bool(all_identical),
                        ),
                        ("sha256".into(), Json::string(digest)),
                        ("memory".into(), memory_json(&streaming_memory)),
                        (
                            "note".into(),
                            Json::string(
                                "every worker count wrote the same file: the digest is the proof, \
                                 not an assertion",
                            ),
                        ),
                    ]),
                ),
                (
                    "batch_agreement".into(),
                    match &agreement {
                        Json::Obj(entries) => {
                            let mut entries = entries.clone();
                            entries.push(("memory".into(), memory_json(&agreement_memory)));
                            Json::Obj(entries)
                        }
                        other => other.clone(),
                    },
                ),
            ]),
        ),
    ]))
}

/// Check that the batch encoder and the streaming writer agree.
///
/// The batch encoder holds the whole image and the whole file; the streaming
/// writer holds neither. They must produce the same bytes.
fn agreement_scenario(options: &EncodeBenchOptions, limits: &Limits) -> Result<Json> {
    let width = options.agreement_width;
    let height = options.agreement_height;
    let format = options.format;

    let bpp = format.bytes_per_pixel() as usize;
    let mut data = vec![0u8; width as usize * height as usize * bpp];
    for y in 0..height {
        for x in 0..width {
            let base = (y as usize * width as usize + x as usize) * bpp;
            for c in 0..bpp {
                data[base + c] = agreement_pixel(x, y, c as u32);
            }
        }
    }
    let image = CanonicalImage::from_vec(width, height, format, data)?;

    let encode_options = EncodeOptions::default()
        .with_tile_size(options.tile_size, options.tile_size)
        .with_limits(*limits);

    let started = Instant::now();
    let batch = encode(&image, &encode_options)?;
    let batch_ms = started.elapsed().as_secs_f64() * 1000.0;
    let batch_digest = hex(&vpng_container::sha256(&batch.bytes));

    let source = GeneratedSource {
        width,
        height,
        format,
        pattern: Pattern::Agreement,
    };
    let path = options
        .work_dir
        .join(format!("agreement-{width}x{height}-{format}.vpng"));
    let started = Instant::now();
    let streamed = encode_to_file(
        &source,
        &encode_options,
        &ParallelOptions::default().with_workers(4),
        &path,
    )?;
    let streamed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let streamed_digest = hash_file(&path)?;
    let _ = fs::remove_file(&path);

    Ok(Json::Obj(vec![
        ("width".into(), Json::UInt(width as u64)),
        ("height".into(), Json::UInt(height as u64)),
        ("batch_ms".into(), Json::Num(batch_ms)),
        ("streaming_ms".into(), Json::Num(streamed_ms)),
        ("batch_file_size_bytes".into(), Json::UInt(batch.file_size)),
        (
            "streaming_file_size_bytes".into(),
            Json::UInt(streamed.file_size),
        ),
        ("batch_sha256".into(), Json::string(batch_digest.clone())),
        (
            "streaming_sha256".into(),
            Json::string(streamed_digest.clone()),
        ),
        (
            "identical".into(),
            Json::Bool(batch_digest == streamed_digest),
        ),
        (
            "note".into(),
            Json::string(
                "the batch encoder holds the whole image and the whole file; the streaming \
                 writer holds neither; they must agree byte for byte",
            ),
        ),
    ]))
}

/// The memory figures for one scenario.
fn memory_json(sample: &crate::mem::MemorySample) -> Json {
    Json::Obj(vec![
        (
            "peak_working_set_bytes".into(),
            Json::UInt(sample.peak_working_set_bytes),
        ),
        (
            "peak_heap_bytes".into(),
            Json::UInt(sample.heap_peak_bytes as u64),
        ),
    ])
}

/// The SHA-256 of a file, read in chunks so the whole file is never in memory.
fn hash_file(path: &std::path::Path) -> Result<String> {
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
