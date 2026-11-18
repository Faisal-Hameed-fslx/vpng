//! The worker pool's tests.
//!
//! The pool's promise is simple to state and easy to break: the parallel
//! engine must produce exactly the pixels the sequential decoder produces,
//! decode each tile once, stop when the user pans away, and survive a
//! failed tile without losing the rest.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vpng_core::{CanonicalImage, ErrorKind, Limits, PixelFormat};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};

use crate::{
    DecoderFactory, FileDecoderFactory, Generation, Pool, PoolOutcome, RequestClass, RequestSpec,
    TileDecoder,
};

/// A temporary directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "vpng-view-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        TempDir(base)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A deterministic image.
fn image(width: u32, height: u32) -> CanonicalImage {
    let bpp = PixelFormat::rgba8().bytes_per_pixel() as usize;
    let mut data = vec![0u8; width as usize * height as usize * bpp];
    for y in 0..height {
        for x in 0..width {
            let base = ((y * width + x) as usize) * bpp;
            for c in 0..bpp {
                data[base + c] = ((x * 11 + y * 5 + c as u32 * 37) & 0xff) as u8;
            }
        }
    }
    CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), data).unwrap()
}

/// Write a file to a temporary path and return the path and the bytes.
fn write_file(directory: &TempDir, name: &str, image: &CanonicalImage, tile: u32) -> PathBuf {
    let bytes = encode(image, &EncodeOptions::default().with_tile_size(tile, tile))
        .unwrap()
        .bytes;
    let path = directory.path(name);
    fs::write(&path, &bytes).unwrap();
    path
}

/// A request for a tile in the pool's current generation.
fn request(pool: &Pool, tile: usize, class: RequestClass) -> RequestSpec {
    RequestSpec {
        tile,
        class,
        distance: 0,
        generation: pool.generation(),
    }
}

/// A decoder that can be told to fail, to stall, or to report that the
/// viewport moved while it was working.
struct StubDecoder {
    delay: Duration,
    fail_on: Option<usize>,
    pan_on: Option<usize>,
    generation: Option<Generation>,
    decoded: Arc<Mutex<Vec<usize>>>,
    counter: Arc<AtomicU64>,
}

impl StubDecoder {
    fn new(
        delay: Duration,
        fail_on: Option<usize>,
        pan_on: Option<usize>,
        generation: Option<Generation>,
        decoded: Arc<Mutex<Vec<usize>>>,
        counter: Arc<AtomicU64>,
    ) -> Self {
        StubDecoder {
            delay,
            fail_on,
            pan_on,
            generation,
            decoded,
            counter,
        }
    }
}

impl TileDecoder for StubDecoder {
    fn decode(&mut self, tile: usize) -> vpng_core::Result<Vec<u8>> {
        self.decoded.lock().unwrap().push(tile);
        self.counter.fetch_add(1, Ordering::SeqCst);

        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }

        if self.pan_on == Some(tile) {
            if let Some(generation) = &self.generation {
                // The user panned while this tile was being decoded.
                generation.advance();
            }
        }

        if self.fail_on == Some(tile) {
            return Err(vpng_core::Error::corrupt(
                "the stub decoder was told to fail",
            ));
        }

        Ok(vec![tile as u8; 16])
    }
}

/// Makes stub decoders, and records what they did.
#[derive(Clone)]
struct StubFactory {
    delay: Duration,
    fail_on: Option<usize>,
    pan_on: Option<usize>,
    decoded: Arc<Mutex<Vec<usize>>>,
    counter: Arc<AtomicU64>,
}

impl StubFactory {
    fn new() -> Self {
        StubFactory {
            delay: Duration::ZERO,
            fail_on: None,
            pan_on: None,
            decoded: Arc::new(Mutex::new(Vec::new())),
            counter: Arc::new(AtomicU64::new(0)),
        }
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    fn failing_on(mut self, tile: usize) -> Self {
        self.fail_on = Some(tile);
        self
    }

    fn panning_on(mut self, tile: usize) -> Self {
        self.pan_on = Some(tile);
        self
    }

    fn decode_count(&self) -> u64 {
        self.counter.load(Ordering::SeqCst)
    }

    fn decoded_tiles(&self) -> Vec<usize> {
        self.decoded.lock().unwrap().clone()
    }
}

impl DecoderFactory for StubFactory {
    type Decoder = StubDecoder;

    fn make_decoder(&self, generation: Generation) -> vpng_core::Result<Self::Decoder> {
        Ok(StubDecoder::new(
            self.delay,
            self.fail_on,
            self.pan_on,
            Some(generation),
            Arc::clone(&self.decoded),
            Arc::clone(&self.counter),
        ))
    }
}

#[test]
fn the_pool_produces_the_same_pixels_as_the_sequential_decoder() {
    let directory = TempDir::new("same-pixels");
    let limits = Limits::default();
    let source = image(512, 384);
    let path = write_file(&directory, "image.vpng", &source, 64);

    // The sequential decoder, over the same bytes.
    let bytes = fs::read(&path).unwrap();
    let document = Document::parse(&bytes, &limits).unwrap();
    let sequential: Vec<Vec<u8>> = (0..document.tile_count() as usize)
        .map(|tile| document.decode_tile(tile, &limits).unwrap())
        .collect();
    assert_eq!(sequential.len(), 48);

    // The parallel pool, over the same file.
    let factory = FileDecoderFactory::open(&path, &limits).unwrap();
    let pool = Pool::new(&factory, 4, 16 * 1024 * 1024, 128).unwrap();

    for tile in 0..sequential.len() {
        assert_eq!(
            pool.request(request(&pool, tile, RequestClass::Visible)),
            PoolOutcome::Queued
        );
    }
    pool.wait_for_idle(Duration::from_secs(60)).unwrap();

    for (tile, expected) in sequential.iter().enumerate() {
        let actual = pool.tile(tile).expect("the tile should be in the cache");
        assert_eq!(
            &actual, expected,
            "tile {tile} differs from the sequential decoder"
        );
    }

    // Every tile was decoded exactly once.
    assert_eq!(pool.stats().completed as usize, sequential.len());
    assert_eq!(pool.cache_stats().decoded_tiles as usize, sequential.len());
}

#[test]
fn a_cached_tile_is_not_decoded_again() {
    let factory = StubFactory::new();
    let pool = Pool::new(&factory, 2, 1024 * 1024, 16).unwrap();

    assert_eq!(
        pool.request(request(&pool, 7, RequestClass::Visible)),
        PoolOutcome::Queued
    );
    pool.wait_for_idle(Duration::from_secs(30)).unwrap();

    // The tile is resident: the second request is served from the cache.
    assert_eq!(
        pool.request(request(&pool, 7, RequestClass::Visible)),
        PoolOutcome::Cached
    );
    pool.wait_for_idle(Duration::from_secs(30)).unwrap();

    assert_eq!(factory.decode_count(), 1);
    assert_eq!(pool.stats().completed, 1);
}

#[test]
fn a_tile_in_flight_is_not_decoded_twice() {
    // The decoder is slow enough that the second request lands while the
    // first decode is still running.
    let factory = StubFactory::new().with_delay(Duration::from_millis(150));
    let pool = Pool::new(&factory, 1, 1024 * 1024, 16).unwrap();

    assert_eq!(
        pool.request(request(&pool, 3, RequestClass::Visible)),
        PoolOutcome::Queued
    );
    // Give the worker time to pick the request up.
    std::thread::sleep(Duration::from_millis(40));
    assert!(pool.is_in_flight(3));

    assert_eq!(
        pool.request(request(&pool, 3, RequestClass::Visible)),
        PoolOutcome::AlreadyInFlight
    );

    pool.wait_for_idle(Duration::from_secs(30)).unwrap();
    assert_eq!(factory.decode_count(), 1);
    assert_eq!(factory.decoded_tiles(), vec![3]);
}

#[test]
fn a_pan_discards_the_result_that_lands_late() {
    // The stub advances the viewport generation while it decodes tile 5.
    let panning = StubFactory::new().panning_on(5);
    let pool = Pool::new(&panning, 1, 1024 * 1024, 16).unwrap();

    assert_eq!(
        pool.request(request(&pool, 5, RequestClass::Visible)),
        PoolOutcome::Queued
    );
    pool.wait_for_idle(Duration::from_secs(30)).unwrap();

    // The pixels were produced, but the viewport moved on: they are
    // dropped, and the drop is counted separately from a failure.
    assert!(!pool.contains(5));
    assert_eq!(pool.stats().discarded, 1);
    assert_eq!(pool.stats().failed, 0);
    assert_eq!(pool.scheduler_counters().discarded_stale, 1);
}

#[test]
fn a_failed_decode_does_not_stop_the_pool() {
    let factory = StubFactory::new().failing_on(2);
    let pool = Pool::new(&factory, 2, 1024 * 1024, 16).unwrap();

    for tile in 0..4 {
        pool.request(request(&pool, tile, RequestClass::Visible));
    }
    pool.wait_for_idle(Duration::from_secs(30)).unwrap();

    // The failed tile is reported as a failure...
    let err = pool.wait_for_tile(2, Duration::from_secs(5)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::CorruptFile);

    // ...and the other tiles were decoded anyway.
    assert_eq!(pool.stats().failed, 1);
    assert_eq!(pool.stats().completed, 3);
    for tile in [0usize, 1, 3] {
        assert!(pool.contains(tile), "tile {tile} should be in the cache");
    }
}

#[test]
fn a_pan_cancels_the_queued_requests() {
    let factory = StubFactory::new().with_delay(Duration::from_millis(50));
    let pool = Pool::new(&factory, 1, 1024 * 1024, 64).unwrap();

    for tile in 0..20 {
        pool.request(request(&pool, tile, RequestClass::Visible));
    }

    // The user pans: the queued requests are dropped.
    pool.advance_generation();
    pool.wait_for_idle(Duration::from_secs(30)).unwrap();

    // The worker may have taken one request before the pan; the rest were
    // cancelled. Either way, no result from the old generation is kept.
    let kept = (0..20).filter(|tile| pool.contains(*tile)).count();
    assert!(
        kept <= 1,
        "at most one tile may survive the pan, got {kept}"
    );
    assert!(pool.scheduler_counters().cancelled >= 1);
}

#[test]
fn the_pool_reports_a_tile_that_is_never_decoded() {
    let factory = StubFactory::new();
    let pool = Pool::new(&factory, 1, 1024 * 1024, 16).unwrap();

    // Nothing was requested, so nothing can be waited for: the pool says
    // so instead of waiting forever.
    let err = pool
        .wait_for_tile(9, Duration::from_millis(200))
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Internal);
}

#[test]
fn the_pool_keeps_the_cache_within_its_budget() {
    // Every tile is 16 bytes; a 64-byte budget holds four.
    let factory = StubFactory::new();
    let pool = Pool::new(&factory, 2, 64, 16).unwrap();

    for tile in 0..10 {
        pool.request(request(&pool, tile, RequestClass::Visible));
    }
    pool.wait_for_idle(Duration::from_secs(30)).unwrap();

    let stats = pool.cache_stats();
    assert!(
        stats.current_bytes <= 64,
        "the cache grew to {} bytes",
        stats.current_bytes
    );
    assert!(stats.peak_bytes <= 64);
    assert!(stats.evictions > 0, "the cache should have evicted tiles");
}
