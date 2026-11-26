//! The command line's HTTP commands must read the same pixels as the local
//! ones, and must refuse a server that does not honour `Range`.
//!
//! The commands under test are `vpng http-info` and
//! `vpng http-decode-region`. Both go through the real `vpng-http` source
//! against the hand-written HTTP/1.1 server, so the test exercises the
//! whole path: the HTTP request, the range check, the container validation,
//! the codec dispatch and the region assembly.
//!
//! The comparison is against the local command, on the same file, so a
//! difference would show up as a different digest.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use vpng_http::test_server::{Behaviour, TestServer};

/// The `vpng` binary, built by Cargo for this test.
const VPNG: &str = env!("CARGO_BIN_EXE_vpng");

/// A temporary directory for one test.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "vpng-cli-http-{name}-{}-{}",
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

/// Encode the raw image with the command line and return the file's path.
fn encode(directory: &Path) -> PathBuf {
    let input = directory.join("input.raw");
    let output = directory.join("picture.vpng");
    fs::write(&input, raw_pixels()).unwrap();

    let status = Command::new(VPNG)
        .arg("encode")
        .arg(&input)
        .arg(&output)
        .args(["--input-format", "raw"])
        .args(["--format", "rgb8"])
        .args(["--width", "300"])
        .args(["--height", "200"])
        .args(["--tile-size", "64"])
        .status()
        .unwrap();
    assert!(status.success(), "the encode failed");
    output
}

/// Pull one field out of the JSON the commands print.
fn field(text: &str, name: &str) -> String {
    let needle = format!("\"{name}\":");
    let start = text.find(&needle).expect("the field is not in the output");
    let rest = &text[start + needle.len()..];
    let rest = rest.trim_start();
    if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"').expect("the string is not closed");
        stripped[..end].to_string()
    } else {
        let end = rest.find([',', '}']).unwrap_or(rest.len());
        rest[..end].trim().to_string()
    }
}

/// `vpng http-info` reports the container and what it cost to fetch it.
#[test]
fn http_info_reports_the_container_without_fetching_the_payloads() {
    let directory = scratch("info");
    let path = encode(&directory);
    let server = TestServer::start(fs::read(&path).unwrap(), Behaviour::Ranges);

    let output = Command::new(VPNG)
        .arg("http-info")
        .arg(server.url())
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "http-info failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);

    assert_eq!(field(&text, "width"), "300");
    assert_eq!(field(&text, "height"), "200");
    assert_eq!(field(&text, "major"), "0");
    assert_eq!(field(&text, "minor"), "1");

    let object_size: u64 = field(&text, "object_size").parse().unwrap();
    let bytes_read: u64 = field(&text, "bytes_read").parse().unwrap();
    assert_eq!(
        object_size,
        fs::metadata(&path).unwrap().len(),
        "the reported object size is not the file's size"
    );
    assert!(
        bytes_read < object_size,
        "http-info fetched the whole object: {bytes_read} of {object_size}"
    );
    // The header is 64 bytes and the directory is 40 bytes per tile; the
    // payloads are the rest of the file.
    assert!(
        bytes_read < 1024,
        "http-info fetched more than the header and the directory: {bytes_read}"
    );

    let _ = fs::remove_dir_all(&directory);
}

/// The region read over HTTP is the region read from the local file.
#[test]
fn http_decode_region_matches_the_local_read() {
    let directory = scratch("region");
    let path = encode(&directory);
    let server = TestServer::start(fs::read(&path).unwrap(), Behaviour::Ranges);

    let remote_png = directory.join("remote.png");
    let output = Command::new(VPNG)
        .arg("http-decode-region")
        .arg(server.url())
        .arg(&remote_png)
        .args(["--x", "64", "--y", "32", "--width", "128", "--height", "96"])
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "http-decode-region failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let remote_text = String::from_utf8_lossy(&output.stdout);

    let local_png = directory.join("local.png");
    let output = Command::new(VPNG)
        .arg("decode-region")
        .arg(&path)
        .arg(&local_png)
        .args(["--x", "64", "--y", "32", "--width", "128", "--height", "96"])
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "decode-region failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let local_text = String::from_utf8_lossy(&output.stdout);

    assert_eq!(
        field(&remote_text, "sha256"),
        field(&local_text, "sha256"),
        "the region read over HTTP is not the region read from the local file"
    );
    assert_eq!(
        fs::read(&remote_png).unwrap(),
        fs::read(&local_png).unwrap(),
        "the two PNG files differ"
    );

    let _ = fs::remove_dir_all(&directory);
}

/// A server that ignores `Range` is refused, not quietly turned into a
/// whole-file download.
#[test]
fn a_server_that_ignores_range_is_refused() {
    let directory = scratch("ignore");
    let path = encode(&directory);
    let server = TestServer::start(fs::read(&path).unwrap(), Behaviour::IgnoreRanges);

    let output = Command::new(VPNG)
        .arg("http-info")
        .arg(server.url())
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "a server that ignores Range was accepted"
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "the exit code is not the unsupported-feature one: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Range"),
        "the error does not mention the range request: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = fs::remove_dir_all(&directory);
}

/// A server that answers with the wrong range is refused.
#[test]
fn a_server_that_answers_the_wrong_range_is_refused() {
    let directory = scratch("wrong");
    let path = encode(&directory);
    let server = TestServer::start(fs::read(&path).unwrap(), Behaviour::WrongRange);

    let output = Command::new(VPNG)
        .arg("http-info")
        .arg(server.url())
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "a server that answers the wrong range was accepted"
    );
    assert_eq!(
        output.status.code(),
        Some(4),
        "the exit code is not the corrupt-file one: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = fs::remove_dir_all(&directory);
}
