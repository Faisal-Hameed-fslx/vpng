//! The editing-engine benchmark driver.
//!
//! The driver runs every editing scenario and writes one report:
//!
//! ```text
//! vpng-editing-bench --work-dir DIR --output benchmarks/p4_editing_results.json
//! ```

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use vpng_core::PixelFormat;
use vpng_validation::bench::{self, BenchOptions};
use vpng_validation::json::Json;
use vpng_validation::mem;

/// The tracking allocator, so the tool can report the heap it actually
/// used rather than guessing from the operating system's figures.
#[global_allocator]
static ALLOCATOR: mem::Tracking = mem::Tracking;

fn main() -> ExitCode {
    let mut work_dir = PathBuf::from("target/vpng-editing-bench");
    let mut output = PathBuf::from("benchmarks/p4_editing_results.json");
    let mut width = 2048u32;
    let mut height = 2048u32;
    let mut tile_size = 256u32;
    let mut format = PixelFormat::rgba8();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--work-dir" => work_dir = PathBuf::from(args.next().unwrap_or_default()),
            "--output" => output = PathBuf::from(args.next().unwrap_or_default()),
            "--width" => width = args.next().and_then(|v| v.parse().ok()).unwrap_or(2048),
            "--height" => height = args.next().and_then(|v| v.parse().ok()).unwrap_or(2048),
            "--tile-size" => tile_size = args.next().and_then(|v| v.parse().ok()).unwrap_or(256),
            "--format" => {
                let value = args.next().unwrap_or_default();
                format = match value.as_str() {
                    "gray8" => PixelFormat::gray8(),
                    "rgb8" => PixelFormat::rgb8(),
                    "rgba8" => PixelFormat::rgba8(),
                    "rgba16" => PixelFormat::rgba16(),
                    _ => PixelFormat::rgba8(),
                };
            }
            "--help" | "-h" => {
                println!(
                    "usage: vpng-editing-bench --work-dir DIR --output FILE \
                     [--width N] [--height N] [--tile-size N] [--format FMT]"
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

    let options = BenchOptions {
        work_dir: work_dir.clone(),
        width,
        height,
        tile_size,
        format,
    };

    let started = Instant::now();
    let scenarios = bench::run_all(&options);
    let elapsed = started.elapsed();
    bench::cleanup(&options);

    let scenario_values: Vec<Json> = scenarios
        .into_iter()
        .map(|scenario| scenario.result)
        .collect();

    let document = Json::Obj(vec![
        ("tool".into(), Json::string("vpng-editing-bench")),
        ("schema".into(), Json::UInt(1)),
        (
            "generated_at".into(),
            Json::string(format!("{}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0))),
        ),
        (
            "context".into(),
            Json::Obj(vec![
                ("width".into(), Json::UInt(width as u64)),
                ("height".into(), Json::UInt(height as u64)),
                ("format".into(), Json::string(format.to_string())),
                ("tile_size".into(), Json::UInt(tile_size as u64)),
                ("elapsed_ms".into(), Json::UInt(elapsed.as_millis() as u64)),
                (
                    "rustc".into(),
                    Json::string(
                        std::env::var("RUSTC_VERSION")
                            .unwrap_or_else(|_| "unknown".to_string()),
                    ),
                ),
            ]),
        ),
        (
            "definitions".into(),
            Json::Obj(vec![
                (
                    "cache_bytes".into(),
                    Json::string(
                        "the bytes of decoded tile pixels the cache holds; the budget is a byte budget, not a tile count",
                    ),
                ),
                (
                    "history_bytes".into(),
                    Json::string(
                        "the bytes the undo history holds: the pixel buffers plus a per-entry overhead estimate",
                    ),
                ),
                (
                    "payload_identity".into(),
                    Json::string(
                        "the CRC32C of every tile payload, before and after the save; an untouched tile must have the same CRC",
                    ),
                ),
                (
                    "digest".into(),
                    Json::string(
                        "the canonical SHA-256 digest of the pixels, as defined in docs/VPNG_SPEC_0_1.md section 7",
                    ),
                ),
                (
                    "tiles_decoded".into(),
                    Json::string(
                        "tiles whose compressed payload was read and decoded back into pixels; decoding is not rewriting",
                    ),
                ),
                (
                    "tiles_modified".into(),
                    Json::string(
                        "tiles whose pixels changed; a modified tile is one the editor's dirty set holds",
                    ),
                ),
                (
                    "tiles_rewritten".into(),
                    Json::string(
                        "tiles whose encoded payload was replaced during a commit, by re-encoding the modified pixels",
                    ),
                ),
                (
                    "tiles_preserved".into(),
                    Json::string(
                        "tiles whose original compressed payload was copied into the new file byte for byte; a preserved tile was neither decoded nor re-encoded during the commit",
                    ),
                ),
            ]),
        ),
        ("scenarios".into(), Json::Arr(scenario_values)),
    ])
    .to_string_pretty();

    if let Some(parent) = output.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(error) = fs::write(&output, &document) {
        eprintln!("could not write {}: {error}", output.display());
        return ExitCode::from(2);
    }
    println!("wrote {}", output.display());
    ExitCode::SUCCESS
}
