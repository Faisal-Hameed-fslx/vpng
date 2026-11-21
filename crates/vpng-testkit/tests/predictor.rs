//! Predictor + entropy codec conformance.
//!
//! The predictor payload layout and the arithmetic coder are pinned by
//! byte-level golden vectors in `vpng-codecs/src/predictor.rs`. These tests
//! exercise the codec through a whole `.vpng` file: exact round-trips over
//! every v0.1 format, edge tiles that are narrower or shorter than the grid,
//! automatic selection, the access paths (full, region, and single-tile
//! decode), and a whole-file golden fixture.

use vpng_codecs::{CODEC_PREDICTOR, CODEC_RAW, IMPLEMENTED_CODEC_IDS};
use vpng_core::{CanonicalImage, ErrorKind, Limits, PixelFormat, Rect};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};
use vpng_testkit::{noise, Rng, V0_1_FORMATS};

/// A smooth image whose horizontal gradient is constant, so the predictor's
/// residuals are small and nearly constant — the codec's ideal case.
fn ramp_image(width: u32, height: u32, format: PixelFormat) -> CanonicalImage {
    let bpp = format.bytes_per_pixel() as usize;
    let mut data = vec![0u8; width as usize * height as usize * bpp];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let base = (y * width as usize + x) * bpp;
            for c in 0..bpp {
                data[base + c] = ((x * 3 + y * 5 + c * 17) & 0xFF) as u8;
            }
        }
    }
    CanonicalImage::from_vec(width, height, format, data).unwrap()
}

/// An image whose every byte is `value`, used to probe the signed residual
/// boundaries (0, 127, 128 and 255 all fold differently).
fn constant_image(width: u32, height: u32, format: PixelFormat, value: u8) -> CanonicalImage {
    let bpp = format.bytes_per_pixel() as usize;
    let data = vec![value; width as usize * height as usize * bpp];
    CanonicalImage::from_vec(width, height, format, data).unwrap()
}

/// Encode with a forced codec and decode the whole file back.
fn round_trip(image: &CanonicalImage, tile: u32, codec: u8) -> CanonicalImage {
    let limits = Limits::default();
    let report = encode(
        image,
        &EncodeOptions::default()
            .with_tile_size(tile, tile)
            .with_codec(codec),
    )
    .unwrap();
    assert!(
        report.codec_ids.iter().all(|&id| id == codec),
        "a forced codec must be recorded verbatim: {:?}",
        report.codec_ids
    );
    let document = Document::parse(&report.bytes, &limits).unwrap();
    document.decode_all(&limits).unwrap()
}

#[test]
fn forced_predictor_round_trips_every_format_and_size() {
    // The sizes cover an exact fit, a single edge column, a single edge row,
    // and both at once, so edge tiles of every shape are exercised.
    let sizes = [
        (1u32, 1u32),
        (7, 3),
        (64, 64),
        (70, 33),
        (129, 130),
        (300, 180),
    ];
    for &format in V0_1_FORMATS.iter() {
        for &(width, height) in &sizes {
            let image = ramp_image(width, height, format);
            let decoded = round_trip(&image, 32, CODEC_PREDICTOR);
            assert_eq!(decoded.data(), image.data(), "{format} at {width}x{height}");
        }
    }
}

#[test]
fn edge_tiles_are_encoded_at_their_actual_width() {
    // A 70x33 image in a 32x32 grid has a 6-pixel-wide last column and a
    // 1-pixel-tall last row. The predictor must learn the real width of each
    // tile, not the nominal 32.
    let limits = Limits::default();
    let image = ramp_image(70, 33, PixelFormat::gray8());
    let report = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(32, 32)
            .with_codec(CODEC_PREDICTOR),
    )
    .unwrap();
    assert_eq!(report.tile_count, 6, "3 columns x 2 rows");

    let document = Document::parse(&report.bytes, &limits).unwrap();
    let widths: Vec<u32> = document
        .partition()
        .tiles()
        .iter()
        .map(|g| g.width)
        .collect();
    let heights: Vec<u32> = document
        .partition()
        .tiles()
        .iter()
        .map(|g| g.height)
        .collect();
    assert_eq!(widths, vec![32, 32, 6, 32, 32, 6]);
    assert_eq!(heights, vec![32, 32, 32, 1, 1, 1]);

    assert_eq!(document.decode_all(&limits).unwrap().data(), image.data());
}

#[test]
fn auto_selection_prefers_predictor_for_smooth_data() {
    let limits = Limits::default();
    let image = ramp_image(128, 128, PixelFormat::gray8());
    let options = EncodeOptions::default().with_tile_size(64, 64);

    let auto = encode(&image, &options).unwrap();
    assert!(
        auto.codec_ids.iter().all(|&id| id == CODEC_PREDICTOR),
        "a smooth ramp is the predictor's case: {:?}",
        auto.codec_ids
    );

    let raw = encode(&image, &options.with_codec(CODEC_RAW)).unwrap();
    assert!(
        auto.file_size * 2 < raw.file_size,
        "predictor {} should be well below raw {}",
        auto.file_size,
        raw.file_size
    );

    let document = Document::parse(&auto.bytes, &limits).unwrap();
    assert_eq!(document.decode_all(&limits).unwrap(), image);
}

#[test]
fn auto_selection_never_loses_to_the_best_forced_codec() {
    // The selector must pick the smallest payload per tile. Forcing each codec
    // in turn and taking the smallest file must never beat `auto`.
    for &(width, height) in &[(64u32, 64u32), (150, 100), (257, 130)] {
        let image = ramp_image(width, height, PixelFormat::rgba8());
        let options = EncodeOptions::default().with_tile_size(64, 64);

        let auto = encode(&image, &options).unwrap();
        let mut best_forced = u64::MAX;
        for &id in IMPLEMENTED_CODEC_IDS {
            match encode(&image, &options.with_codec(id)) {
                Ok(report) => best_forced = best_forced.min(report.file_size),
                Err(err) => assert_eq!(
                    err.kind(),
                    ErrorKind::UnsupportedFeature,
                    "codec {id} should only refuse tiles it cannot represent"
                ),
            }
        }
        assert!(
            auto.file_size <= best_forced,
            "auto {} must not lose to the best forced {} at {width}x{height}",
            auto.file_size,
            best_forced
        );
    }
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
    // Raw never expands.
    let expected: Vec<u64> = vec![64 * 64 * 4; report.tile_count as usize];
    assert_eq!(report.compressed_lengths, expected);

    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert_eq!(document.decode_all(&limits).unwrap(), image);
}

#[test]
fn region_and_tile_decode_agree_for_predictor_tiles() {
    let limits = Limits::default();
    let image = ramp_image(150, 100, PixelFormat::rgba8());
    let report = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(CODEC_PREDICTOR),
    )
    .unwrap();
    assert!(report.codec_ids.iter().all(|&id| id == CODEC_PREDICTOR));

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
fn predictor_encoding_is_deterministic() {
    let image = ramp_image(200, 150, PixelFormat::rgba8());
    let options = EncodeOptions::default()
        .with_tile_size(64, 64)
        .with_codec(CODEC_PREDICTOR);
    let first = encode(&image, &options).unwrap().bytes;
    let second = encode(&image, &options).unwrap().bytes;
    assert_eq!(first, second, "encoder output must be byte-identical");
}

/// Property: every byte pattern survives a forced predictor round-trip.
///
/// The sweep is deterministic (no clock, no RNG seeding from the environment),
/// so a failure is always reproducible.
#[test]
fn property_any_byte_pattern_round_trips_exactly() {
    for &format in V0_1_FORMATS.iter() {
        let bpp = format.bytes_per_pixel() as usize;
        for &(width, height) in &[(17u32, 13u32), (64, 64), (70, 33)] {
            // A handful of adversarial patterns, then a seeded random sweep.
            let patterns: Vec<Vec<u8>> = vec![
                vec![0x00],
                vec![0xFF],
                vec![0x80],
                vec![0x7F],
                vec![0x00, 0xFF],
                vec![0x80, 0x80],
                (0..=255u8).collect(),
            ];
            for pattern in &patterns {
                let data: Vec<u8> = (0..width as usize * height as usize * bpp)
                    .map(|i| pattern[i % pattern.len()])
                    .collect();
                let image = CanonicalImage::from_vec(width, height, format, data).unwrap();
                let decoded = round_trip(&image, 32, CODEC_PREDICTOR);
                assert_eq!(
                    decoded.data(),
                    image.data(),
                    "{format} at {width}x{height} with pattern {pattern:?}"
                );
            }

            for seed in 0..6u64 {
                let mut rng = Rng::new(seed + 1);
                let mut data = vec![0u8; width as usize * height as usize * bpp];
                rng.fill(&mut data);
                let image = CanonicalImage::from_vec(width, height, format, data).unwrap();
                let decoded = round_trip(&image, 32, CODEC_PREDICTOR);
                assert_eq!(
                    decoded.data(),
                    image.data(),
                    "{format} at {width}x{height} with seed {seed}"
                );
            }
        }
    }
}

/// Property: the residual folding boundaries survive exactly.
///
/// 127 and 128 fold to different signed values, and 255 wraps to -1; a codec
/// that mixed those up would corrupt every such image.
#[test]
fn property_signed_residual_boundaries_survive() {
    for value in [0x00u8, 0x01, 0x7F, 0x80, 0xFE, 0xFF] {
        let image = constant_image(37, 11, PixelFormat::gray8(), value);
        let decoded = round_trip(&image, 16, CODEC_PREDICTOR);
        assert_eq!(decoded.data(), image.data(), "constant {value:#04X}");
    }
}

#[test]
fn golden_predictor_ramp_64x64_gray8_tile64() {
    use std::path::PathBuf;

    // A single 64x64 tile in a 64x64 grid: the whole file is one predictor
    // payload, so the fixture pins the container and the codec together.
    let image = ramp_image(64, 64, PixelFormat::gray8());
    let report = encode(
        &image,
        &EncodeOptions::default()
            .with_tile_size(64, 64)
            .with_codec(CODEC_PREDICTOR),
    )
    .unwrap();
    assert!(report.codec_ids.iter().all(|&id| id == CODEC_PREDICTOR));
    assert_eq!(report.tile_count, 1);

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("predictor_ramp_64x64_gray8_tile64.vpng");
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
