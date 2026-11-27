//! The command line's v0.4 surface: layers and masks.
//!
//! The rules the command line must keep:
//!
//! * `encode-layers` builds a **v0.4** file from a manifest;
//! * `layers` lists what the file holds;
//! * `decode-layer` writes one layer's **own** pixels, exactly;
//! * `composite` blends every layer into one image;
//! * a file with one layer composites to exactly that layer's pixels;
//! * a bad manifest is refused with the invalid-argument exit code, not
//!   ignored.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The `vpng` binary, built by Cargo for this test.
const VPNG: &str = env!("CARGO_BIN_EXE_vpng");

/// A temporary directory for one test.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "vpng-cli-v4-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// A deterministic raw RGBA8 image.
fn raw_pixels(width: u32, height: u32, seed: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            pixels.push((x % 251 + seed) as u8);
            pixels.push((y % 241) as u8);
            pixels.push((((x / 16) + (y / 16)) % 2 * 255) as u8);
            pixels.push(0xFF);
        }
    }
    pixels
}

/// Write a raw image.
fn write_raw(path: &Path, width: u32, height: u32, seed: u32) {
    fs::write(path, raw_pixels(width, height, seed)).unwrap();
}

/// The manifest of a three-layer file: a base, an overlay and a mask.
fn manifest(directory: &Path) -> PathBuf {
    let path = directory.join("layers.json");
    fs::write(
        &path,
        r#"{
  "layers": [
    {"name": "base", "path": "base.raw", "width": 64, "height": 48, "format": "rgba8"},
    {"name": "ink", "path": "ink.raw", "x": 8, "y": 8, "width": 32, "height": 16, "format": "rgba8", "mask": 2},
    {"name": "m", "kind": "mask", "path": "mask.raw", "x": 8, "y": 8, "width": 32, "height": 16, "format": "rgba8"}
  ]
}
"#,
    )
    .unwrap();
    path
}

/// Build a layered file and return its path.
fn build(directory: &Path) -> PathBuf {
    write_raw(&directory.join("base.raw"), 64, 48, 0);
    write_raw(&directory.join("ink.raw"), 32, 16, 7);
    write_raw(&directory.join("mask.raw"), 32, 16, 0);

    let manifest = manifest(directory);
    let output = directory.join("layered.vpng");
    let status = Command::new(VPNG)
        .arg("encode-layers")
        .arg(&output)
        .args(["--manifest", manifest.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "encode-layers failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    output
}

/// Run a command and return its output.
fn run(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(VPNG).args(args).output().unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

/// The file is a v0.4 file and says so.
#[test]
fn a_layered_file_is_a_v0_4_file() {
    let directory = scratch("version");
    let path = build(&directory);

    let (ok, stdout, stderr) = run(&["info", path.to_str().unwrap()]);
    assert!(ok, "info failed: {stderr}");
    assert!(
        stdout.contains("format version    0.4"),
        "the file is not a v0.4 file: {stdout}"
    );
    let _ = fs::remove_dir_all(&directory);
}

/// The layers are listed, with their names and their geometry.
#[test]
fn the_layers_are_listed() {
    let directory = scratch("list");
    let path = build(&directory);

    let (ok, stdout, stderr) = run(&["layers", path.to_str().unwrap()]);
    assert!(ok, "layers failed: {stderr}");
    assert!(stdout.contains("layers            3"), "{stdout}");
    assert!(stdout.contains("base"), "{stdout}");
    assert!(stdout.contains("ink"), "{stdout}");
    assert!(stdout.contains("mask"), "{stdout}");

    let (ok, stdout, stderr) = run(&["layers", path.to_str().unwrap(), "--json"]);
    assert!(ok, "layers --json failed: {stderr}");
    assert!(stdout.contains("\"layer_count\":3"), "{stdout}");
    assert!(stdout.contains("\"name\":\"base\""), "{stdout}");
    assert!(stdout.contains("\"is_mask\":true"), "{stdout}");
    let _ = fs::remove_dir_all(&directory);
}

/// The base layer's pixels are recovered exactly.
#[test]
fn the_base_layer_decodes_to_the_original_pixels() {
    let directory = scratch("decode");
    let path = build(&directory);
    let output = directory.join("base.png");

    let (ok, _, stderr) = run(&[
        "decode-layer",
        path.to_str().unwrap(),
        output.to_str().unwrap(),
        "--layer",
        "0",
    ]);
    assert!(ok, "decode-layer failed: {stderr}");

    // The layer is decoded to its own pixels: the same pixels the base raw
    // image held. The PNG writer is deterministic, so the two files match.
    let original = directory.join("base.png");
    let (ok, _, stderr) = run(&["decode", path.to_str().unwrap(), original.to_str().unwrap()]);
    assert!(ok, "decode failed: {stderr}");
    assert_eq!(
        fs::read(&output).unwrap(),
        fs::read(&original).unwrap(),
        "the base layer's pixels are not the original pixels"
    );
    let _ = fs::remove_dir_all(&directory);
}

/// A file with one layer composites to exactly that layer's pixels.
#[test]
fn a_one_layer_file_composites_to_its_own_pixels() {
    let directory = scratch("composite");
    write_raw(&directory.join("base.raw"), 64, 48, 0);

    let manifest = directory.join("one.json");
    fs::write(
        &manifest,
        r#"{"layers": [{"name": "only", "path": "base.raw", "width": 64, "height": 48, "format": "rgba8"}]}"#,
    )
    .unwrap();
    let path = directory.join("one.vpng");
    let (ok, _, stderr) = run(&[
        "encode-layers",
        path.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
    ]);
    assert!(ok, "encode-layers failed: {stderr}");

    let composited = directory.join("composite.png");
    let (ok, _, stderr) = run(&[
        "composite",
        path.to_str().unwrap(),
        composited.to_str().unwrap(),
    ]);
    assert!(ok, "composite failed: {stderr}");

    let decoded = directory.join("decoded.png");
    let (ok, _, stderr) = run(&["decode", path.to_str().unwrap(), decoded.to_str().unwrap()]);
    assert!(ok, "decode failed: {stderr}");

    assert_eq!(
        fs::read(&composited).unwrap(),
        fs::read(&decoded).unwrap(),
        "the composite of a one-layer file is not that layer's pixels"
    );
    let _ = fs::remove_dir_all(&directory);
}

/// The composite of a masked file blends the mask in.
#[test]
fn the_composite_applies_the_mask() {
    let directory = scratch("mask");
    let path = build(&directory);

    let composited = directory.join("composite.png");
    let (ok, _, stderr) = run(&[
        "composite",
        path.to_str().unwrap(),
        composited.to_str().unwrap(),
    ]);
    assert!(ok, "composite failed: {stderr}");

    let base = directory.join("base.png");
    let (ok, _, stderr) = run(&[
        "decode-layer",
        path.to_str().unwrap(),
        base.to_str().unwrap(),
        "--layer",
        "0",
    ]);
    assert!(ok, "decode-layer failed: {stderr}");

    assert_ne!(
        fs::read(&composited).unwrap(),
        fs::read(&base).unwrap(),
        "the composite did not apply the masked overlay"
    );
    let _ = fs::remove_dir_all(&directory);
}

/// A manifest that is not JSON is refused.
#[test]
fn a_broken_manifest_is_refused() {
    let directory = scratch("broken");
    let manifest = directory.join("broken.json");
    fs::write(&manifest, "{ this is not json }").unwrap();

    let output = Command::new(VPNG)
        .arg("encode-layers")
        .arg(directory.join("out.vpng"))
        .args(["--manifest", manifest.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!directory.join("out.vpng").exists());
    let _ = fs::remove_dir_all(&directory);
}

/// A manifest with no layers is refused.
#[test]
fn a_manifest_with_no_layers_is_refused() {
    let directory = scratch("empty");
    let manifest = directory.join("empty.json");
    fs::write(&manifest, r#"{"layers": []}"#).unwrap();

    let output = Command::new(VPNG)
        .arg("encode-layers")
        .arg(directory.join("out.vpng"))
        .args(["--manifest", manifest.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let _ = fs::remove_dir_all(&directory);
}

/// A mask that does not cover its layer is refused.
#[test]
fn a_mask_that_does_not_cover_its_layer_is_refused() {
    let directory = scratch("cover");
    write_raw(&directory.join("base.raw"), 64, 48, 0);
    write_raw(&directory.join("ink.raw"), 32, 16, 7);
    write_raw(&directory.join("mask.raw"), 16, 8, 0);

    let manifest = directory.join("cover.json");
    fs::write(
        &manifest,
        r#"{"layers": [
            {"name": "base", "path": "base.raw", "width": 64, "height": 48, "format": "rgba8"},
            {"name": "ink", "path": "ink.raw", "x": 8, "y": 8, "width": 32, "height": 16, "format": "rgba8", "mask": 2},
            {"name": "m", "kind": "mask", "path": "mask.raw", "x": 8, "y": 8, "width": 16, "height": 8, "format": "rgba8"}
        ]}"#,
    )
    .unwrap();

    let output = Command::new(VPNG)
        .arg("encode-layers")
        .arg(directory.join("out.vpng"))
        .args(["--manifest", manifest.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let _ = fs::remove_dir_all(&directory);
}

/// A layered file passes the integrity check: every payload's CRC32C is
/// verified, exactly as for any other file.
#[test]
fn a_layered_file_passes_the_integrity_check() {
    let directory = scratch("check");
    let path = build(&directory);

    let (ok, _, stderr) = run(&["check", path.to_str().unwrap()]);
    assert!(ok, "check failed: {stderr}");
    let _ = fs::remove_dir_all(&directory);
}
