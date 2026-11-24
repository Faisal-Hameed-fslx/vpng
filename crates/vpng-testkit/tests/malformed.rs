//! Malformed-input tests.
//!
//! Every mutation must produce a typed error and must never panic. This is the
//! v0.1 stand-in for full fuzzing (see `docs/THREAT_MODEL.md`).

use vpng_container::{Directory, HEADER_SIZE};
use vpng_core::{Limits, PixelFormat};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};
use vpng_testkit::gradient;

fn valid_file() -> Vec<u8> {
    let image = gradient(128, 96, PixelFormat::rgba8(), 1);
    // Force Raw so this suite exercises the container structure, not codec
    // selection; palette robustness is covered in `palette.rs`.
    encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(vpng_codecs::CODEC_RAW),
    )
    .unwrap()
    .bytes
}

fn expect_rejected(bytes: &[u8], what: &str) {
    let limits = Limits::default();
    match Document::parse(bytes, &limits) {
        Err(_) => {}
        Ok(document) => {
            // Some corruption only shows up when pixels are actually decoded.
            if document.decode_all(&limits).is_ok() {
                panic!("corruption not detected: {what}");
            }
        }
    }
}

#[test]
fn control_file_is_valid() {
    let bytes = valid_file();
    let limits = Limits::default();
    let document = Document::parse(&bytes, &limits).expect("control file must parse");
    assert!(document.decode_all(&limits).is_ok());
}

#[test]
fn bad_magic_is_rejected() {
    let mut bytes = valid_file();
    bytes[0] ^= 0xff;
    expect_rejected(&bytes, "flipped magic byte");
}

#[test]
fn truncated_file_is_rejected() {
    let bytes = valid_file();
    expect_rejected(&bytes[..bytes.len() - 1], "truncated payload");
    expect_rejected(&bytes[..HEADER_SIZE - 1], "truncated header");
    expect_rejected(&bytes[..HEADER_SIZE + 8], "truncated directory");
}

#[test]
fn empty_file_is_rejected() {
    expect_rejected(&[], "empty file");
}

#[test]
fn header_field_mutations_are_rejected() {
    // width, height, tile count, directory offset, declared size, checksum
    for offset in [16usize, 20, 28, 32, 40, 52] {
        let mut bytes = valid_file();
        bytes[offset] ^= 0x01;
        expect_rejected(&bytes, &format!("header byte {offset}"));
    }
}

#[test]
fn directory_corruption_is_rejected() {
    let mut bytes = valid_file();
    // First directory entry begins right after the header.
    bytes[HEADER_SIZE + 3] ^= 0x08;
    expect_rejected(&bytes, "directory entry mutation");
}

#[test]
fn directory_checksum_trailer_is_checked() {
    let mut bytes = valid_file();
    // 4 tiles -> directory is 4 * 40 + 4 bytes; flip the trailer.
    let trailer = HEADER_SIZE + 4 * 40;
    bytes[trailer] ^= 0xff;
    expect_rejected(&bytes, "directory trailer");
}

#[test]
fn payload_corruption_is_caught_by_checksum() {
    let mut bytes = valid_file();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    expect_rejected(&bytes, "payload byte flip");
}

#[test]
fn forged_tile_count_is_rejected() {
    let mut bytes = valid_file();
    // Force tile_count to a large value.
    bytes[28] = 0xff;
    bytes[29] = 0xff;
    expect_rejected(&bytes, "forged tile count");
}

#[test]
fn unknown_codec_id_is_unsupported() {
    let mut bytes = valid_file();
    // Set codec_id (byte 12 of the first directory entry) to 7, which no
    // version has ever defined, then recompute the directory CRC so that only
    // the codec rule can fire and not the checksum rule.
    bytes[HEADER_SIZE + 12] = 7;
    let tile_count = u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);
    let size = Directory::total_size(tile_count).unwrap();
    let body_len = size - 4;
    let crc = vpng_container::crc32c(&bytes[HEADER_SIZE..HEADER_SIZE + body_len]);
    bytes[HEADER_SIZE + body_len..HEADER_SIZE + body_len + 4].copy_from_slice(&crc.to_le_bytes());

    let limits = Limits::default();
    let err = Document::parse(&bytes, &limits).unwrap_err();
    assert_eq!(err.kind(), vpng_core::ErrorKind::UnsupportedFeature);
}

#[test]
fn a_v0_1_file_may_not_name_a_v0_3_codec() {
    let mut bytes = valid_file();
    // Codec id 4 (the LZ77 dictionary) was promoted in v0.3. A file whose
    // minor version is 1 never defined it, so the file is malformed.
    bytes[HEADER_SIZE + 12] = 4;
    let tile_count = u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);
    let size = Directory::total_size(tile_count).unwrap();
    let body_len = size - 4;
    let crc = vpng_container::crc32c(&bytes[HEADER_SIZE..HEADER_SIZE + body_len]);
    bytes[HEADER_SIZE + body_len..HEADER_SIZE + body_len + 4].copy_from_slice(&crc.to_le_bytes());

    let limits = Limits::default();
    let err = Document::parse(&bytes, &limits).unwrap_err();
    assert_eq!(err.kind(), vpng_core::ErrorKind::UnsupportedFeature);
}

#[test]
fn byte_truncating_sweep_never_panics() {
    // Systematically drop bytes and ensure we always get a Result, never a panic.
    let bytes = valid_file();
    for len in 0..bytes.len() {
        let limits = Limits::default();
        let _ = Document::parse(&bytes[..len], &limits);
    }
}

#[test]
fn single_byte_flip_sweep_never_panics() {
    let bytes = valid_file();
    let limits = Limits::default();
    // Sample the file at a fixed stride to keep the test fast but broad.
    let mut index = 0usize;
    while index < bytes.len() {
        let mut mutated = bytes.clone();
        mutated[index] ^= 0x5a;
        if let Ok(document) = Document::parse(&mutated, &limits) {
            let _ = document.decode_all(&limits);
        }
        index += 7;
    }
}
