//! The HTTP source, tested against a server it does not control.
//!
//! The questions the tests answer are the ones a random-access reader
//! depends on:
//!
//! * Does the reader fetch **only** the bytes it needs?
//! * Does it refuse a server that ignores `Range`?
//! * Does it refuse a server that answers with the wrong range?
//! * Does it refuse a truncated body?
//! * Are the pixels it returns the same pixels the local file has?

use std::path::PathBuf;

use vpng_core::{ErrorKind, Limits, PixelFormat, Rect};
use vpng_source::{ByteSource, RangeReader};

use crate::source::{HttpSource, HttpSourceOptions};
use crate::test_server::{Behaviour, TestServer};

/// The limits the tests use.
fn limits() -> Limits {
    Limits::default()
}

/// Write a small image to a temporary file and return its path.
fn write_container(name: &str, pixels: &[u8], width: u32, height: u32) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vpng-http-{name}"));
    std::fs::create_dir_all(&dir).expect("the temporary directory can be created");
    let path = dir.join("picture.vpng");
    let image =
        vpng_core::CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), pixels.to_vec())
            .expect("the canonical image is well formed");
    vpng_encoder::stream::encode_to_file(&image, &Default::default(), &Default::default(), &path)
        .expect("the container can be written");
    path
}

/// A deterministic little image.
fn image_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.push((x % 251) as u8);
            pixels.push((y % 241) as u8);
            pixels.push(((x + y) % 239) as u8);
            pixels.push(255);
        }
    }
    pixels
}

#[test]
fn the_reader_fetches_only_the_bytes_it_needs() {
    let width = 300u32;
    let height = 200u32;
    let pixels = image_bytes(width, height);
    let path = write_container("bytes", &pixels, width, height);
    let file = std::fs::read(&path).expect("the container can be read");

    let server = TestServer::start(file.clone(), Behaviour::Ranges);
    let source = HttpSource::open(&server.url(), HttpSourceOptions::default())
        .expect("the source opens over HTTP");
    let mut reader = RangeReader::open_source(source, &limits()).expect("the container validates");

    // The header and the directory were fetched; nothing else.
    let after_open = reader.bytes_read();
    assert!(after_open < file.len() as u64);
    assert!(after_open >= 64);

    let before = server.bytes_sent();
    let tile = reader.decode_tile(0, &limits()).expect("the tile decodes");
    let payload = server.bytes_sent() - before;

    // One tile's payload, and no more. The file is much larger than the
    // tile, and the whole file was never fetched.
    assert!(payload < file.len() as usize);
    assert!(payload > 0);
    assert!(server.bytes_sent() < file.len());
    assert!(!tile.is_empty());
}

#[test]
fn the_pixels_over_http_are_the_pixels_on_disk() {
    let width = 300u32;
    let height = 200u32;
    let pixels = image_bytes(width, height);
    let path = write_container("pixels", &pixels, width, height);
    let file = std::fs::read(&path).expect("the container can be read");

    let server = TestServer::start(file, Behaviour::Ranges);
    let source = HttpSource::open(&server.url(), HttpSourceOptions::default())
        .expect("the source opens over HTTP");
    let mut reader = RangeReader::open_source(source, &limits()).expect("the container validates");

    let region = Rect::new(10, 20, 64, 48).expect("the rectangle is well formed");
    let over_http = reader
        .decode_region(region, &limits())
        .expect("the region decodes");

    let mut local = RangeReader::open(&path, &limits()).expect("the local container opens");
    let on_disk = local
        .decode_region(region, &limits())
        .expect("the region decodes");

    // The expected region, built row by row: the source's rows are wider
    // than the region, so the region is not one contiguous run of bytes.
    let mut expected = Vec::with_capacity(48 * 64 * 4);
    for y in 20..20 + 48 {
        let start = (y * width as usize + 10) * 4;
        expected.extend_from_slice(&pixels[start..start + 64 * 4]);
    }

    assert_eq!(over_http.data(), on_disk.data());
    assert_eq!(over_http.data(), &expected[..]);
}

#[test]
fn a_server_that_ignores_range_is_refused() {
    let server = TestServer::start(vec![7u8; 4096], Behaviour::IgnoreRanges);
    let error = HttpSource::open(&server.url(), HttpSourceOptions::default())
        .expect_err("a server that ignores Range is refused");
    assert_eq!(error.kind(), ErrorKind::UnsupportedFeature);
    assert!(error.to_string().contains("does not honour HTTP Range"));
}

#[test]
fn a_server_that_answers_the_wrong_range_is_refused() {
    let server = TestServer::start(vec![7u8; 4096], Behaviour::WrongRange);
    let source = HttpSource::open(&server.url(), HttpSourceOptions::default())
        .expect("the source opens: the length probe is a range of one byte");
    let mut source = source;
    let error = source
        .read_at(10, 16)
        .expect_err("a server that answers the wrong range is refused");
    assert_eq!(error.kind(), ErrorKind::CorruptFile);
    assert!(error.to_string().contains("answered the range"));
}

#[test]
fn a_truncated_body_is_refused() {
    let server = TestServer::start(vec![7u8; 4096], Behaviour::ShortBody);
    let source = HttpSource::open(&server.url(), HttpSourceOptions::default())
        .expect("the source opens: the length probe is a range of one byte");
    let mut source = source;
    let error = source
        .read_at(10, 16)
        .expect_err("a truncated body is refused");
    assert_eq!(error.kind(), ErrorKind::CorruptFile);
}

#[test]
fn a_server_that_refuses_head_still_works() {
    let server = TestServer::start(vec![7u8; 4096], Behaviour::NoHead);
    let source = HttpSource::open(&server.url(), HttpSourceOptions::default())
        .expect("a server that refuses HEAD is still usable");
    assert_eq!(source.length(), 4096);
}

#[test]
fn a_range_beyond_the_object_is_refused() {
    let server = TestServer::start(vec![7u8; 4096], Behaviour::Ranges);
    let mut source = HttpSource::open(&server.url(), HttpSourceOptions::default())
        .expect("the source opens over HTTP");
    let error = source
        .read_at(4000, 100)
        .expect_err("a range past the end is refused");
    assert_eq!(error.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn a_large_range_is_fetched_in_several_requests() {
    let server = TestServer::start(vec![7u8; 4096], Behaviour::Ranges);
    let options = HttpSourceOptions {
        max_range_bytes: 100,
        ..HttpSourceOptions::default()
    };
    let mut source = HttpSource::open(&server.url(), options).expect("the source opens over HTTP");
    let bytes = source.read_at(0, 1000).expect("the range is fetched");
    assert_eq!(bytes.len(), 1000);
    assert!(bytes.iter().all(|byte| *byte == 7));

    // One probe plus ten hundred-byte ranges, in hundred-byte pieces.
    assert!(server.range_requests() >= 10);
    assert_eq!(server.bytes_sent(), 1 + 1000);
}

#[test]
fn the_reader_reports_what_it_read() {
    let width = 300u32;
    let height = 200u32;
    let pixels = image_bytes(width, height);
    let path = write_container("report", &pixels, width, height);
    let file = std::fs::read(&path).expect("the container can be read");

    let server = TestServer::start(file.clone(), Behaviour::Ranges);
    let source = HttpSource::open(&server.url(), HttpSourceOptions::default())
        .expect("the source opens over HTTP");
    let mut reader = RangeReader::open_source(source, &limits()).expect("the container validates");

    let meta = reader.meta();
    let tile_count = meta.header().tile_count as usize;
    assert!(tile_count > 1, "the test image has more than one tile");

    let mut decoded = 0usize;
    for index in 0..tile_count {
        reader
            .decode_tile(index, &limits())
            .expect("the tile decodes");
        decoded += 1;
    }

    assert_eq!(decoded, tile_count);
    assert_eq!(reader.payloads_read() as usize, tile_count);
    // The bytes read are the header, the directory and every payload: no
    // more, and never the whole file plus more.
    assert!(reader.bytes_read() <= file.len() as u64);
    assert!(reader.bytes_read() >= 64);

    // The server answered one request per range — the length probe, the
    // header, the directory and each tile — plus one `HEAD`.
    assert!(server.requests() >= tile_count);
    assert_eq!(server.requests(), server.range_requests() + 1);
}
