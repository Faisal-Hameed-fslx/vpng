//! The command line's v0.3 surface: the promoted codecs.
//!
//! The rules the command line must keep:
//!
//! * the default encode is still a **v0.1** file, byte for byte what it
//!   always was;
//! * `--codec-set extended` lets the selector use the promoted codecs, and
//!   the file says so in its version when it names one;
//! * `--codec lz77`, `--codec predictor+dictionary` and
//!   `--codec context-entropy` force one of them;
//! * an unknown codec set or codec name is refused, not ignored;
//! * the file round-trips: `vpng verify` decodes it and compares it with the
//!   original pixels.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The `vpng` binary, built by Cargo for this test.
const VPNG: &str = env!("CARGO_BIN_EXE_vpng");

/// A temporary directory for one test.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "vpng-cli-v3-{name}-{}-{}",
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
    assert!(status.success(), "the encode failed");
    output
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

/// The default file is still a v0.1 file, and the stable set says so.
#[test]
fn the_default_encode_is_a_v0_1_file() {
    let directory = scratch("default");
    let path = encode(&directory, "default.vpng", &[]);
    let text = info(&path);
    assert!(
        text.contains("format version    0.1"),
        "the default file is not a v0.1 file: {text}"
    );
    let _ = fs::remove_dir_all(&directory);
}

/// The stable set writes the same bytes as the default.
#[test]
fn the_stable_set_is_the_default() {
    let directory = scratch("stable");
    let plain = encode(&directory, "plain.vpng", &[]);
    let stable = encode(&directory, "stable.vpng", &["--codec-set", "stable"]);
    assert_eq!(
        fs::read(&plain).unwrap(),
        fs::read(&stable).unwrap(),
        "`--codec-set stable` changed the default bytes"
    );
    let _ = fs::remove_dir_all(&directory);
}

/// The extended set may name a promoted codec, and then the file is a v0.3
/// file.
#[test]
fn the_extended_set_can_write_a_v0_3_file() {
    let directory = scratch("extended");
    let path = encode(&directory, "extended.vpng", &["--codec-set", "extended"]);
    let text = info(&path);
    let version = if text.contains("format version    0.3") {
        3
    } else {
        assert!(
            text.contains("format version    0.1"),
            "the extended file is neither a v0.1 nor a v0.3 file: {text}"
        );
        1
    };

    // Whatever the version is, it must agree with the codecs the file names.
    let bytes = fs::read(&path).unwrap();
    let tile_count = u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);
    let directory_offset = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
    let mut uses_promoted = false;
    for index in 0..tile_count as usize {
        let entry = directory_offset as usize + index * 40;
        uses_promoted |= vpng_codecs::is_v0_3_codec(bytes[entry + 12]);
    }
    if uses_promoted {
        assert_eq!(
            version, 3,
            "a file that names a promoted codec must be v0.3"
        );
    }
    let _ = fs::remove_dir_all(&directory);
}

/// Every promoted codec can be forced from the command line, and the file it
/// writes is a v0.3 file that round-trips.
#[test]
fn every_promoted_codec_can_be_forced() {
    for codec in [
        "lz77",
        "predictor+dictionary",
        "context-entropy",
        "4",
        "5",
        "6",
    ] {
        let directory = scratch(&format!("forced-{codec}"));
        let path = encode(&directory, "forced.vpng", &["--codec", codec]);
        let text = info(&path);
        assert!(
            text.contains("format version    0.3"),
            "forcing `{codec}` did not write a v0.3 file: {text}"
        );

        // The file round-trips: the decoded pixels are the original pixels.
        let input = directory.join("input.raw");
        let output = Command::new(VPNG)
            .arg("verify")
            .arg(&input)
            .arg(&path)
            .args(["--input-format", "raw"])
            .args(["--format", "rgb8"])
            .args(["--width", "300"])
            .args(["--height", "200"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "forcing `{codec}` did not round-trip: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let _ = fs::remove_dir_all(&directory);
    }
}

/// An unknown codec set is refused, not ignored.
#[test]
fn an_unknown_codec_set_is_refused() {
    let directory = scratch("unknown-set");
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
        .args(["--codec-set", "nonsense"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "an unknown codec set was accepted"
    );
    let _ = fs::remove_dir_all(&directory);
}

/// An unknown codec name is refused, not ignored.
#[test]
fn an_unknown_codec_is_refused() {
    let directory = scratch("unknown-codec");
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
        .args(["--codec", "nonsense"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "an unknown codec was accepted");
    let _ = fs::remove_dir_all(&directory);
}

/// The worker count does not change the bytes, with the extended set either.
#[test]
fn the_worker_count_does_not_change_the_bytes() {
    let directory = scratch("workers");
    let one = encode(
        &directory,
        "one.vpng",
        &["--codec-set", "extended", "--workers", "1"],
    );
    let many = encode(
        &directory,
        "many.vpng",
        &["--codec-set", "extended", "--workers", "4"],
    );
    assert_eq!(
        fs::read(&one).unwrap(),
        fs::read(&many).unwrap(),
        "the worker count changed the extended file"
    );
    let _ = fs::remove_dir_all(&directory);
}
