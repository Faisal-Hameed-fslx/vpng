//! Exact round-trip conformance across every v0.1 pixel format, a range of
//! sizes, and several tile sizes.

use vpng_core::{CanonicalImage, Limits, PixelFormat, Rect};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};
use vpng_testkit::{checker, gradient, noise, SIZES, V0_1_FORMATS};

const TILE_SIZES: [u32; 4] = [16, 64, 128, 256];

fn roundtrip_exact(image: &CanonicalImage, tile: u32) {
    let limits = Limits::default();
    let options = EncodeOptions::default().with_tile_size(tile, tile);
    let report = encode(image, &options).expect("encode failed");
    let document = Document::parse(&report.bytes, &limits).expect("parse failed");
    let decoded = document.decode_all(&limits).expect("decode failed");

    assert_eq!(decoded.width(), image.width());
    assert_eq!(decoded.height(), image.height());
    assert_eq!(decoded.format(), image.format());
    assert_eq!(
        decoded.data(),
        image.data(),
        "pixels differ after round-trip"
    );
}

#[test]
fn gradient_roundtrips_exactly_everywhere() {
    for &format in V0_1_FORMATS.iter() {
        for &(width, height) in SIZES.iter() {
            let image = gradient(width, height, format, 1);
            for &tile in TILE_SIZES.iter() {
                roundtrip_exact(&image, tile);
            }
        }
    }
}

#[test]
fn noise_roundtrips_exactly_everywhere() {
    for &format in V0_1_FORMATS.iter() {
        for &(width, height) in SIZES.iter() {
            let image = noise(width, height, format, 0xC0FFEE);
            for &tile in TILE_SIZES.iter() {
                roundtrip_exact(&image, tile);
            }
        }
    }
}

#[test]
fn checker_roundtrips_exactly_everywhere() {
    for &format in V0_1_FORMATS.iter() {
        for &(width, height) in SIZES.iter() {
            let image = checker(width, height, format, 3);
            for &tile in TILE_SIZES.iter() {
                roundtrip_exact(&image, tile);
            }
        }
    }
}

#[test]
fn transparent_pixel_channels_are_preserved() {
    // Construct an RGBA image whose transparent pixels carry non-zero RGB.
    let mut data = Vec::new();
    for i in 0..16u8 {
        data.extend_from_slice(&[i, i.wrapping_add(1), i.wrapping_add(2), 0x00]);
    }
    let image = CanonicalImage::from_vec(4, 4, PixelFormat::rgba8(), data).unwrap();

    let limits = Limits::default();
    let report = encode(&image, &EncodeOptions::default().with_tile_size(2, 2)).unwrap();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    let decoded = document.decode_all(&limits).unwrap();

    assert_eq!(decoded.data(), image.data());
    // Spot-check that the RGB under zero alpha really survived.
    assert_eq!(&decoded.data()[0..4], &[0, 1, 2, 0]);
}

#[test]
fn region_decode_equals_full_decode_crop() {
    let limits = Limits::default();
    for &format in V0_1_FORMATS.iter() {
        let image = gradient(300, 180, format, 3);
        let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();
        let document = Document::parse(&report.bytes, &limits).unwrap();
        let full = document.decode_all(&limits).unwrap();

        let bpp = format.bytes_per_pixel() as usize;
        let stride = format.bytes_per_row(300).unwrap() as usize;

        let rects = [
            Rect::new(0, 0, 1, 1).unwrap(),
            Rect::new(63, 63, 3, 3).unwrap(),
            Rect::new(120, 40, 100, 90).unwrap(),
            Rect::new(299, 179, 1, 1).unwrap(),
            Rect::new(0, 0, 300, 180).unwrap(),
        ];

        for &rect in rects.iter() {
            let region = document.decode_region(rect, &limits).unwrap();
            assert_eq!(region.width(), rect.width);
            assert_eq!(region.height(), rect.height);
            let region_stride = region.row_bytes().unwrap() as usize;

            for y in 0..rect.height as usize {
                let src = (rect.y as usize + y) * stride + rect.x as usize * bpp;
                let dst = y * region_stride;
                assert_eq!(
                    &region.data()[dst..dst + rect.width as usize * bpp],
                    &full.data()[src..src + rect.width as usize * bpp],
                    "region mismatch at row {y} for {rect:?} {format}"
                );
            }
        }
    }
}

#[test]
fn single_tile_decode_matches_full_image() {
    let limits = Limits::default();
    let image = gradient(300, 180, PixelFormat::rgba8(), 9);
    let report = encode(&image, &EncodeOptions::default().with_tile_size(128, 128)).unwrap();
    let document = Document::parse(&report.bytes, &limits).unwrap();

    let bpp = 4usize;
    let stride = 300 * bpp;
    for (index, geom) in document.partition().tiles().iter().enumerate() {
        let tile = document.decode_tile(index, &limits).unwrap();
        assert_eq!(tile.len(), geom.width as usize * geom.height as usize * bpp);
        for row in 0..geom.height as usize {
            let src = (geom.y as usize + row) * stride + geom.x as usize * bpp;
            let dst = row * geom.width as usize * bpp;
            assert_eq!(
                &tile[dst..dst + geom.width as usize * bpp],
                &image.data()[src..src + geom.width as usize * bpp]
            );
        }
    }
}

#[test]
fn encoding_is_deterministic() {
    let image = noise(257, 130, PixelFormat::rgba8(), 5);
    let options = EncodeOptions::default().with_tile_size(64, 64);
    let first = encode(&image, &options).unwrap().bytes;
    let second = encode(&image, &options).unwrap().bytes;
    assert_eq!(first, second, "encoder output must be byte-identical");
}
