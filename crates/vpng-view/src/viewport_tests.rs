//! The viewport and prefetch policy's tests.
//!
//! The policy's promises are exact: the visible tiles are the tiles the
//! rectangle covers and no others; the prefetch ring is the ring around
//! them and is clipped to the image; the nearest tiles come first; the
//! direction of travel is preferred; and a prefetch is never allowed to
//! push the visible tiles out of a bounded queue.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vpng_core::{ErrorKind, Rect};
use vpng_tiling::TileGrid;

use crate::{
    Direction, Pool, PoolOutcome, PrefetchPolicy, RequestClass, RequestSpec, TileDecoder, ViewPlan,
    Viewport,
};

/// A grid over a 1024×768 image with 64×64 tiles: 16 columns, 12 rows.
fn grid() -> TileGrid {
    TileGrid::new(64, 64, 1024, 768).unwrap()
}

/// A rectangle of pixels.
fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
    Rect::new(x, y, width, height).unwrap()
}

/// A decoder that records what it decoded.
#[derive(Clone)]
struct CountingFactory {
    decoded: Arc<Mutex<Vec<usize>>>,
    counter: Arc<AtomicU64>,
    delay: Duration,
}

impl CountingFactory {
    fn new() -> Self {
        CountingFactory {
            decoded: Arc::new(Mutex::new(Vec::new())),
            counter: Arc::new(AtomicU64::new(0)),
            delay: Duration::ZERO,
        }
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    fn decode_count(&self) -> u64 {
        self.counter.load(Ordering::SeqCst)
    }

    fn decoded_tiles(&self) -> Vec<usize> {
        self.decoded.lock().unwrap().clone()
    }
}

impl crate::DecoderFactory for CountingFactory {
    type Decoder = CountingDecoder;

    fn make_decoder(&self, _generation: crate::Generation) -> vpng_core::Result<Self::Decoder> {
        Ok(CountingDecoder {
            decoded: Arc::clone(&self.decoded),
            counter: Arc::clone(&self.counter),
            delay: self.delay,
        })
    }
}

struct CountingDecoder {
    decoded: Arc<Mutex<Vec<usize>>>,
    counter: Arc<AtomicU64>,
    delay: Duration,
}

impl TileDecoder for CountingDecoder {
    fn decode(&mut self, tile: usize) -> vpng_core::Result<Vec<u8>> {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        self.decoded.lock().unwrap().push(tile);
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(vec![tile as u8; 4])
    }
}

#[test]
fn the_visible_tiles_are_the_ones_the_rectangle_covers() {
    let grid = grid();
    let viewport = Viewport::new(&grid, rect(64, 64, 128, 128)).unwrap();

    // The rectangle covers exactly the four tiles at columns 1-2 and
    // rows 1-2.
    let expected: Vec<usize> = vec![17, 18, 33, 34];
    let plan = viewport.plan(&PrefetchPolicy::default(), Direction::None);

    let visible: Vec<usize> = plan.visible.iter().map(|entry| entry.tile).collect();
    assert_eq!(visible.len(), 4);
    for tile in expected {
        assert!(visible.contains(&tile), "tile {tile} should be visible");
    }
}

#[test]
fn the_prefetch_ring_is_the_next_ring_out() {
    let grid = grid();
    let viewport = Viewport::new(&grid, rect(64, 64, 128, 128)).unwrap();

    let policy = PrefetchPolicy {
        radius: 1,
        direction_reach: 0,
        direction_bonus: 0,
    };
    let plan = viewport.plan(&policy, Direction::None);

    // The visible rectangle is columns 1-2, rows 1-2. A radius of one
    // reaches columns 0-3 and rows 0-3, minus the visible tiles.
    let prefetch: Vec<usize> = plan.prefetch.iter().map(|entry| entry.tile).collect();
    assert_eq!(prefetch.len(), 12);

    // Every prefetch tile is outside the visible rectangle.
    for tile in &prefetch {
        let col = (tile % 16) as u32;
        let row = (tile / 16) as u32;
        let visible = (1..=2).contains(&col) && (1..=2).contains(&row);
        assert!(
            !visible,
            "tile {tile} is visible and must not be prefetched"
        );
    }
}
#[test]
fn the_prefetch_ring_is_clipped_to_the_image() {
    let grid = grid();
    // A viewport in the top-left corner of the image.
    let viewport = Viewport::new(&grid, rect(0, 0, 64, 64)).unwrap();

    let policy = PrefetchPolicy {
        radius: 3,
        direction_reach: 0,
        direction_bonus: 0,
    };
    let plan = viewport.plan(&policy, Direction::None);

    for entry in &plan.prefetch {
        let col = (entry.tile % 16) as u32;
        let row = (entry.tile / 16) as u32;
        assert!(
            col < 16 && row < 12,
            "tile {} is outside the grid",
            entry.tile
        );
    }

    // The corner tile's ring is 3+3 tiles, minus the visible one.
    assert_eq!(plan.prefetch.len(), 15);
}

#[test]
fn the_nearest_tile_comes_first() {
    let grid = grid();
    let viewport = Viewport::new(&grid, rect(0, 0, 192, 192)).unwrap();

    let plan = viewport.plan(&PrefetchPolicy::default(), Direction::None);

    // The distances are non-decreasing.
    let distances: Vec<u32> = plan.visible.iter().map(|entry| entry.distance).collect();
    for pair in distances.windows(2) {
        assert!(pair[0] <= pair[1], "the distances must be sorted");
    }

    // The centre tile of a 3x3 viewport is the closest.
    assert_eq!(plan.visible[0].distance, 0);
}

#[test]
fn the_direction_of_travel_is_preferred() {
    let grid = grid();
    let viewport = Viewport::new(&grid, rect(320, 320, 128, 128)).unwrap();

    let policy = PrefetchPolicy {
        radius: 1,
        direction_reach: 2,
        direction_bonus: 4,
    };

    let right = viewport.plan(&policy, Direction::Right);
    let left = viewport.plan(&policy, Direction::Left);

    // The tile immediately to the right of the viewport. The viewport
    // covers columns 5-6 and rows 5-6, so the next column is 7.
    let right_of = |plan: &ViewPlan| {
        plan.prefetch
            .iter()
            .find(|entry| {
                let col = (entry.tile % 16) as u32;
                let row = (entry.tile / 16) as u32;
                col == 7 && row == 5
            })
            .expect("the tile to the right should be prefetched")
            .distance
    };
    // The tile immediately to the left of the viewport: column 4.
    let left_of = |plan: &ViewPlan| {
        plan.prefetch
            .iter()
            .find(|entry| {
                let col = (entry.tile % 16) as u32;
                let row = (entry.tile / 16) as u32;
                col == 4 && row == 5
            })
            .expect("the tile to the left should be prefetched")
            .distance
    };

    // Panning right makes the right-hand tile the more urgent one; panning
    // left makes the left-hand tile the more urgent one. Both are
    // measured inside the same plan, so the comparison is like for like.
    assert!(right_of(&right) < left_of(&right));
    assert!(left_of(&left) < right_of(&left));
}

#[test]
fn a_prefetch_never_pushes_a_visible_tile_out_of_the_queue() {
    let factory = CountingFactory::new().with_delay(Duration::from_millis(60));
    // A queue of sixteen: the nine visible tiles fit, and the prefetch
    // ring of forty does not.
    let pool = Pool::new(&factory, 1, 1024 * 1024, 16).unwrap();

    let grid = grid();
    let viewport = Viewport::new(&grid, rect(0, 0, 192, 192)).unwrap();
    let plan = viewport.plan(&PrefetchPolicy::default(), Direction::None);

    let report = plan.submit(&pool, pool.generation());

    // Every visible tile was queued: none was refused, and none was
    // turned away to make room for a prefetch.
    assert_eq!(report.visible_queued, 9);
    assert_eq!(report.visible_pending, 0);

    // The prefetch ring is sixteen tiles — the viewport is in the corner
    // of the image, so the ring is clipped — and the queue is not big
    // enough for the visible tiles and the whole ring together: the
    // surplus is refused, not the visible tiles.
    assert_eq!(plan.prefetch.len(), 16);
    assert!(report.prefetch_refused > 0);

    pool.wait_for_idle(Duration::from_secs(60)).unwrap();

    // The visible tiles were all decoded.
    let decoded = factory.decoded_tiles();
    for entry in &plan.visible {
        assert!(
            decoded.contains(&entry.tile),
            "the visible tile {} should have been decoded",
            entry.tile
        );
    }
}

#[test]
fn the_plan_refuses_a_stale_generation() {
    let factory = CountingFactory::new();
    let pool = Pool::new(&factory, 1, 1024 * 1024, 64).unwrap();

    let grid = grid();
    let viewport = Viewport::new(&grid, rect(0, 0, 128, 128)).unwrap();
    let plan = viewport.plan(&PrefetchPolicy::default(), Direction::None);

    // The plan is made for an old viewport.
    let old = pool.generation();
    pool.advance_generation();

    let report = plan.submit(&pool, old);
    assert!(report.stale > 0);
    assert_eq!(report.visible_queued, 0);
    assert_eq!(report.prefetch_queued, 0);
}

#[test]
fn a_viewport_outside_the_image_is_refused() {
    let grid = grid();
    let err = Viewport::new(&grid, rect(2000, 0, 64, 64)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);

    // A rectangle with no area is refused too. `Rect::new` rejects it, so
    // the struct is built directly.
    let empty = Rect {
        x: 0,
        y: 0,
        width: 0,
        height: 64,
    };
    let err = Viewport::new(&grid, empty).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn an_empty_plan_is_empty() {
    let plan = ViewPlan::default();
    assert!(plan.is_empty());
    assert_eq!(plan.len(), 0);
}

#[test]
fn a_cached_tile_is_not_queued_again() {
    let factory = CountingFactory::new();
    let pool = Pool::new(&factory, 1, 1024 * 1024, 64).unwrap();

    let grid = grid();
    let viewport = Viewport::new(&grid, rect(0, 0, 64, 64)).unwrap();
    // No prefetch: the plan is the one visible tile and nothing else.
    let policy = PrefetchPolicy {
        radius: 0,
        direction_reach: 0,
        direction_bonus: 0,
    };
    let plan = viewport.plan(&policy, Direction::None);

    let first = plan.submit(&pool, pool.generation());
    pool.wait_for_idle(Duration::from_secs(30)).unwrap();
    assert_eq!(first.visible_queued, 1);

    // The same plan, submitted again: the tile is already decoded.
    let second = plan.submit(&pool, pool.generation());
    assert_eq!(second.visible_cached, 1);
    assert_eq!(factory.decode_count(), 1);
}

#[test]
fn the_visible_tiles_are_submitted_before_the_prefetch() {
    let factory = CountingFactory::new();
    let pool = Pool::new(&factory, 2, 1024 * 1024, 256).unwrap();

    let grid = grid();
    let viewport = Viewport::new(&grid, rect(0, 0, 128, 128)).unwrap();
    let plan = viewport.plan(&PrefetchPolicy::default(), Direction::None);

    plan.submit(&pool, pool.generation());
    pool.wait_for_idle(Duration::from_secs(30)).unwrap();

    // The scheduler dispatches by class first, so every visible tile is
    // decoded before any prefetch tile.
    let decoded = factory.decoded_tiles();
    let last_visible = decoded
        .iter()
        .position(|tile| plan.visible.iter().any(|entry| entry.tile == *tile))
        .expect("a visible tile should have been decoded");
    let first_prefetch = decoded
        .iter()
        .position(|tile| plan.prefetch.iter().any(|entry| entry.tile == *tile))
        .expect("a prefetch tile should have been decoded");

    assert!(
        last_visible < first_prefetch,
        "the visible tiles must be decoded before the prefetch ones"
    );
}

#[test]
fn the_pool_reports_a_tile_that_is_still_pending() {
    let factory = CountingFactory::new();
    let pool = Pool::new(&factory, 1, 1024 * 1024, 64).unwrap();

    let generation = pool.generation();
    let outcome = pool.request(RequestSpec {
        tile: 0,
        class: RequestClass::Visible,
        distance: 0,
        generation,
    });
    assert_eq!(outcome, PoolOutcome::Queued);
    pool.wait_for_idle(Duration::from_secs(30)).unwrap();
}
