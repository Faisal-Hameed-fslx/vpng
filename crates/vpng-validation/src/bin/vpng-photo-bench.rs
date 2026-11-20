//! The multi-run photographic benchmark driver.
//!
//! One run produces one report: a latency matrix over the tile size and the
//! worker count, every case repeated, with the median and the p95. The
//! photograph is read from a raw canonical file, so the benchmark never
//! holds the image in memory.
//!
//! ```text
//! vpng-photo-bench --source corpus-data/cache/<name>.raw \
//!     --width 15360 --height 8640 --format rgb8 \
//!     --tile-sizes 256,512,1024 --workers 1,2,4,8 --runs 10 \
//!     --work-dir E:\work --output benchmarks/p5_bench_results.json
//! ```

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use vpng_core::PixelFormat;
use vpng_validation::json::Json;
use vpng_validation::mem;
use vpng_validation::photo_bench::{self, PhotoBenchOptions};

/// The tracking allocator, so the tool can report the heap it actually used
/// rather than guessing from the operating system's figures.
#[global_allocator]
static ALLOCATOR: mem::Tracking = mem::Tracking;

fn main() -> ExitCode {
    let mut options = PhotoBenchOptions::default_with(PathBuf::from("."));
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
            "--tile-sizes" => {
                options.tile_sizes = args
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|part| part.trim().parse::<u32>().ok())
                    .filter(|&size| size >= 1)
                    .collect();
                if options.tile_sizes.is_empty() {
                    eprintln!("the tile-size list must name at least one tile size");
                    return ExitCode::from(2);
                }
            }
            "--workers" => {
                options.worker_counts = args
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|part| part.trim().parse::<usize>().ok())
                    .filter(|&count| count >= 1)
                    .collect();
                if options.worker_counts.is_empty() {
                    eprintln!("the worker list must name at least one worker");
                    return ExitCode::from(2);
                }
            }
            "--runs" => options.runs = parse_u32(args.next()).max(1) as usize,
            "--viewport-width" => options.viewport_width = parse_u32(args.next()),
            "--viewport-height" => options.viewport_height = parse_u32(args.next()),
            "--chunk-tiles" => options.chunk_tiles = parse_u32(args.next()).max(1) as usize,
            "--work-dir" => options.work_dir = PathBuf::from(args.next().unwrap_or_default()),
            "--output" => output = args.next().map(PathBuf::from),
            "--keep-file" => options.keep_file = true,
            "--help" | "-h" => {
                println!(
                    "usage: vpng-photo-bench --source FILE --width N --height N --format FMT \
                     --tile-sizes 256,512,1024 --workers 1,2,4,8 --runs N \
                     --viewport-width N --viewport-height N --chunk-tiles N --work-dir DIR \
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

    let started = std::time::Instant::now();
    let report = match photo_bench::run(&options) {
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
