//! The command line's v0.5 surface: the GPU payload table.
//!
//! The rules the command line must keep:
//!
//! * `encode --gpu` writes a **v0.5** file that carries a GPU payload table;
//! * the lossless payload is untouched: `verify` still passes, and the
//!   decoded picture is byte for byte the original;
//! * `gpu-info` lists what the table holds;
//! * `gpu-decode` writes the **lossy** GPU payload, as RGBA8;
//! * a file without a GPU table says so, with the unsupported-feature exit
//!   code, not a silent empty result.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The `vpng` binary, built by Cargo for this test.
const VPNG: &str = env!("CARGO_BIN_EXE_vpng");

/// A temporary directory for one test.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "vpng-cli-v5-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// A deterministic raw RGB8 image.
fn raw_pixels(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        for x in 0..width {
            pixels.push((x % 251) as u8);
            pixels.push((y % 241) as u8);
            pixels.push(((((x / 16) + (y / 16)) % 2) * 255) as u8);
        }
    }
    pixels
}

/// Write a raw image.
fn write_raw(path: &Path, width: u32, height: u32) {
    fs::write(path, raw_pixels(width, height)).unwrap();
}

/// A deterministic raw RGBA8 image.
fn raw_pixels_rgba(width: u32, height: u32, seed: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            pixels.push((x % 251 + seed) as u8);
            pixels.push((y % 241) as u8);
            pixels.push(((((x / 16) + (y / 16)) % 2) * 255) as u8);
            pixels.push(0xFF);
        }
    }
    pixels
}

/// Write a raw RGBA8 image.
fn write_raw_rgba(path: &Path, width: u32, height: u32, seed: u32) {
    fs::write(path, raw_pixels_rgba(width, height, seed)).unwrap();
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

/// Run a command and return its exit code.
fn code(args: &[&str]) -> i32 {
    Command::new(VPNG)
        .args(args)
        .output()
        .unwrap()
        .status
        .code()
        .unwrap_or(-1)
}

/// Build a file, with or without a GPU payload table.
fn build(directory: &Path, gpu: bool) -> PathBuf {
    write_raw(&directory.join("in.raw"), 128, 96);
    let output = directory.join("picture.vpng");
    let input = directory.join("in.raw");
    let mut args = vec![
        "encode",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
        "--input-format",
        "raw",
        "--width",
        "128",
        "--height",
        "96",
        "--format",
        "rgb8",
        "--tile-size",
        "64",
    ];
    if gpu {
        args.push("--gpu");
    }
    let (ok, _, stderr) = run(&args);
    assert!(ok, "encode failed: {stderr}");
    output
}

#[test]
fn the_gpu_flag_writes_a_v0_5_file() {
    let directory = scratch("version");
    let path = build(&directory, true);

    let (ok, stdout, stderr) = run(&["info", path.to_str().unwrap()]);
    assert!(ok, "info failed: {stderr}");
    assert!(
        stdout.contains("format version    0.5"),
        "the file is not a v0.5 file: {stdout}"
    );
    assert!(stdout.contains("GPU table"), "{stdout}");

    let (ok, stdout, stderr) = run(&["info", path.to_str().unwrap(), "--json"]);
    assert!(ok, "info --json failed: {stderr}");
    assert!(stdout.contains("\"minor\":5"), "{stdout}");
    assert!(stdout.contains("\"gpu_table_length\":"), "{stdout}");
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn the_default_encode_is_still_a_v0_1_file() {
    let directory = scratch("default");
    let path = build(&directory, false);

    let (ok, stdout, stderr) = run(&["info", path.to_str().unwrap()]);
    assert!(ok, "info failed: {stderr}");
    assert!(
        stdout.contains("format version    0.1"),
        "the default encode moved: {stdout}"
    );
    assert!(!stdout.contains("GPU table"), "{stdout}");
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn the_lossless_pixels_still_round_trip_exactly() {
    let directory = scratch("verify");
    let path = build(&directory, true);

    let (ok, stdout, stderr) = run(&[
        "verify",
        directory.join("in.raw").to_str().unwrap(),
        path.to_str().unwrap(),
        "--input-format",
        "raw",
        "--width",
        "128",
        "--height",
        "96",
        "--format",
        "rgb8",
    ]);
    assert!(ok, "verify failed: {stderr}");
    assert!(stdout.contains("PASS"), "{stdout}");
    let _ = fs::remove_dir_all(&directory);
}
#[test]
fn the_gpu_table_is_listed() {
    let directory = scratch("info");
    let path = build(&directory, true);

    let (ok, stdout, stderr) = run(&["gpu-info", path.to_str().unwrap()]);
    assert!(ok, "gpu-info failed: {stderr}");
    assert!(stdout.contains("gpu records       4"), "{stdout}");
    assert!(stdout.contains("tiles with colour 4"), "{stdout}");
    assert!(stdout.contains("LOSSY"), "{stdout}");

    let (ok, stdout, stderr) = run(&["gpu-info", path.to_str().unwrap(), "--json"]);
    assert!(ok, "gpu-info --json failed: {stderr}");
    assert!(stdout.contains("\"record_count\":4"), "{stdout}");
    assert!(stdout.contains("\"colour_codec\":7"), "{stdout}");
    assert!(stdout.contains("\"lossy\":true"), "{stdout}");
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn the_gpu_payload_decodes_to_a_lossy_rgba8_image() {
    let directory = scratch("decode");
    let path = build(&directory, true);
    let output = directory.join("gpu.png");

    let (ok, stdout, stderr) = run(&[
        "gpu-decode",
        path.to_str().unwrap(),
        output.to_str().unwrap(),
        "--x",
        "0",
        "--y",
        "0",
        "--width",
        "128",
        "--height",
        "96",
    ]);
    assert!(ok, "gpu-decode failed: {stderr}");
    assert!(stdout.contains("LOSSY"), "{stdout}");
    assert!(output.exists(), "the GPU payload was not written");

    // The GPU payload is a different picture from the exact one.
    let exact = directory.join("exact.png");
    let (ok, _, stderr) = run(&["decode", path.to_str().unwrap(), exact.to_str().unwrap()]);
    assert!(ok, "decode failed: {stderr}");
    assert_ne!(
        fs::read(&output).unwrap(),
        fs::read(&exact).unwrap(),
        "the GPU payload is identical to the exact picture, so it is not lossy"
    );
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn a_file_without_a_gpu_table_says_so() {
    let directory = scratch("none");
    let path = build(&directory, false);

    // The unsupported-feature exit code is 3.
    assert_eq!(code(&["gpu-info", path.to_str().unwrap()]), 3);
    assert_eq!(
        code(&[
            "gpu-decode",
            path.to_str().unwrap(),
            directory.join("gpu.png").to_str().unwrap(),
            "--x",
            "0",
            "--y",
            "0",
            "--width",
            "128",
            "--height",
            "96",
        ]),
        3
    );
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn a_gpu_file_passes_the_integrity_check() {
    let directory = scratch("check");
    let path = build(&directory, true);

    let (ok, stdout, stderr) = run(&["check", path.to_str().unwrap()]);
    assert!(ok, "check failed: {stderr}");
    assert!(stdout.contains("OK"), "{stdout}");
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn the_worker_count_does_not_change_the_bytes() {
    let directory = scratch("workers");
    write_raw(&directory.join("in.raw"), 128, 96);

    let mut digests = Vec::new();
    for workers in ["1", "2", "4", "8"] {
        let output = directory.join(format!("many-{workers}.vpng"));
        let (ok, _, stderr) = run(&[
            "encode",
            directory.join("in.raw").to_str().unwrap(),
            output.to_str().unwrap(),
            "--input-format",
            "raw",
            "--width",
            "128",
            "--height",
            "96",
            "--format",
            "rgb8",
            "--tile-size",
            "64",
            "--gpu",
            "--workers",
            workers,
        ]);
        assert!(ok, "encode failed: {stderr}");
        digests.push(vpng_container::sha256(&fs::read(&output).unwrap()));
    }

    for digest in &digests {
        assert_eq!(digest, &digests[0], "the worker count changed the bytes");
    }
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn a_layered_gpu_file_carries_the_masks_coverage_alone() {
    // A **mask** layer's colour channels carry no meaning: its coverage is
    // its alpha channel. Its GPU payload therefore holds the coverage alone,
    // in BC4 blocks, with no colour blocks at all.
    let directory = scratch("layers");
    write_raw_rgba(&directory.join("base.raw"), 64, 48, 0);
    write_raw_rgba(&directory.join("ink.raw"), 32, 16, 7);
    write_raw_rgba(&directory.join("mask.raw"), 32, 16, 0);

    let manifest = directory.join("layers.json");
    fs::write(
        &manifest,
        r#"{
  "layers": [
    {"name": "base", "path": "base.raw", "width": 64, "height": 48, "format": "rgba8"},
    {"name": "m", "kind": "mask", "path": "mask.raw", "x": 8, "y": 8, "width": 32, "height": 16, "format": "rgba8"}
  ]
}
"#,
    )
    .unwrap();

    let output = directory.join("layered.vpng");
    let (ok, stdout, stderr) = run(&[
        "encode-layers",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--tile-size",
        "32",
        "--gpu",
        "--json",
    ]);
    assert!(ok, "encode-layers failed: {stderr}");
    assert!(stdout.contains("\"gpu\":true"), "{stdout}");

    let (ok, stdout, stderr) = run(&["gpu-info", output.to_str().unwrap(), "--json"]);
    assert!(ok, "gpu-info failed: {stderr}");
    assert!(stdout.contains("\"minor\":5"), "{stdout}");

    // The base layer's tile comes first, and the mask layer's tile after it.
    let records: Vec<&str> = stdout
        .split("\"colour_codec\":")
        .skip(1)
        .map(|part| part.split(',').next().unwrap())
        .collect();
    assert!(
        records.len() >= 2,
        "the table holds too few records: {stdout}"
    );

    let alphas: Vec<&str> = stdout
        .split("\"alpha_codec\":")
        .skip(1)
        .map(|part| part.split(',').next().unwrap())
        .collect();
    assert_eq!(
        alphas.len(),
        records.len(),
        "the records disagree: {stdout}"
    );

    // Every base-layer tile carries BC1 colour and BC4 alpha.
    for record in &records[..records.len() - 1] {
        assert_eq!(
            *record, "7",
            "a base layer's tile has no BC1 blocks: {stdout}"
        );
    }
    for alpha in &alphas[..alphas.len() - 1] {
        assert_eq!(
            *alpha, "8",
            "a base layer's tile has no BC4 blocks: {stdout}"
        );
    }

    // The mask layer's tile carries the coverage alone.
    assert_eq!(
        records[records.len() - 1],
        "0",
        "a mask's tile must carry no colour blocks: {stdout}"
    );
    assert_eq!(
        alphas[alphas.len() - 1],
        "8",
        "a mask's tile must carry its coverage in BC4 blocks: {stdout}"
    );

    // And the file is still exact.
    let (ok, stdout, stderr) = run(&["check", output.to_str().unwrap()]);
    assert!(ok, "check failed: {stderr}");
    assert!(stdout.contains("OK"), "{stdout}");
    let _ = fs::remove_dir_all(&directory);
}
