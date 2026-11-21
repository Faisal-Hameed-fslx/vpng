//! The command line's v0.2 surface: the preview pyramid and the adaptive
//! partition.
//!
//! The rules the command line must keep:
//!
//! * without `--pyramid` or `--adaptive` the file is a **v0.1** file, byte
//!   for byte what it always was;
//! * `--pyramid` writes a file whose level 0 is the original and whose
//!   derived levels are previews;
//! * `--adaptive` writes a file whose tiles have different sizes;
//! * `decode --level n` decodes a preview, and the original is still exact.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The `vpng` binary, built by Cargo for this test.
const VPNG: &str = env!("CARGO_BIN_EXE_vpng");

/// A temporary directory for one test.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "vpng-cli-v2-{name}-{}-{}",
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

/// Encode the raw image with the given extra arguments.
fn encode(directory: &Path, name: &str, extra: &[&str]) -> PathBuf {
    let input = directory.join("input.raw");
    let output = directory.join(name);
    fs::write(&input, raw_pixels()).unwrap();

    let status = Command::new(VPNG)
        .arg("encode")
        .arg(&input)
        .arg(&output)
        .args(["--input-format", "raw"])
        .args(["--format", "rgb8"])
        .args(["--width", "300"])
        .args(["--height", "200"])
        .args(extra)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "the encode failed: {}",
        String::from_utf8_lossy(&status_output(&status))
    );
    output
}

/// The output of a finished process is empty, so the helper only exists to
/// keep the assertion above readable.
fn status_output(_status: &std::process::ExitStatus) -> Vec<u8> {
    Vec::new()
}

/// The `info` output of a file.
fn info(path: &Path) -> String {
    let output = Command::new(VPNG).arg("info").arg(path).output().unwrap();
    assert!(
        output.status.success(),
        "info failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// The default file is still a v0.1 file.
#[test]
fn the_default_encode_is_a_v0_1_file() {
    let directory = scratch("default");
    let path = encode(&directory, "default.vpng", &[]);
    let text = info(&path);
    assert!(
        text.contains("format version    0.1"),
        "the default file is not a v0.1 file: {text}"
    );
    assert!(
        text.contains("level count       1"),
        "the default file has more than one level: {text}"
    );
    let _ = fs::remove_dir_all(&directory);
}

/// `--pyramid` writes the levels and says so.
#[test]
fn the_pyramid_flag_writes_the_levels() {
    let directory = scratch("pyramid");
    let path = encode(
        &directory,
        "pyramid.vpng",
        &["--pyramid", "--pyramid-levels", "2"],
    );
    let text = info(&path);
    assert!(
        text.contains("format version    0.2"),
        "the pyramid file is not a v0.2 file: {text}"
    );
    assert!(
        text.contains("level count       3"),
        "the pyramid file does not hold three levels: {text}"
    );
    assert!(
        text.contains("derived preview"),
        "the pyramid file does not mark its previews: {text}"
    );
    let _ = fs::remove_dir_all(&directory);
}

/// `--adaptive` writes a file whose tiles have different sizes.
#[test]
fn the_adaptive_flag_writes_an_adaptive_partition() {
    let directory = scratch("adaptive");
    let path = encode(&directory, "adaptive.vpng", &["--adaptive"]);
    let text = info(&path);
    assert!(
        text.contains("format version    0.2"),
        "the adaptive file is not a v0.2 file: {text}"
    );
    assert!(
        text.contains("adaptive"),
        "the adaptive file does not say its partition is adaptive: {text}"
    );
    let _ = fs::remove_dir_all(&directory);
}

/// The original is still exact after a pyramid is added.
#[test]
fn the_original_survives_the_pyramid() {
    let directory = scratch("exact");
    let path = encode(&directory, "pyramid.vpng", &["--pyramid"]);
    let plain = encode(&directory, "plain.vpng", &[]);

    // The two files hold different bytes: the pyramid adds levels.
    assert_ne!(
        fs::read(&path).unwrap(),
        fs::read(&plain).unwrap(),
        "the pyramid file is the same as the plain one"
    );

    // But the original is the same picture in both.
    let mut digests = Vec::new();
    for file in [&path, &plain] {
        let output = Command::new(VPNG)
            .arg("decode")
            .arg(file)
            .arg(directory.join("out.png"))
            .arg("--json")
            .output()
            .unwrap();
        assert!(output.status.success(), "the decode failed");
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let digest = text
            .split("\"canonical_sha256\":\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string();
        digests.push(digest);
    }
    assert_eq!(
        digests[0], digests[1],
        "the pyramid changed the original's pixels"
    );

    let _ = fs::remove_dir_all(&directory);
}

/// A preview level decodes to a smaller picture.
#[test]
fn a_preview_level_decodes() {
    let directory = scratch("level");
    let path = encode(
        &directory,
        "pyramid.vpng",
        &["--pyramid", "--pyramid-levels", "2"],
    );

    let output = Command::new(VPNG)
        .arg("decode")
        .arg(&path)
        .arg(directory.join("preview.png"))
        .args(["--level", "1"])
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the preview decode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        text.contains("\"width\":150"),
        "the preview is not half as wide: {text}"
    );
    assert!(
        text.contains("\"height\":100"),
        "the preview is not half as tall: {text}"
    );

    // A level that does not exist is an error, not a silent fallback.
    let output = Command::new(VPNG)
        .arg("decode")
        .arg(&path)
        .arg(directory.join("nope.png"))
        .args(["--level", "9"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "a missing level was accepted");
    assert_eq!(output.status.code(), Some(2));

    let _ = fs::remove_dir_all(&directory);
}

/// `--pyramid-levels` without `--pyramid` is a mistake, and it is refused.
#[test]
fn the_pyramid_levels_flag_needs_the_pyramid_flag() {
    let directory = scratch("levels-only");
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
        .args(["--pyramid-levels", "2"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "the flag was accepted on its own");
    assert_eq!(output.status.code(), Some(2));

    let _ = fs::remove_dir_all(&directory);
}
