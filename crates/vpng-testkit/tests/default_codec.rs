//! Conformance tests for the header's `default codec id` (byte 27).
//!
//! The specification resolves byte 27 as **informational** (specification
//! section 2.3, ADR-0003): the per-tile directory entry is the authority for
//! how a tile is decoded, and the header's byte is only a hint about the file
//! as a whole. These tests pin that behaviour from both directions:
//!
//! * a reader ignores the header's byte, whatever it says -- even when it
//!   names a codec the file does not use, or a codec id that does not exist;
//! * a writer records the file's codec there when one codec applies, and
//!   records Raw when the tiles were selected individually, so the byte can
//!   never be mistaken for the authority.
//!
//! The binary layout is unchanged by any of this: the byte stays at offset 27
//! and the format version stays 0.1.

use vpng_container::{Header, HEADER_SIZE};
use vpng_core::{CanonicalImage, Limits, PixelFormat};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};
use vpng_testkit::{checker, gradient};

/// A file whose tiles are all Raw, so the only codec in play is Raw.
fn raw_file() -> Vec<u8> {
    let image = gradient(128, 96, PixelFormat::rgba8(), 1);
    encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(vpng_codecs::CODEC_RAW),
    )
    .unwrap()
    .bytes
}

/// Replace the header's default codec byte with `codec`, recomputing the
/// header checksum so the file stays structurally valid.
fn with_default_codec(bytes: &[u8], codec: u8) -> Vec<u8> {
    let mut header = Header::parse(bytes).expect("the control file must parse");
    header.default_codec = codec;
    let mut patched = bytes.to_vec();
    patched[..HEADER_SIZE].copy_from_slice(&header.to_bytes().unwrap());
    patched
}

/// The pixels a file decodes to, or a panic with the reason.
fn decoded_pixels(bytes: &[u8]) -> CanonicalImage {
    let limits = Limits::default();
    let document = Document::parse(bytes, &limits).expect("the file must parse");
    document.decode_all(&limits).expect("the file must decode")
}

#[test]
fn the_control_file_records_the_codec_it_used() {
    let bytes = raw_file();
    let header = Header::parse(&bytes).unwrap();
    // Raw was forced for every tile, so the header records Raw.
    assert_eq!(header.default_codec, vpng_codecs::CODEC_RAW);
    // And every tile says Raw in the directory, which is the authority.
    let limits = Limits::default();
    let document = Document::parse(&bytes, &limits).unwrap();
    for index in 0..document.tile_count() as usize {
        assert_eq!(
            document.tile_entry(index).unwrap().codec_id,
            vpng_codecs::CODEC_RAW,
            "tile {index} must name Raw in the directory"
        );
    }
}

#[test]
fn a_different_default_codec_does_not_change_the_pixels() {
    let original = raw_file();
    let expected = decoded_pixels(&original);

    // The header now claims Palette for a file whose tiles are Raw. If the
    // header were normative, this file would decode as Palette (or fail).
    let patched = with_default_codec(&original, vpng_codecs::CODEC_PALETTE);
    let header = Header::parse(&patched).unwrap();
    assert_eq!(header.default_codec, vpng_codecs::CODEC_PALETTE);

    let actual = decoded_pixels(&patched);
    assert_eq!(
        actual.data(),
        expected.data(),
        "the per-tile codec must decide the pixels, not the header"
    );
    assert_eq!(actual.dimensions(), expected.dimensions());
}

#[test]
fn an_unknown_default_codec_is_not_rejected() {
    let original = raw_file();
    let expected = decoded_pixels(&original);

    // 0xFE is not a codec id at all. The byte is informational, so it is not
    // validated: the file still parses and still decodes exactly.
    let patched = with_default_codec(&original, 0xFE);
    let header = Header::parse(&patched).unwrap();
    assert_eq!(header.default_codec, 0xFE);

    let actual = decoded_pixels(&patched);
    assert_eq!(
        actual.data(),
        expected.data(),
        "an unknown default codec id must not affect the decode"
    );
}

#[test]
fn a_reserved_default_codec_is_not_rejected() {
    let original = raw_file();
    let expected = decoded_pixels(&original);

    // 4 is reserved (LZ77 dictionary) in v0.1. A *tile* naming a reserved
    // codec is rejected with UnsupportedFeature; the header's byte is not a
    // tile, so it is accepted and ignored.
    let patched = with_default_codec(&original, 4);
    let actual = decoded_pixels(&patched);
    assert_eq!(actual.data(), expected.data());
}

#[test]
fn a_forced_codec_is_recorded_in_the_header() {
    // A checkerboard has two distinct pixel values, so the palette codec can
    // represent every tile of it.
    let image = checker(128, 96, PixelFormat::rgba8(), 16);
    let bytes = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(vpng_codecs::CODEC_PALETTE),
    )
    .unwrap()
    .bytes;
    let header = Header::parse(&bytes).unwrap();
    assert_eq!(header.default_codec, vpng_codecs::CODEC_PALETTE);

    let limits = Limits::default();
    let document = Document::parse(&bytes, &limits).unwrap();
    for index in 0..document.tile_count() as usize {
        assert_eq!(
            document.tile_entry(index).unwrap().codec_id,
            vpng_codecs::CODEC_PALETTE
        );
    }
}

#[test]
fn an_individually_selected_file_records_raw_in_the_header() {
    let image = gradient(128, 96, PixelFormat::rgba8(), 1);
    let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();
    let header = Header::parse(&report.bytes).unwrap();

    // The tiles were selected one by one, so there is no single file codec
    // and the header records Raw rather than pretending to name one.
    assert_eq!(header.default_codec, vpng_codecs::CODEC_RAW);

    // The tiles may still disagree with each other, which is exactly why the
    // header cannot be the authority.
    let limits = Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    let codecs: Vec<u8> = (0..document.tile_count() as usize)
        .map(|index| document.tile_entry(index).unwrap().codec_id)
        .collect();
    assert!(
        codecs.iter().all(|codec| *codec != vpng_codecs::CODEC_RAW),
        "the encoder should have found a smaller codec than Raw for these tiles"
    );
}

#[test]
fn the_decoded_pixels_are_exact_whatever_the_header_says() {
    let original = raw_file();
    let expected = decoded_pixels(&original);
    for codec in [0u8, 1, 2, 3, 4, 6, 0x80, 0xFE, 0xFF] {
        let patched = with_default_codec(&original, codec);
        let actual = decoded_pixels(&patched);
        assert_eq!(
            actual.data(),
            expected.data(),
            "default codec {codec} changed the decoded pixels"
        );
    }
}
