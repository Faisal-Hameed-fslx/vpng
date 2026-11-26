//! The native region-decode benchmark.
//!
//! The WebAssembly package's claim is that the browser's pixels are the
//! command line's pixels. The *timing* claim is different and weaker, and
//! this tool is what makes it measurable: it decodes one region of a
//! container, repeatedly, through **the same reader and the same assembly
//! function** the WebAssembly module uses, and reports the distribution.
//!
//! ```text
//! vpng-region-bench picture.vpng --x 0 --y 0 --width 1024 --height 1024 \
//!     --runs 5 --source memory
//! ```
//!
//! The two sources are the two ways the same bytes can be reached:
//!
//! * `--source file` is the streaming reader over the file on disk. It
//!   reads the header, the directory and only the payloads the region
//!   touches. The disk read is part of the measurement.
//! * `--source memory` is the reader over the bytes already in memory. It is
//!   the exact counterpart of the WebAssembly module's whole-file path,
//!   which also has the container in memory before it decodes.
//!
//! The tool prints the region's canonical digest, so the caller can compare
//! it with the WebAssembly module's digest for the same rectangle.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use vpng_core::{Error, Limits, Rect};
use vpng_source::{FileSource, MemorySource, RangeReader};
use vpng_validation::json::Json;

fn main() -> ExitCode {
    let mut input: Option<PathBuf> = None;
    let mut x = 0u32;
    let mut y = 0u32;
    let mut width = 1024u32;
    let mut height = 1024u32;
    let mut runs = 5u32;
    let mut memory = false;
    let mut output: Option<PathBuf> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--x" => x = parse_u32(args.next()),
            "--y" => y = parse_u32(args.next()),
            "--width" => width = parse_u32(args.next()),
            "--height" => height = parse_u32(args.next()),
            "--runs" => runs = parse_u32(args.next()).max(1),
            "--source" => {
                let value = args.next().unwrap_or_default();
                match value.as_str() {
                    "file" => memory = false,
                    "memory" => memory = true,
                    other => {
                        eprintln!("unknown source {other} (expected file or memory)");
                        return ExitCode::from(2);
                    }
                }
            }
            "--output" => output = args.next().map(PathBuf::from),
            "--help" | "-h" => {
                println!(
                    "usage: vpng-region-bench FILE --x N --y N --width N --height N \
                     [--runs N] [--source file|memory] [--output FILE]"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                if input.is_none() {
                    input = Some(PathBuf::from(other));
                } else {
                    eprintln!("unexpected argument {other}");
                    return ExitCode::from(2);
                }
            }
        }
    }

    let Some(input) = input else {
        eprintln!("the benchmark needs a container to read");
        return ExitCode::from(2);
    };

    let limits = Limits::default();
    let rect = match Rect::new(x, y, width, height) {
        Ok(rect) => rect,
        Err(error) => {
            eprintln!("the rectangle is not valid: {error}");
            return ExitCode::from(2);
        }
    };

    // The bytes are read once, outside the measurement, so the file read is
    // not counted twice.
    let bytes = match std::fs::read(&input) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("could not read {}: {error}", input.display());
            return ExitCode::from(2);
        }
    };

    let mut samples = Vec::with_capacity(runs as usize);
    let mut digest = String::new();
    let mut bytes_read = 0u64;
    let mut payloads_read = 0u64;

    for _ in 0..runs {
        let started = Instant::now();
        let region = if memory {
            decode_from_memory(bytes.clone(), rect, &limits)
        } else {
            decode_from_file(&input, rect, &limits)
        };
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        match region {
            Ok((region, read, payloads)) => {
                samples.push(elapsed);
                bytes_read = read;
                payloads_read = payloads;
                digest = vpng_container::to_hex(&vpng_container::canonical_digest(&region));
            }
            Err(error) => {
                eprintln!("the region did not decode: {error}");
                return ExitCode::FAILURE;
            }
        }
    }

    let mut ordered = samples.clone();
    ordered.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = ordered[ordered.len() / 2];

    let report = Json::Obj(vec![
        ("input".into(), Json::string(input.display().to_string())),
        (
            "source".into(),
            Json::string(if memory { "memory" } else { "file" }),
        ),
        ("file_size_bytes".into(), Json::UInt(bytes.len() as u64)),
        (
            "region".into(),
            Json::Obj(vec![
                ("x".into(), Json::UInt(rect.x as u64)),
                ("y".into(), Json::UInt(rect.y as u64)),
                ("width".into(), Json::UInt(rect.width as u64)),
                ("height".into(), Json::UInt(rect.height as u64)),
            ]),
        ),
        ("runs".into(), Json::UInt(runs as u64)),
        (
            "samples_ms".into(),
            Json::Arr(samples.iter().map(|ms| Json::Num(*ms)).collect()),
        ),
        ("median_ms".into(), Json::Num(median)),
        ("min_ms".into(), Json::Num(ordered[0])),
        ("max_ms".into(), Json::Num(ordered[ordered.len() - 1])),
        ("bytes_read".into(), Json::UInt(bytes_read)),
        ("payloads_read".into(), Json::UInt(payloads_read)),
        ("sha256".into(), Json::string(digest)),
    ]);

    let document = report.to_string_pretty();
    match output {
        Some(path) => {
            if let Err(error) = std::fs::write(&path, &document) {
                eprintln!("could not write {}: {error}", path.display());
                return ExitCode::from(2);
            }
            println!("wrote {}", path.display());
        }
        None => print!("{document}"),
    }
    ExitCode::SUCCESS
}

/// Decode a region through the reader over the file on disk.
fn decode_from_file(
    path: &std::path::Path,
    rect: Rect,
    limits: &Limits,
) -> Result<(vpng_core::CanonicalImage, u64, u64), Error> {
    let source = FileSource::open(path)?;
    let mut reader = RangeReader::open_source(source, limits)?;
    let region = reader.decode_region(rect, limits)?;
    Ok((region, reader.bytes_read(), reader.payloads_read()))
}

/// Decode a region through the reader over bytes already in memory.
///
/// This is the exact counterpart of the WebAssembly module's whole-file
/// path, which copies the container into the module's memory before it
/// decodes.
fn decode_from_memory(
    bytes: Vec<u8>,
    rect: Rect,
    limits: &Limits,
) -> Result<(vpng_core::CanonicalImage, u64, u64), Error> {
    let source = MemorySource::new(bytes);
    let mut reader = RangeReader::open_source(source, limits)?;
    let region = reader.decode_region(rect, limits)?;
    Ok((region, reader.bytes_read(), reader.payloads_read()))
}

/// Parse an unsigned 32-bit argument.
fn parse_u32(value: Option<String>) -> u32 {
    value.and_then(|text| text.parse::<u32>().ok()).unwrap_or(0)
}
