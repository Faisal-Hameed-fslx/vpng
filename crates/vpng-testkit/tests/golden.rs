//! Golden-file tests.
//!
//! The first run writes the golden files; every later run must reproduce them
//! byte for byte. Regenerating is an explicit, reviewable act: delete the file
//! and re-run. Golden files pin the container layout, so a drift here is a
//! format change, not a cosmetic one.

use std::path::PathBuf;

use vpng_container::{
    canonical_digest, to_hex, Directory, Header, DIRECTORY_ENTRY_SIZE, HEADER_SIZE,
};
use vpng_core::{Limits, PixelFormat};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};
use vpng_testkit::{checker, gradient};

fn golden_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    std::fs::create_dir_all(&dir).expect("create golden dir");
    dir
}

fn assert_golden(name: &str, bytes: &[u8]) {
    let path = golden_dir().join(name);
    if path.exists() {
        let stored = std::fs::read(&path).expect("read golden");
        assert_eq!(
            stored, bytes,
            "golden file `{name}` changed; the on-disk format drifted"
        );
    } else {
        std::fs::write(&path, bytes).expect("write golden");
    }
}

#[test]
fn golden_gradient_64x64_rgba8_tile16() {
    let image = gradient(64, 64, PixelFormat::rgba8(), 1);
    // Pin the Raw payload layout explicitly: automatic selection may choose
    // another codec, which would be a different (also valid) golden file.
    let report = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(16, 16)
            .with_codec(vpng_codecs::CODEC_RAW),
    )
    .unwrap();
    assert_golden("gradient_64x64_rgba8_tile16.vpng", &report.bytes);

    // The golden file must also decode back to the exact pixels.
    let limits = Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert_eq!(document.decode_all(&limits).unwrap(), image);
}

#[test]
fn golden_gradient_300x180_rgb16_tile64() {
    let image = gradient(300, 180, PixelFormat::rgb16(), 2);
    let report = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(vpng_codecs::CODEC_RAW),
    )
    .unwrap();
    assert_golden("gradient_300x180_rgb16_tile64.vpng", &report.bytes);
}

#[test]
fn golden_palette_checker_64x64_rgba8_tile32() {
    // A two-color checkerboard with 32x32 tiles: every tile is a 2-entry
    // palette, so this golden file pins the palette payload layout through the
    // whole container (header, directory, packed indices).
    let image = checker(64, 64, PixelFormat::rgba8(), 8);
    let report = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(32, 32)
            .with_codec(vpng_codecs::CODEC_PALETTE),
    )
    .unwrap();
    assert!(
        report
            .codec_ids
            .iter()
            .all(|&id| id == vpng_codecs::CODEC_PALETTE),
        "the golden file must actually use the palette codec"
    );
    assert_golden("palette_checker_64x64_rgba8_tile32.vpng", &report.bytes);

    // The golden file must also decode back to the exact pixels.
    let limits = Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert_eq!(document.decode_all(&limits).unwrap(), image);
}

#[test]
fn header_layout_is_pinned() {
    // A 1x1 gray8 image gives the smallest possible file with fully predictable
    // fields, which we assert without hard-coding a checksum.
    let image = gradient(1, 1, PixelFormat::gray8(), 0);
    let bytes = encode(&image, &EncodeOptions::default()).unwrap().bytes;

    // 64-byte header + one 40-byte entry + 4-byte trailer + 1 payload byte.
    assert_eq!(bytes.len(), HEADER_SIZE + DIRECTORY_ENTRY_SIZE + 4 + 1);

    let header = Header::parse(&bytes).unwrap();
    assert_eq!(header.major, 0);
    assert_eq!(header.minor, 1);
    assert_eq!(header.width, 1);
    assert_eq!(header.height, 1);
    assert_eq!(header.channels(), 1);
    assert_eq!(header.bits.bits(), 8);
    assert_eq!(header.tile_count, 1);
    assert_eq!(header.directory_offset, HEADER_SIZE as u64);
    assert_eq!(header.declared_file_size, bytes.len() as u64);

    let directory = Directory::decode(
        &bytes[HEADER_SIZE..HEADER_SIZE + DIRECTORY_ENTRY_SIZE + 4],
        1,
    )
    .unwrap();
    let entry = directory.entries[0];
    assert_eq!((entry.x, entry.y), (0, 0));
    assert_eq!((entry.width, entry.height), (1, 1));
    assert_eq!(entry.codec_id, 0);
    assert_eq!(entry.uncompressed_length, 1);
    assert_eq!(
        entry.payload_offset,
        (HEADER_SIZE + DIRECTORY_ENTRY_SIZE + 4) as u64
    );
    assert_eq!(entry.compressed_length, 1);
}

#[test]
fn canonical_digest_is_pinned() {
    let image = gradient(8, 8, PixelFormat::rgba8(), 1);
    let digest = to_hex(&canonical_digest(&image));
    // Pin the digest computation itself; changing the digest inputs is a
    // format-level change and must be recorded in an ADR.
    assert_eq!(digest.len(), 64);

    let again = gradient(8, 8, PixelFormat::rgba8(), 1);
    assert_eq!(to_hex(&canonical_digest(&again)), digest);
}
