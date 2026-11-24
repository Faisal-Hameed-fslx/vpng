//! The v0.3 file format: the promoted codecs.
//!
//! These tests pin the rules that make a v0.3 file safe:
//!
//! * a file that names codec id 4, 5 or 6 says so in its minor version;
//! * a file whose minor version is below 3 may **not** name one of them, and
//!   is rejected with `UnsupportedFeature` rather than decoded best-effort;
//! * a v0.3 file that names one of them decodes **exactly**;
//! * the version states what the file **contains**, not what the encoder was
//!   allowed to do;
//! * the research ids may never reach a file;
//! * the default path still writes the v0.1 bytes.

use vpng_container::{Directory, Header, HEADER_SIZE, MINOR_V0_1, MINOR_V0_2, MINOR_V0_3};
use vpng_core::{CanonicalImage, ErrorKind, Limits, PixelFormat};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};

/// A picture with enough structure that every codec has something to chew on.
fn picture(width: u32, height: u32) -> CanonicalImage {
    let bpp = 4usize;
    let mut data = vec![0u8; (width * height) as usize * bpp];
    let mut state = 0x1234_5678_9ABC_DEF0u64;
    for y in 0..height {
        for x in 0..width {
            let base = ((y * width + x) as usize) * bpp;
            if x < width / 2 {
                data[base] = 0x20;
                data[base + 1] = 0x30;
                data[base + 2] = 0x40;
            } else {
                for c in 0..3 {
                    state ^= state >> 12;
                    state ^= state << 25;
                    state ^= state >> 27;
                    data[base + c] = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as u8;
                }
            }
            data[base + 3] = 0xFF;
        }
    }
    CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), data).unwrap()
}

/// Rewrite the directory's CRC trailer, so a mutation of an entry is judged
/// by the codec rule and not by the checksum rule.
fn refresh_directory_crc(bytes: &mut [u8]) {
    let tile_count = u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);
    let size = Directory::total_size(tile_count).unwrap();
    let body_len = size - 4;
    let crc = vpng_container::crc32c(&bytes[HEADER_SIZE..HEADER_SIZE + body_len]);
    bytes[HEADER_SIZE + body_len..HEADER_SIZE + body_len + 4].copy_from_slice(&crc.to_le_bytes());
}

/// Set the first directory entry's codec id.
fn set_first_codec(bytes: &mut [u8], codec: u8) {
    bytes[HEADER_SIZE + 12] = codec;
    refresh_directory_crc(bytes);
}

/// Set the header's minor version and repair the header checksum.
fn set_minor(bytes: &mut [u8], minor: u16) {
    bytes[10..12].copy_from_slice(&minor.to_le_bytes());
    let sum = vpng_container::checksum::crc32c_parts(&[&bytes[0..52], &bytes[56..64]]);
    bytes[52..56].copy_from_slice(&sum.to_le_bytes());
}

#[test]
fn the_default_path_is_still_a_v0_1_file() {
    // The promotion must not move a single published byte.
    let image = picture(300, 180);
    let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();
    let header = Header::parse(&report.bytes).unwrap();
    assert_eq!(header.minor, MINOR_V0_1);
    assert!(report
        .codec_ids
        .iter()
        .all(|&id| !vpng_codecs::is_v0_3_codec(id)));
}

#[test]
fn a_promoted_codec_makes_the_file_a_v0_3_file() {
    for codec in [
        vpng_codecs::CODEC_LZ77,
        vpng_codecs::CODEC_PREDICTOR_DICTIONARY,
        vpng_codecs::CODEC_CONTEXT_ENTROPY,
    ] {
        let image = picture(300, 180);
        let options = EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(codec);
        let report = encode(&image, &options).unwrap();
        let header = Header::parse(&report.bytes).unwrap();
        assert_eq!(
            header.minor, MINOR_V0_3,
            "codec {codec} must take the file to v0.3"
        );
        assert!(report.codec_ids.iter().all(|&id| id == codec));
    }
}

#[test]
fn a_v0_3_file_that_names_a_promoted_codec_decodes_exactly() {
    for codec in [
        vpng_codecs::CODEC_LZ77,
        vpng_codecs::CODEC_PREDICTOR_DICTIONARY,
        vpng_codecs::CODEC_CONTEXT_ENTROPY,
    ] {
        let image = picture(300, 180);
        let options = EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(codec);
        let report = encode(&image, &options).unwrap();
        let limits = Limits::default();
        let document = Document::parse(&report.bytes, &limits).unwrap();
        assert_eq!(document.decode_all(&limits).unwrap(), image);
    }
}

#[test]
fn a_v0_1_file_may_not_name_a_promoted_codec() {
    let image = picture(128, 96);
    let bytes = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(vpng_codecs::CODEC_RAW),
    )
    .unwrap()
    .bytes;

    for codec in [
        vpng_codecs::CODEC_LZ77,
        vpng_codecs::CODEC_PREDICTOR_DICTIONARY,
        vpng_codecs::CODEC_CONTEXT_ENTROPY,
    ] {
        let mut mutated = bytes.clone();
        set_first_codec(&mut mutated, codec);
        let limits = Limits::default();
        let err = Document::parse(&mutated, &limits).unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::UnsupportedFeature,
            "codec {codec} in a v0.1 file must be refused"
        );
    }
}

#[test]
fn a_v0_2_file_may_not_name_a_promoted_codec() {
    // A pyramid file is a v0.2 file. The rule binds the codec id to the
    // version, so a v0.2 file is refused just as a v0.1 file is.
    let image = picture(300, 180);
    let bytes = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_pyramid(2)
            .with_codec(vpng_codecs::CODEC_RAW),
    )
    .unwrap()
    .bytes;

    let header = Header::parse(&bytes).unwrap();
    assert_eq!(header.minor, MINOR_V0_2);

    let mut mutated = bytes.clone();
    set_first_codec(&mut mutated, vpng_codecs::CODEC_LZ77);
    let limits = Limits::default();
    let err = Document::parse(&mutated, &limits).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
}

#[test]
fn the_same_bytes_are_a_v0_3_file_when_the_version_says_so() {
    // The very same directory entry is fine in a v0.3 file and malformed in a
    // v0.1 file: the version is what binds the codec id.
    let image = picture(128, 96);
    let bytes = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(vpng_codecs::CODEC_LZ77),
    )
    .unwrap()
    .bytes;

    let mut downgraded = bytes.clone();
    set_minor(&mut downgraded, MINOR_V0_1);
    let limits = Limits::default();
    let err = Document::parse(&downgraded, &limits).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);

    let document = Document::parse(&bytes, &limits).unwrap();
    assert_eq!(document.decode_all(&limits).unwrap(), image);
}

#[test]
fn a_research_id_may_never_reach_a_file() {
    let image = picture(128, 96);
    let mut bytes = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(vpng_codecs::CODEC_RAW),
    )
    .unwrap()
    .bytes;

    for research in [0x80u8, 0x81, 0x83, 0x90, 0x91, 0x92, 0x93, 0xFF] {
        set_first_codec(&mut bytes, research);
        // Even with the newest version, a research id is not a codec.
        set_minor(&mut bytes, MINOR_V0_3);
        let limits = Limits::default();
        let err = Document::parse(&bytes, &limits).unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::UnsupportedFeature,
            "research id {research:#04x} must never be accepted"
        );
    }
}

#[test]
fn the_version_states_what_the_file_contains() {
    // The extended set is permitted, but this picture is coded better by the
    // original four, so the file stays a v0.1 file. The permission is not a
    // claim about the bytes.
    let image = picture(64, 64);
    let report = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_extended_codecs(),
    )
    .unwrap();
    let header = Header::parse(&report.bytes).unwrap();
    if report
        .codec_ids
        .iter()
        .any(|&id| vpng_codecs::is_v0_3_codec(id))
    {
        assert_eq!(header.minor, MINOR_V0_3);
    } else {
        assert_eq!(header.minor, MINOR_V0_1);
    }
}

#[test]
fn a_v0_3_file_with_a_pyramid_is_a_valid_v0_3_file() {
    // v0.2's features and v0.3's codecs coexist.
    let image = picture(300, 180);
    let report = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_extended_codecs()
            .with_pyramid(2),
    )
    .unwrap();
    let header = Header::parse(&report.bytes).unwrap();
    assert_eq!(header.minor, MINOR_V0_3);
    assert!(header.has_pyramid());

    let limits = Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert_eq!(document.level_count(), 3);
    assert_eq!(document.decode_all(&limits).unwrap(), image);
}
