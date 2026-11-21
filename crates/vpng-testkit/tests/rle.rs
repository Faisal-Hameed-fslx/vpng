//! Run-length codec conformance.
//!
//! The run-length token format is pinned by byte-level golden vectors in
//! `vpng-codecs/src/rle.rs`. These tests exercise the codec through a whole
//! `.vpng` file: exact round-trips over several run structures, automatic
//! selection, and the access paths (full, region, and single-tile decode).

use vpng_codecs::{CODEC_RAW, CODEC_RLE};
use vpng_core::{CanonicalImage, Limits, PixelFormat, Rect};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};
use vpng_testkit::{noise, V0_1_FORMATS};

/// Build an image whose per-row symbol structure is chosen by `pattern`.
///
/// `pattern` selects the shape of runs: 0 = one flat color, 1 = a left flat half
/// and a right alternating half, 2 = runs of `run` pixels, 3 = pseudo-random.
fn pattern_image(
    width: u32,
    height: u32,
    format: PixelFormat,
    pattern: u32,
    run: u32,
) -> CanonicalImage {
    let bpp = format.bytes_per_pixel() as usize;
    let mut data = vec![0u8; width as usize * height as usize * bpp];
    for (i, pixel) in data.chunks_exact_mut(bpp).enumerate() {
        let x = (i % width as usize) as u32;
        let value = match pattern {
            0 => 0x33,
            1 => {
                if x < width / 2 {
                    0x30
                } else {
                    ((x % 2) as u8).wrapping_mul(0xFF)
                }
            }
            2 => ((x / run) as u8).wrapping_mul(37),
            _ => (x.wrapping_mul(2654435761) >> 13) as u8,
        };
        for (channel, byte) in pixel.iter_mut().enumerate() {
            *byte = value.wrapping_add(channel as u8);
        }
    }
    CanonicalImage::from_vec(width, height, format, data).unwrap()
}

#[test]
fn forced_rle_round_trips_every_format_and_pattern() {
    let limits = Limits::default();
    for &format in V0_1_FORMATS.iter() {
        for &pattern in &[0u32, 1, 2, 3] {
            for &(width, height) in &[(64u32, 64u32), (70, 33), (1, 1), (129, 3)] {
                let image = pattern_image(width, height, format, pattern, 7);
                let report = encode(
                    &image,
                    &EncodeOptions::default()
                        .with_tile_size(32, 32)
                        .with_codec(CODEC_RLE),
                )
                .unwrap();
                assert!(
                    report.codec_ids.iter().all(|&id| id == CODEC_RLE),
                    "a forced codec must be recorded verbatim"
                );

                let document = Document::parse(&report.bytes, &limits).unwrap();
                let decoded = document.decode_all(&limits).unwrap();
                assert_eq!(
                    decoded.data(),
                    image.data(),
                    "{format} pattern {pattern} at {width}x{height}"
                );
            }
        }
    }
}

#[test]
fn auto_selection_picks_rle_for_a_flat_image() {
    let limits = Limits::default();
    let image = pattern_image(128, 128, PixelFormat::rgba8(), 0, 1);
    let options = EncodeOptions::default().with_tile_size(64, 64);

    let auto = encode(&image, &options).unwrap();
    assert!(
        auto.codec_ids.iter().all(|&id| id == CODEC_RLE),
        "a flat image is a run: {:?}",
        auto.codec_ids
    );

    let raw = encode(&image, &options.with_codec(CODEC_RAW)).unwrap();
    assert!(
        auto.file_size * 50 < raw.file_size,
        "rle {} should be far below raw {}",
        auto.file_size,
        raw.file_size
    );

    let document = Document::parse(&auto.bytes, &limits).unwrap();
    assert_eq!(document.decode_all(&limits).unwrap(), image);
}

#[test]
fn incompressible_noise_falls_back_to_raw() {
    let limits = Limits::default();
    let image = noise(128, 128, PixelFormat::rgba8(), 0x5EED);
    let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();

    assert!(
        report.codec_ids.iter().all(|&id| id == CODEC_RAW),
        "noise must fall back to raw: {:?}",
        report.codec_ids
    );

    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert_eq!(document.decode_all(&limits).unwrap(), image);
}

#[test]
fn region_and_tile_decode_agree_for_rle_tiles() {
    let limits = Limits::default();
    let image = pattern_image(150, 100, PixelFormat::rgba8(), 2, 5);
    let report = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(CODEC_RLE),
    )
    .unwrap();
    assert!(report.codec_ids.iter().all(|&id| id == CODEC_RLE));

    let document = Document::parse(&report.bytes, &limits).unwrap();
    let full = document.decode_all(&limits).unwrap();

    let bpp = 4usize;
    let stride = 150 * bpp;
    let rect = Rect::new(60, 40, 70, 50).unwrap();
    let region = document.decode_region(rect, &limits).unwrap();
    let region_stride = region.row_bytes().unwrap() as usize;
    for y in 0..rect.height as usize {
        let src = (rect.y as usize + y) * stride + rect.x as usize * bpp;
        let dst = y * region_stride;
        assert_eq!(
            &region.data()[dst..dst + rect.width as usize * bpp],
            &full.data()[src..src + rect.width as usize * bpp]
        );
    }

    for (index, geom) in document.partition().tiles().iter().enumerate() {
        let tile = document.decode_tile(index, &limits).unwrap();
        for row in 0..geom.height as usize {
            let src = (geom.y as usize + row) * stride + geom.x as usize * bpp;
            let dst = row * geom.width as usize * bpp;
            assert_eq!(
                &tile[dst..dst + geom.width as usize * bpp],
                &full.data()[src..src + geom.width as usize * bpp]
            );
        }
    }
}

#[test]
fn rle_encoding_is_deterministic() {
    let image = pattern_image(200, 150, PixelFormat::rgba8(), 1, 4);
    let options = EncodeOptions::default().with_tile_size(64, 64);
    let first = encode(&image, &options).unwrap().bytes;
    let second = encode(&image, &options).unwrap().bytes;
    assert_eq!(first, second, "encoder output must be byte-identical");
}

#[test]
fn golden_rle_mixed_64x64_gray8_tile64() {
    use std::path::PathBuf;

    // Left half flat (repeat tokens) and right half alternating (literal
    // tokens), so one fixture pins both token kinds through the container.
    let image = pattern_image(64, 64, PixelFormat::gray8(), 1, 1);
    let report = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(CODEC_RLE),
    )
    .unwrap();
    assert!(report.codec_ids.iter().all(|&id| id == CODEC_RLE));
    // Per row: repeat(32) = 2 bytes, literal(32) = 33 bytes => 35 bytes; 64 rows.
    // Container: 64 header + 44 directory + 2240 payload.
    assert_eq!(report.file_size, 64 + 44 + 64 * 35);

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rle_mixed_64x64_gray8_tile64.vpng");
    if path.exists() {
        let stored = std::fs::read(&path).unwrap();
        assert_eq!(
            stored, report.bytes,
            "golden file changed; the on-disk format drifted"
        );
    } else {
        std::fs::write(&path, &report.bytes).unwrap();
    }

    let limits = Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert_eq!(document.decode_all(&limits).unwrap(), image);
}
