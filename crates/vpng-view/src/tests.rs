//! The scheduler's tests.
//!
//! The scheduler's promises are exact: a visible tile is dispatched before
//! a prefetched one, the nearer tile first, equal requests in arrival
//! order, a stale request never dispatched, a tile never queued twice, and
//! the queue never grows past its bound. Each is checked here.

use vpng_core::Result;

use crate::{
    DecodedTile, Driver, Generation, RequestClass, RequestSpec, Scheduler, Step, SubmitOutcome,
    TileDecoder,
};

/// A request for a tile, in the current generation.
fn request(scheduler: &Scheduler, tile: usize, class: RequestClass, distance: u32) -> RequestSpec {
    RequestSpec {
        tile,
        class,
        distance,
        generation: scheduler.generation(),
    }
}

/// A decoder that returns a predictable pixel value per tile.
struct FakeDecoder {
    decoded: Vec<usize>,
    fail_on: Option<usize>,
}

impl FakeDecoder {
    fn new() -> Self {
        FakeDecoder {
            decoded: Vec::new(),
            fail_on: None,
        }
    }

    fn failing_on(tile: usize) -> Self {
        FakeDecoder {
            decoded: Vec::new(),
            fail_on: Some(tile),
        }
    }
}

impl TileDecoder for FakeDecoder {
    fn decode(&mut self, tile: usize) -> Result<Vec<u8>> {
        self.decoded.push(tile);
        if self.fail_on == Some(tile) {
            return Err(vpng_core::Error::corrupt(
                "the fake decoder was told to fail",
            ));
        }
        Ok(vec![tile as u8; 4])
    }
}

#[test]
fn a_visible_request_is_dispatched_before_a_prefetch() {
    let mut scheduler = Scheduler::new(16);

    // The prefetch is submitted first, so only the class can order them.
    scheduler.submit(request(&scheduler, 7, RequestClass::Prefetch, 0));
    scheduler.submit(request(&scheduler, 3, RequestClass::Visible, 5));

    assert_eq!(scheduler.dispatch().unwrap().tile, 3);
    assert_eq!(scheduler.dispatch().unwrap().tile, 7);
}

#[test]
fn the_nearer_tile_is_dispatched_first() {
    let mut scheduler = Scheduler::new(16);

    scheduler.submit(request(&scheduler, 9, RequestClass::Visible, 12));
    scheduler.submit(request(&scheduler, 4, RequestClass::Visible, 2));
    scheduler.submit(request(&scheduler, 6, RequestClass::Visible, 7));

    assert_eq!(scheduler.dispatch().unwrap().tile, 4);
    assert_eq!(scheduler.dispatch().unwrap().tile, 6);
    assert_eq!(scheduler.dispatch().unwrap().tile, 9);
}

#[test]
fn equally_urgent_requests_keep_their_arrival_order() {
    let mut scheduler = Scheduler::new(16);

    for tile in [11, 2, 8, 5] {
        scheduler.submit(request(&scheduler, tile, RequestClass::Visible, 3));
    }

    let order: Vec<usize> = std::iter::from_fn(|| scheduler.dispatch().map(|r| r.tile)).collect();
    assert_eq!(order, vec![11, 2, 8, 5]);
}

#[test]
fn a_class_outranks_a_distance() {
    // A visible tile that is far still beats a prefetch that is close: the
    // class is the first key, not the distance.
    let mut scheduler = Scheduler::new(16);

    scheduler.submit(request(&scheduler, 1, RequestClass::Visible, 99));
    scheduler.submit(request(&scheduler, 2, RequestClass::Prefetch, 0));

    assert_eq!(scheduler.dispatch().unwrap().tile, 1);
}

#[test]
fn a_full_queue_refuses_a_prefetch_request() {
    let mut scheduler = Scheduler::new(2);

    assert_eq!(
        scheduler.submit(request(&scheduler, 1, RequestClass::Visible, 0)),
        SubmitOutcome::Accepted
    );
    assert_eq!(
        scheduler.submit(request(&scheduler, 2, RequestClass::Visible, 0)),
        SubmitOutcome::Accepted
    );

    // The queue is full. A prefetch is refused, and the caller is not
    // blocked.
    assert_eq!(
        scheduler.submit(request(&scheduler, 3, RequestClass::Prefetch, 0)),
        SubmitOutcome::RejectedFull
    );
    assert_eq!(
        scheduler.submit(request(&scheduler, 4, RequestClass::Background, 0)),
        SubmitOutcome::RejectedFull
    );
    assert_eq!(scheduler.len(), 2);
    assert_eq!(scheduler.counters().rejected_full, 2);
}

#[test]
fn a_visible_request_displaces_the_least_urgent_request() {
    let mut scheduler = Scheduler::new(2);

    scheduler.submit(request(&scheduler, 1, RequestClass::Visible, 4));
    scheduler.submit(request(&scheduler, 2, RequestClass::Prefetch, 9));

    // The queue is full, but a visible request is never refused: it takes
    // the place of the least urgent queued request.
    assert_eq!(
        scheduler.submit(request(&scheduler, 3, RequestClass::Visible, 0)),
        SubmitOutcome::Accepted
    );
    assert_eq!(scheduler.len(), 2);
    assert!(!scheduler.contains(2));

    // The prefetch was the one that went; the visible tile stayed.
    let first = scheduler.dispatch().unwrap();
    assert_eq!(first.tile, 3);
    assert_eq!(scheduler.dispatch().unwrap().tile, 1);
}

#[test]
fn advancing_the_generation_cancels_the_queued_requests() {
    let mut scheduler = Scheduler::new(16);

    scheduler.submit(request(&scheduler, 1, RequestClass::Visible, 0));
    scheduler.submit(request(&scheduler, 2, RequestClass::Visible, 0));

    // The user panned. The old viewport's requests are worthless.
    scheduler.advance_generation();

    assert!(scheduler.is_empty());
    assert_eq!(scheduler.counters().cancelled, 2);
    assert_eq!(scheduler.dispatch(), None);
}

#[test]
fn a_request_from_an_old_generation_is_refused() {
    let mut scheduler = Scheduler::new(16);
    let old = scheduler.generation();

    scheduler.advance_generation();

    let outcome = scheduler.submit(RequestSpec {
        tile: 1,
        class: RequestClass::Visible,
        distance: 0,
        generation: old,
    });
    assert_eq!(outcome, SubmitOutcome::RejectedStale);
    assert!(scheduler.is_empty());
    assert_eq!(scheduler.counters().rejected_stale, 1);
}

#[test]
fn a_tile_is_not_queued_twice() {
    let mut scheduler = Scheduler::new(16);

    assert_eq!(
        scheduler.submit(request(&scheduler, 5, RequestClass::Visible, 0)),
        SubmitOutcome::Accepted
    );
    assert_eq!(
        scheduler.submit(request(&scheduler, 5, RequestClass::Visible, 0)),
        SubmitOutcome::AlreadyPending
    );
    assert_eq!(scheduler.len(), 1);
    assert_eq!(scheduler.counters().deduplicated, 1);
}

#[test]
fn a_promoted_request_becomes_more_urgent() {
    let mut scheduler = Scheduler::new(16);

    // The tile is prefetched...
    scheduler.submit(request(&scheduler, 5, RequestClass::Prefetch, 0));
    scheduler.submit(request(&scheduler, 6, RequestClass::Visible, 9));

    // ...and then the user pans onto it.
    assert_eq!(
        scheduler.submit(request(&scheduler, 5, RequestClass::Visible, 0)),
        SubmitOutcome::Promoted
    );

    // It is dispatched first, and the queue did not grow.
    assert_eq!(scheduler.len(), 2);
    assert_eq!(scheduler.dispatch().unwrap().tile, 5);
}

#[test]
fn the_counters_add_up() {
    let mut scheduler = Scheduler::new(4);

    scheduler.submit(request(&scheduler, 1, RequestClass::Visible, 0));
    scheduler.submit(request(&scheduler, 1, RequestClass::Visible, 0));
    scheduler.submit(request(&scheduler, 2, RequestClass::Prefetch, 0));
    scheduler.submit(request(&scheduler, 3, RequestClass::Prefetch, 0));
    scheduler.submit(request(&scheduler, 4, RequestClass::Prefetch, 0));
    scheduler.submit(request(&scheduler, 5, RequestClass::Prefetch, 0));

    scheduler.dispatch();
    scheduler.dispatch();
    scheduler.advance_generation();

    let counters = scheduler.counters();
    assert_eq!(counters.submitted, 6);
    assert_eq!(counters.accepted, 4);
    assert_eq!(counters.deduplicated, 1);
    assert_eq!(counters.rejected_full, 1);
    assert_eq!(counters.dispatched, 2);
    assert_eq!(counters.cancelled, 2);
}

#[test]
fn the_driver_decodes_in_priority_order() {
    let mut scheduler = Scheduler::new(16);
    scheduler.submit(request(&scheduler, 7, RequestClass::Prefetch, 0));
    scheduler.submit(request(&scheduler, 3, RequestClass::Visible, 5));
    scheduler.submit(request(&scheduler, 4, RequestClass::Visible, 1));

    let mut driver = Driver::new(scheduler, FakeDecoder::new());
    let decoded = driver.drain().unwrap();

    let order: Vec<usize> = decoded.iter().map(|t| t.tile).collect();
    assert_eq!(order, vec![4, 3, 7]);
    assert_eq!(decoded[0].pixels, vec![4, 4, 4, 4]);
}

#[test]
fn the_driver_reports_an_empty_queue_as_idle() {
    let mut driver = Driver::new(Scheduler::new(4), FakeDecoder::new());
    assert_eq!(driver.step().unwrap(), Step::Idle);
}

#[test]
fn the_driver_discards_a_result_the_user_panned_away_from() {
    // The request is dispatched under one generation, and the viewport
    // moves while the tile is being decoded.
    struct PanickingDecoder {
        generation: Generation,
    }

    impl TileDecoder for PanickingDecoder {
        fn decode(&mut self, _tile: usize) -> Result<Vec<u8>> {
            // The viewport moves while this "decode" is in progress.
            self.generation.advance();
            Ok(vec![1, 2, 3, 4])
        }
    }

    let mut scheduler = Scheduler::new(16);
    let generation = scheduler.generation_handle();
    scheduler.submit(request(&scheduler, 1, RequestClass::Visible, 0));

    let mut driver = Driver::new(scheduler, PanickingDecoder { generation });

    // The pixels were produced, but nobody wants them any more. That is
    // not an error.
    assert_eq!(driver.step().unwrap(), Step::Discarded);
    assert_eq!(driver.scheduler().counters().discarded_stale, 1);
}

#[test]
fn a_pan_cancels_the_queued_requests_before_they_are_dispatched() {
    let mut scheduler = Scheduler::new(16);
    scheduler.submit(request(&scheduler, 1, RequestClass::Visible, 0));
    scheduler.submit(request(&scheduler, 2, RequestClass::Visible, 0));

    // The user panned while the queue was still full.
    scheduler.advance_generation();

    let mut driver = Driver::new(scheduler, FakeDecoder::new());
    assert_eq!(driver.step().unwrap(), Step::Idle);
    assert_eq!(driver.scheduler().counters().cancelled, 2);
    assert_eq!(driver.scheduler().counters().dispatched, 0);
}

#[test]
fn the_driver_returns_a_tile_the_viewport_still_wants() {
    let mut scheduler = Scheduler::new(16);
    scheduler.submit(request(&scheduler, 1, RequestClass::Visible, 0));

    let mut driver = Driver::new(scheduler, FakeDecoder::new());
    assert_eq!(
        driver.step().unwrap(),
        Step::Decoded(DecodedTile {
            tile: 1,
            pixels: vec![1, 1, 1, 1],
        })
    );
    assert_eq!(driver.scheduler().counters().discarded_stale, 0);
}

#[test]
fn a_decoder_failure_is_reported_as_an_error() {
    let mut scheduler = Scheduler::new(16);
    scheduler.submit(request(&scheduler, 3, RequestClass::Visible, 0));

    let mut driver = Driver::new(scheduler, FakeDecoder::failing_on(3));
    let err = driver.step().unwrap_err();
    assert_eq!(err.kind(), vpng_core::ErrorKind::CorruptFile);
}
