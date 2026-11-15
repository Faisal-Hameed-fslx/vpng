//! Bounded mutation sweep over every v0.1 codec.
//!
//! This is a deterministic, in-test corruption sweep, not coverage-guided
//! fuzzing: every truncation and every single-byte substitution of a small
//! Raw/Palette/RLE/Predictor file, plus a seeded multi-byte corruption run.
//! The contract under test is that parsing and decoding always return a typed
//! `Result` and never panic, abort, or hang. `cargo fuzz` remains the P10 tool
//! (see `docs/THREAT_MODEL.md`).

use vpng_codecs::{CODEC_PALETTE, CODEC_PREDICTOR, CODEC_RAW, CODEC_RLE};
use vpng_core::{CanonicalImage, Limits, PixelFormat};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};
use vpng_testkit::{checker, gradient};

/// Deterministic xorshift64* generator, mirroring the testkit PRNG.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

/// Left half flat, right half alternating: both RLE token kinds.
fn rle_image() -> CanonicalImage {
    let (width, height) = (64u32, 64u32);
    let mut data = vec![0u8; width as usize * height as usize];
    for y in 0..height as usize {
        for x in 0..width as usize {
            data[y * width as usize + x] = if x < width as usize / 2 {
                0x30
            } else if x % 2 == 0 {
                0x00
            } else {
                0xff
            };
        }
    }
    CanonicalImage::from_vec(width, height, PixelFormat::gray8(), data).unwrap()
}

fn encoded(image: &CanonicalImage, codec: u8) -> Vec<u8> {
    encode(
        image,
        &EncodeOptions::default()
            .with_tile_size(32, 32)
            .with_codec(codec),
    )
    .unwrap()
    .bytes
}

fn fixtures() -> Vec<(&'static str, u8, Vec<u8>)> {
    let raw = gradient(64, 64, PixelFormat::rgba8(), 3);
    let palette = checker(64, 64, PixelFormat::rgba8(), 8);
    let rle = rle_image();
    // A smooth ramp, so the predictor codec actually applies and the file
    // carries a real predictor plan and arithmetic-coded residuals.
    let predictor = gradient(64, 64, PixelFormat::rgba8(), 1);
    vec![
        ("raw", CODEC_RAW, encoded(&raw, CODEC_RAW)),
        ("palette", CODEC_PALETTE, encoded(&palette, CODEC_PALETTE)),
        ("rle", CODEC_RLE, encoded(&rle, CODEC_RLE)),
        (
            "predictor",
            CODEC_PREDICTOR,
            encoded(&predictor, CODEC_PREDICTOR),
        ),
    ]
}

/// Parse, and if parsing succeeds decode every tile; either outcome must be a
/// typed error or a success. A panic fails the test.
fn accept_any_outcome(bytes: &[u8]) {
    let limits = Limits::default();
    if let Ok(document) = Document::parse(bytes, &limits) {
        let _ = document.decode_all(&limits);
    }
}

#[test]
fn fixtures_are_valid_before_mutation() {
    let limits = Limits::default();
    for (name, codec, bytes) in fixtures() {
        let document = Document::parse(&bytes, &limits).unwrap_or_else(|err| {
            panic!("{name} fixture must parse: {err}");
        });
        assert!(
            document.decode_all(&limits).is_ok(),
            "{name} fixture must decode"
        );
        assert_eq!(
            document.tile_entry(0).unwrap().codec_id,
            codec,
            "{name} fixture must use its codec"
        );
    }
}

#[test]
fn every_truncation_is_handled() {
    for (name, _, bytes) in fixtures() {
        for len in 0..bytes.len() {
            let limits = Limits::default();
            let _ = Document::parse(&bytes[..len], &limits);
        }
        // The full file still parses.
        let limits = Limits::default();
        assert!(
            Document::parse(&bytes, &limits).is_ok(),
            "{name} file must survive its own sweep"
        );
    }
}

#[test]
fn every_single_byte_substitution_is_handled() {
    // Three substitutions per offset gives zero, all-ones, and a mixed pattern
    // without the cost of cloning the file for every mutation.
    const VALUES: [u8; 3] = [0x00, 0xff, 0x5a];
    for (name, _, bytes) in fixtures() {
        let mut mutated = bytes.clone();
        for offset in 0..bytes.len() {
            let original = bytes[offset];
            for &value in VALUES.iter() {
                if value == original {
                    continue;
                }
                mutated[offset] = value;
                accept_any_outcome(&mutated);
            }
            mutated[offset] = original;
        }
        assert_eq!(mutated, bytes, "{name} sweep must restore the fixture");
    }
}

#[test]
fn seeded_multi_byte_corruption_is_handled() {
    let mut rng = Rng(0xC0FF_EE00_1234_5678);
    for (name, _, bytes) in fixtures() {
        let mut mutated = bytes.clone();
        for _ in 0..2000 {
            let changes = 1 + rng.below(4);
            let mut touched = Vec::with_capacity(changes);
            for _ in 0..changes {
                let offset = rng.below(bytes.len());
                touched.push((offset, mutated[offset]));
                mutated[offset] = rng.next() as u8;
            }
            accept_any_outcome(&mutated);
            // Restore in reverse: an offset may have been hit more than once, and
            // the later record holds the intermediate value.
            for (offset, original) in touched.into_iter().rev() {
                mutated[offset] = original;
            }
        }
        assert_eq!(mutated, bytes, "{name} sweep must restore the fixture");
    }
}
