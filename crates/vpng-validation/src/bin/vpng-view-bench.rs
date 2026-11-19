//! The interactive-viewing benchmark driver.
//!
//! One run produces one report: the file is written (or reused), and then
//! the four scenarios run in one process, so the figures describe the same
//! machine and the same file.
//!
//! ```text
//! vpng-view-bench --width 16384 --height 16384 --format rgba8 \
//!     --tile-size 256 --workers 8 --work-dir E:\work \
//!     --output benchmarks/results/p5_view_results.json
//! ```
//!
//! The report is raw data. The prose that interprets it lives in
//! `docs/P5_REPORT.md`, and `benchmarks/verify_reports.py` checks every
//! figure quoted there against this file.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use vpng_core::PixelFormat;
use vpng_validation::json::Json;
use vpng_validation::mem;
use vpng_validation::view_bench::{self, ViewBenchOptions};

/// The tracking allocator, so the tool can report the heap it actually
/// used rather than guessing from the operating system's figures.
#[global_allocator]
static ALLOCATOR: mem::Tracking = mem::Tracking;

fn main() -> ExitCode {
    let mut options = ViewBenchOptions::default_with(PathBuf::from("."));
    let mut output: Option<PathBuf> = None;
    let mut keep_file = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
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
            "--workers" => options.workers = parse_u32(args.next()) as usize,
            "--viewport-width" => options.viewport_width = parse_u32(args.next()),
            "--viewport-height" => options.viewport_height = parse_u32(args.next()),
            "--prefetch-radius" => options.prefetch_radius = parse_u32(args.next()),
            "--direction-reach" => options.direction_reach = parse_u32(args.next()),
            "--direction-bonus" => options.direction_bonus = parse_u32(args.next()),
            "--pans" => options.pans = parse_u32(args.next()),
            "--pan-step" => options.pan_step = parse_u32(args.next()),
            "--cache-bytes" => options.cache_bytes = parse_u64(args.next()),
            "--queue-capacity" => options.queue_capacity = parse_u32(args.next()) as usize,
            "--work-dir" => options.work_dir = PathBuf::from(args.next().unwrap_or_default()),
            "--output" => output = args.next().map(PathBuf::from),
            "--reuse-file" => options.reuse_file = true,
            "--keep-file" => keep_file = true,
            "--help" | "-h" => {
                println!(
                    "usage: vpng-view-bench --width N --height N --format FMT --tile-size N \
                     --workers N --viewport-width N --viewport-height N \
                     --prefetch-radius N --pans N --pan-step N --cache-bytes N \
                     --queue-capacity N --work-dir DIR [--output FILE] [--reuse-file] \
                     [--keep-file]"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument {other}");
                return ExitCode::from(2);
            }
        }
    }

    if options.workers == 0 {
        eprintln!("the worker count must be at least one");
        return ExitCode::from(2);
    }
    if options.tile_size == 0 {
        eprintln!("the tile size must be at least one");
        return ExitCode::from(2);
    }

    let started = std::time::Instant::now();
    let report = match view_bench::run(&options) {
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
                (
                    "allocations".into(),
                    Json::UInt(mem::allocation_count() as u64),
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

    if !keep_file {
        let _ = fs::remove_file(options.file_path());
    }

    ExitCode::SUCCESS
}

/// Parse an unsigned 32-bit argument.
fn parse_u32(value: Option<String>) -> u32 {
    value.and_then(|text| text.parse::<u32>().ok()).unwrap_or(0)
}

/// Parse an unsigned 64-bit argument.
fn parse_u64(value: Option<String>) -> u64 {
    value.and_then(|text| text.parse::<u64>().ok()).unwrap_or(0)
}
