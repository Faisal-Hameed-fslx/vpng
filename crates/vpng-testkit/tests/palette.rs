//! Palette codec conformance.
//!
//! The palette payload layout is pinned by byte-level golden vectors in
//! `vpng-codecs/src/palette.rs`. These tests exercise the codec through a whole
//! `.vpng` file: exact round-trips, automatic selection, and the access paths
//! (full, region, and single-tile decode).

use vpng_codecs::{CODEC_PALETTE, CODEC_RAW};
use vpng_core::{CanonicalImage, Limits, PixelFormat, Rect};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};
use vpng_testkit::{checker, noise, V0_1_FORMATS};

/// Paint an image that uses exactly `colors` distinct pixel values, repeating in
/// scan order. Every tile therefore also stays within that color count.
fn palette_image(width: u32, height: u32, format: PixelFormat, colors: usize) -> CanonicalImage {
    let bpp = format.bytes_per_pixel() as usize;
    let mut data = vec![0u8; width as usize * height as usize * bpp];
    for (index, pixel) in data.chunks_exact_mut(bpp).enumerate() {
        let c = (index % colors) as u8;
        for (channel, byte) in pixel.iter_mut().enumerate() {
            *byte = c.wrapping_mul(31).wrapping_add(channel as u8 * 7);
        }
    }
    CanonicalImage::from_vec(width, height, format, data).unwrap()
}

#[test]
fn forced_palette_round_trips_every_format_and_color_count() {
    let limits = Limits::default();
    // 1, 2, 3, 16 and 256 distinct colors cross every index width the format
    // permits (1, 2, 4 and 8 bits).
    for &format in V0_1_FORMATS.iter() {
        for &colors in &[1usize, 2, 3, 16, 256] {
            for &(width, height) in &[(64u32, 64u32), (70, 33), (1, 1)] {
                let image = palette_image(width, height, format, colors);
                let report = encode(
                    &image,
                    &EncodeOptions::default()
                        .with_tile_size(32, 32)
                        .with_codec(CODEC_PALETTE),
                )
                .unwrap();
                assert!(
                    report.codec_ids.iter().all(|&id| id == CODEC_PALETTE),
                    "a forced codec must be recorded verbatim"
                );

                let document = Document::parse(&report.bytes, &limits).unwrap();
                let decoded = document.decode_all(&limits).unwrap();
                assert_eq!(
                    decoded.data(),
                    image.data(),
                    "{format} with {colors} colors at {width}x{height}"
                );
            }
        }
    }
}

#[test]
fn auto_selection_prefers_palette_for_a_two_color_image() {
    let limits = Limits::default();
    let image = checker(128, 128, PixelFormat::rgba8(), 4);
    let options = EncodeOptions::default().with_tile_size(64, 64);

    let auto = encode(&image, &options).unwrap();
    assert!(
        auto.codec_ids.iter().all(|&id| id == CODEC_PALETTE),
        "a two-color image should be palettised: {:?}",
        auto.codec_ids
    );

    let raw = encode(&image, &options.with_codec(CODEC_RAW)).unwrap();
    assert!(
        auto.file_size * 4 < raw.file_size,
        "palette {} should be far below raw {}",
        auto.file_size,
        raw.file_size
    );

    let document = Document::parse(&auto.bytes, &limits).unwrap();
    assert_eq!(document.decode_all(&limits).unwrap(), image);
}

#[test]
fn incompressible_noise_falls_back_to_raw() {
    let limits = Limits::default();
    let image = noise(128, 128, PixelFormat::rgba8(), 0xABCD);
    let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();

    assert!(
        report.codec_ids.iter().all(|&id| id == CODEC_RAW),
        "noise must fall back to raw: {:?}",
        report.codec_ids
    );
    // Raw never expands.
    let expected: Vec<u64> = vec![64 * 64 * 4; report.tile_count as usize];
    assert_eq!(report.compressed_lengths, expected);

    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert_eq!(document.decode_all(&limits).unwrap(), image);
}

#[test]
fn region_and_tile_decode_agree_for_palette_tiles() {
    let limits = Limits::default();
    let image = checker(150, 100, PixelFormat::rgba8(), 7);
    let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();
    assert!(report.codec_ids.iter().all(|&id| id == CODEC_PALETTE));

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

    for (index, geom) in document.grid().iter().enumerate() {
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
fn palette_encoding_is_deterministic() {
    let image = palette_image(200, 150, PixelFormat::rgba8(), 5);
    let options = EncodeOptions::default().with_tile_size(64, 64);
    let first = encode(&image, &options).unwrap().bytes;
    let second = encode(&image, &options).unwrap().bytes;
    assert_eq!(first, second, "encoder output must be byte-identical");
}
