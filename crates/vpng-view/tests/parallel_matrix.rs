//! The parallel-decode coverage matrix.
//!
//! The P5 report's Q11 says the parallel engine's tiles are byte-identical
//! to the sequential decoder's for one file. This test widens that promise
//! to **every pixel format, every representative size and three tile
//! geometries**: the parallel engine must never produce a different pixel,
//! whatever the file looks like.
//!
//! The rule it checks is the one the whole engine rests on: the pool
//! changes *when* a tile is decoded, never *what* it decodes.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use vpng_core::Limits;
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};
use vpng_testkit::{SIZES, V0_1_FORMATS};
use vpng_view::{FileDecoderFactory, Pool, RequestClass, RequestSpec};

/// A temporary directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "vpng-view-matrix-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        TempDir(base)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Decode every tile of a file with four workers and with the sequential
/// decoder, and assert that the two agree on every byte.
fn assert_parallel_matches_sequential(path: &Path, label: &str) {
    let limits = Limits::default();
    let bytes = fs::read(path).unwrap();
    let document = Document::parse(&bytes, &limits).unwrap();
    let tile_count = document.tile_count() as usize;

    let factory = FileDecoderFactory::open(path, &limits).unwrap();
    let pool = Pool::new(&factory, 4, 64 * 1024 * 1024, 4096).unwrap();

    let generation = pool.generation();
    for tile in 0..tile_count {
        let outcome = pool.request(RequestSpec {
            tile,
            class: RequestClass::Visible,
            distance: 0,
            generation,
        });
        assert!(
            !matches!(
                outcome,
                vpng_view::PoolOutcome::RejectedFull | vpng_view::PoolOutcome::RejectedStale
            ),
            "{label}: tile {tile} was refused"
        );
    }
    pool.wait_for_idle(Duration::from_secs(120)).unwrap();

    let mut compared = 0usize;
    for tile in 0..tile_count {
        let expected = document.decode_tile(tile, &limits).unwrap();
        let actual = pool
            .tile(tile)
            .unwrap_or_else(|| panic!("{label}: tile {tile} is missing from the cache"));
        assert_eq!(
            actual.len(),
            expected.len(),
            "{label}: tile {tile} has the wrong length"
        );
        assert_eq!(actual, expected, "{label}: tile {tile} differs");
        compared += 1;
    }

    assert_eq!(compared, tile_count);
    assert_eq!(pool.stats().completed as usize, tile_count);
}

#[test]
fn the_parallel_engine_matches_the_sequential_decoder_everywhere() {
    let directory = TempDir::new();
    let limits = Limits::default();
    let mut cases = 0usize;

    for format in V0_1_FORMATS {
        for (width, height) in SIZES {
            for tile_size in [16u32, 64, 256] {
                let image = vpng_testkit::gradient(width, height, format, 19);
                let bytes = encode(
                    &image,
                    &EncodeOptions::default().with_tile_size(tile_size, tile_size),
                )
                .unwrap()
                .bytes;

                let label = format!("{format}-{width}x{height}-tile{tile_size}");
                let path = directory.0.join(format!("case-{cases}.vpng"));
                fs::write(&path, &bytes).unwrap();

                assert_parallel_matches_sequential(&path, &label);
                let _ = limits;
                cases += 1;
            }
        }
    }

    // 8 formats x 6 sizes x 3 tile geometries.
    assert_eq!(cases, 144);
}
