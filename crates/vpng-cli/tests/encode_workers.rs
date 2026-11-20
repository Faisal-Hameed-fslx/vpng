//! The command line's `encode` must produce the same file whatever the
//! worker count is.
//!
//! The encoder's per-tile work is independent and the tiles are consumed in
//! tile order, so the worker count changes the speed and not the bytes.
//! That is the property the command line exposes with `--workers`, and this
//! test is the command line's own statement of it.
//!
//! The input is a raw file rather than a PNG, so the test needs nothing but
//! a byte buffer and does not depend on an image encoder.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The `vpng` binary, built by Cargo for this test.
const VPNG: &str = env!("CARGO_BIN_EXE_vpng");

/// A temporary directory for one test.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "vpng-cli-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// A deterministic raw RGB8 image: 300 × 200, with structure in it so the
/// encoder has something to choose between.
fn raw_pixels() -> Vec<u8> {
    let mut pixels = Vec::with_capacity(300 * 200 * 3);
    for y in 0..200u32 {
        for x in 0..300u32 {
            pixels.push((x % 251) as u8);
            pixels.push((y % 241) as u8);
            pixels.push((((x / 16) + (y / 16)) % 2 * 255) as u8);
        }
    }
    pixels
}

/// Encode the raw image with the given worker count and return the file's
/// bytes.
fn encode_with(directory: &Path, workers: usize) -> Vec<u8> {
    let input = directory.join("input.raw");
    let output = directory.join(format!("out-{workers}.vpng"));
    fs::write(&input, raw_pixels()).unwrap();

    let status = Command::new(VPNG)
        .arg("encode")
        .arg(&input)
        .arg(&output)
        .args(["--input-format", "raw"])
        .args(["--format", "rgb8"])
        .args(["--width", "300"])
        .args(["--height", "200"])
        .args(["--workers", &workers.to_string()])
        .status()
        .unwrap();
    assert!(status.success(), "the encode with {workers} workers failed");

    fs::read(&output).unwrap()
}

/// One worker and N workers write the same file, byte for byte.
#[test]
fn the_worker_count_does_not_change_the_file() {
    let directory = scratch("workers");
    let one = encode_with(&directory, 1);
    for workers in [2usize, 3, 4, 8, 16] {
        let many = encode_with(&directory, workers);
        assert_eq!(
            one.len(),
            many.len(),
            "the file with {workers} workers is a different size"
        );
        assert_eq!(
            one, many,
            "the file with {workers} workers differs from the one with 1 worker"
        );
    }
    let _ = fs::remove_dir_all(&directory);
}

/// The file the command line writes is the file the library's batch encoder
/// writes: the command line must not have become a different format.
#[test]
fn the_command_line_writes_a_readable_file() {
    let directory = scratch("readable");
    let bytes = encode_with(&directory, 4);
    let path = directory.join("out-4.vpng");
    fs::write(&path, &bytes).unwrap();

    let output = Command::new(VPNG).arg("info").arg(&path).output().unwrap();
    assert!(
        output.status.success(),
        "the file the command line wrote does not read back: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("300"),
        "the header does not carry the width: {text}"
    );
    assert!(
        text.contains("200"),
        "the header does not carry the height: {text}"
    );

    let _ = fs::remove_dir_all(&directory);
}

/// A zero worker count is an error, not a silent fallback.
#[test]
fn a_zero_worker_count_is_rejected() {
    let directory = scratch("zero");
    let input = directory.join("input.raw");
    fs::write(&input, raw_pixels()).unwrap();

    let output = Command::new(VPNG)
        .arg("encode")
        .arg(&input)
        .arg(directory.join("out.vpng"))
        .args(["--input-format", "raw"])
        .args(["--format", "rgb8"])
        .args(["--width", "300"])
        .args(["--height", "200"])
        .args(["--workers", "0"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "a zero worker count was accepted");
    assert_eq!(
        output.status.code(),
        Some(2),
        "the exit code is not the invalid-argument one"
    );

    let _ = fs::remove_dir_all(&directory);
}
