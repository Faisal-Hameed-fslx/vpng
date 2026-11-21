//! The preview-pyramid benchmark driver.
//!
//! One run produces one report: the same photograph is encoded as an ordinary
//! v0.1 file and as a v0.2 file with a preview pyramid, and the latency of
//! the first viewport, of every preview level and of the whole picture is
//! measured with the median and the p95.
//!
//! ```text
//! vpng-preview-bench --source corpus-data/cache/<name>.raw \
//!     --width 15360 --height 8640 --format rgb8 \
//!     --tile-size 256 --workers 8 --runs 5 --pyramid-levels 3 \
//!     --work-dir E:\work --output benchmarks/p6_preview_results.json
//! ```

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use vpng_core::PixelFormat;
use vpng_validation::json::Json;
use vpng_validation::mem;
use vpng_validation::preview_bench::{self, PreviewBenchOptions};

/// The tracking allocator, so the tool can report the heap it actually used
/// rather than guessing from the operating system's figures.
#[global_allocator]
static ALLOCATOR: mem::Tracking = mem::Tracking;

fn main() -> ExitCode {
    let mut options = PreviewBenchOptions::default_with(PathBuf::from("."));
    let mut output: Option<PathBuf> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source" => options.source_path = PathBuf::from(args.next().unwrap_or_default()),
            "--width" => options.width = parse_u32(args.next()),
            "--height" => options.height = parse_u32(args.next()),
            "--format" => {
                let value = args.next().unwrap_or_default();
                options.format = match value.as_str() {
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
            "--tile-size" => options.tile_size = parse_u32(args.next()),
            "--workers" => options.workers = parse_u32(args.next()).max(1) as usize,
            "--runs" => options.runs = parse_u32(args.next()).max(1) as usize,
            "--pyramid-levels" => options.pyramid_levels = parse_u32(args.next()),
            "--viewport-width" => options.viewport_width = parse_u32(args.next()),
            "--viewport-height" => options.viewport_height = parse_u32(args.next()),
            "--work-dir" => options.work_dir = PathBuf::from(args.next().unwrap_or_default()),
            "--output" => output = args.next().map(PathBuf::from),
            "--keep-file" => options.keep_file = true,
            "--help" | "-h" => {
                println!(
                    "usage: vpng-preview-bench --source FILE --width N --height N --format FMT \
                     --tile-size N --workers N --runs N --pyramid-levels N \
                     --viewport-width N --viewport-height N --work-dir DIR \
                     [--output FILE] [--keep-file]"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument {other}");
                return ExitCode::from(2);
            }
        }
    }

    if options.source_path.as_os_str().is_empty() {
        eprintln!("a canonical source file is required");
        return ExitCode::from(2);
    }
    if !options.source_path.exists() {
        eprintln!(
            "the canonical source {} does not exist; run benchmarks/corpus/canonical_cache.py",
            options.source_path.display()
        );
        return ExitCode::from(2);
    }
    if options.tile_size == 0 {
        eprintln!("the tile size must be at least one");
        return ExitCode::from(2);
    }

    let started = std::time::Instant::now();
    let report = match preview_bench::run(&options) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("the benchmark failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

    let document = Json::Obj(vec![
        ("report".into(), report),
        (
            "process".into(),
            Json::Obj(vec![
                ("elapsed_ms".into(), Json::Num(elapsed_ms)),
                (
                    "peak_working_set_bytes".into(),
                    Json::UInt(mem::sample().peak_working_set_bytes),
                ),
                (
                    "peak_heap_bytes".into(),
                    Json::UInt(mem::peak_heap_bytes() as u64),
                ),
            ]),
        ),
    ])
    .to_string_pretty();

    match output {
        Some(path) => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    let _ = fs::create_dir_all(parent);
                }
            }
            if let Err(error) = fs::write(&path, &document) {
                eprintln!("could not write {}: {error}", path.display());
                return ExitCode::from(2);
            }
            println!("wrote {}", path.display());
        }
        None => print!("{document}"),
    }

    ExitCode::SUCCESS
}

/// Parse an unsigned 32-bit argument.
fn parse_u32(value: Option<String>) -> u32 {
    value.and_then(|text| text.parse::<u32>().ok()).unwrap_or(0)
}
