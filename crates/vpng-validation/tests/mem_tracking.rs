//! The tracking allocator's own test, in its own test binary.
//!
//! The allocator's counters are **process-wide**: every thread's allocation
//! moves them. A test that measures "the counter went up by exactly one
//! megabyte" therefore races with every other thread in the process — the
//! test harness's own traffic is enough to make an exact comparison fail
//! for a reason that has nothing to do with the allocator. (The harness
//! frees a few dozen bytes inside the measurement window; the block
//! allocated there is four megabytes.)
//!
//! An integration test gets its own process, so this file runs alone of the
//! rest of the suite, and the two tests inside it take a lock so they do not
//! measure each other. The assertions allow a small tolerance for the
//! harness's residual traffic, and the tolerance is a thousandth of the
//! block — tight enough that a broken counter would still fail.

use std::sync::Mutex;

use vpng_validation::mem::{self, Tracking};

/// The tracking allocator, for this test binary.
#[global_allocator]
static TEST_ALLOCATOR: Tracking = Tracking;

/// The lock the two tests take, so they do not measure each other.
static LOCK: Mutex<()> = Mutex::new(());

/// The size of the block the counter test allocates.
const BLOCK: usize = 4 * 1024 * 1024;

/// The slack the harness's own traffic is allowed.
const TOLERANCE: usize = 64 * 1024;

/// The counter follows the allocations: up when they are made, down when
/// they are released.
#[test]
fn the_heap_counter_follows_the_allocations() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    let before = mem::current_heap_bytes();
    let block = vec![0u8; BLOCK];
    let during = mem::current_heap_bytes();
    assert!(
        during + TOLERANCE >= before + BLOCK,
        "the counter did not see the allocation: {before} -> {during} (block {BLOCK})"
    );

    drop(block);
    let after = mem::current_heap_bytes();
    assert!(
        after + TOLERANCE <= during,
        "the counter did not see the deallocation: {during} -> {after}"
    );
    assert!(
        after < before + BLOCK,
        "the counter still holds the released block: {before} -> {after} (block {BLOCK})"
    );
}

/// The peak is a high-water mark: it does not come back down.
#[test]
fn the_heap_peak_never_falls() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    let before = mem::peak_heap_bytes();
    let block = vec![0u8; BLOCK];
    let during = mem::peak_heap_bytes();
    assert!(during >= before, "the peak went down: {before} -> {during}");
    drop(block);
    assert_eq!(mem::peak_heap_bytes(), during);
}
