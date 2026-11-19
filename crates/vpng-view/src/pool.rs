//! The worker pool.
//!
//! The pool is the parallel half of the viewer: a fixed set of worker
//! threads that take requests from the scheduler, decode them, and publish
//! the results into the cache.
//!
//! # The rules
//!
//! * **The lock is never held across a decode.** The workers share one
//!   mutex, but they hold it only while they take a request or publish a
//!   result. The decode — the expensive part — happens with the lock
//!   released, so the workers actually run in parallel.
//! * **One decode per tile.** A tile that is being decoded is *in flight*,
//!   and a request for it does not start a second decode: the caller is
//!   told that the work is already under way.
//! * **A stale result is dropped.** A worker that finishes a decode after
//!   the viewport has moved on throws the pixels away and counts it
//!   separately from a failure. The user panned away; the decoder did not
//!   break.
//! * **A failed decode does not stop the pool.** The failure is recorded
//!   against the tile, the worker carries on with the next request, and
//!   the caller is told when it waits for that tile.
//! * **The pixels are the same pixels.** The pool changes *when* a tile is
//!   decoded, never *what* it decodes: every worker uses the same codec
//!   and the same checksum check as the ordinary decoder.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use vpng_cache::{CacheStats, TileCache};
use vpng_core::{Error, Result};

use crate::driver::TileDecoder;
use crate::scheduler::{Counters, Generation, RequestClass, RequestSpec, Scheduler, SubmitOutcome};

/// A source of decoders: one per worker.
///
/// Each worker gets its own decoder, because a decoder holds mutable state
/// — a file position, for one — and sharing that would serialise the
/// workers.
pub trait DecoderFactory: Send + Sync + 'static {
    /// The decoder a worker uses.
    type Decoder: TileDecoder + Send + 'static;

    /// Make a decoder for one worker.
    ///
    /// The pool passes its viewport-generation handle, so a decoder that
    /// runs on a worker thread can see that the viewport moved while the
    /// tile was being decoded — the cue to throw the result away rather
    /// than show a tile the user has panned off.
    fn make_decoder(&self, generation: Generation) -> Result<Self::Decoder>;
}

/// What the pool did with a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolOutcome {
    /// The tile was already decoded and is still in the cache: nothing to
    /// do.
    Cached,
    /// The request was queued for a worker.
    Queued,
    /// The tile was already queued; the queued request was made more
    /// urgent instead of being duplicated.
    Promoted,
    /// The tile was already queued at the same urgency or a better one.
    AlreadyPending,
    /// The tile is being decoded right now; no second decode was started.
    AlreadyInFlight,
    /// The queue was full and the request was not urgent enough to
    /// displace anything.
    RejectedFull,
    /// The request belonged to an older viewport generation.
    RejectedStale,
}

/// The state the workers and the caller share.
#[derive(Debug)]
struct Shared {
    /// The request queue.
    scheduler: Scheduler,
    /// The decoded tiles.
    cache: TileCache,
    /// The tiles currently being decoded.
    in_flight: HashSet<usize>,
    /// The generation that last wanted each in-flight tile.
    ///
    /// A tile that is being decoded for an old viewport may be wanted
    /// again by the new one — the user panned back, or the new viewport
    /// overlaps the old one. The pixels do not depend on the viewport, so
    /// the result is worth keeping; this map is how the worker knows.
    wanted: HashMap<usize, u64>,
    /// The tiles whose decode was thrown away because the viewport moved
    /// on.
    discarded_tiles: HashSet<usize>,
    /// The tiles whose decode failed, with the reason.
    failures: HashMap<usize, String>,
    /// The decodes that completed.
    completed: u64,
    /// The decodes that failed.
    failed: u64,
    /// The results thrown away because the viewport moved on.
    discarded: u64,
    /// The results that landed after a pan but were wanted again.
    recovered: u64,
    /// The bytes the workers have read from their sources.
    bytes_read: u64,
    /// The visible requests the cache satisfied.
    visible_hits: u64,
    /// The visible requests the cache did not hold.
    visible_misses: u64,
    /// The speculative requests the cache satisfied.
    speculative_hits: u64,
    /// The speculative requests the cache did not hold.
    speculative_misses: u64,
    /// Whether the workers should stop.
    shutdown: bool,
}

/// The mutex and the condition variable the workers and the caller share.
#[derive(Debug)]
struct Inner {
    state: Mutex<Shared>,
    /// Signalled when work arrives or a result lands.
    wake: Condvar,
}

/// What the pool reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PoolStats {
    /// The number of worker threads.
    pub workers: u64,
    /// The decodes that completed.
    pub completed: u64,
    /// The decodes that failed.
    pub failed: u64,
    /// The results thrown away because the viewport moved on.
    pub discarded: u64,
    /// The results that landed after a pan but were wanted again, and were
    /// therefore kept.
    pub recovered: u64,
    /// The bytes the workers have read from their sources.
    pub bytes_read: u64,
    /// The visible requests the cache satisfied.
    pub visible_hits: u64,
    /// The visible requests the cache did not hold.
    pub visible_misses: u64,
    /// The speculative requests the cache satisfied.
    pub speculative_hits: u64,
    /// The speculative requests the cache did not hold.
    pub speculative_misses: u64,
    /// The tiles being decoded right now.
    pub in_flight: u64,
    /// The scheduler's counters.
    pub scheduler: Counters,
    /// The cache's counters.
    pub cache: CacheStats,
}

impl PoolStats {
    /// The share of the *visible* requests the cache satisfied.
    ///
    /// The speculative requests are counted apart, because a prefetch that
    /// misses is not a failure: it is a guess that has not come true yet.
    /// The visible hit rate is the figure that says whether the cache is
    /// doing its job.
    pub fn visible_hit_rate(&self) -> Option<f64> {
        let total = self.visible_hits + self.visible_misses;
        if total == 0 {
            None
        } else {
            Some(self.visible_hits as f64 / total as f64)
        }
    }

    /// The share of the speculative requests the cache satisfied.
    pub fn speculative_hit_rate(&self) -> Option<f64> {
        let total = self.speculative_hits + self.speculative_misses;
        if total == 0 {
            None
        } else {
            Some(self.speculative_hits as f64 / total as f64)
        }
    }
}

/// A pool of worker threads that decode tiles.
pub struct Pool {
    inner: Arc<Inner>,
    workers: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("workers", &self.workers.len())
            .finish()
    }
}

impl Pool {
    /// Start a pool.
    ///
    /// The pool queues at most `queue_capacity` requests and holds at most
    /// `cache_bytes` of decoded pixels. A worker that cannot be given a
    /// decoder is a startup failure, not a silent reduction of the pool:
    /// the caller asked for *n* workers and gets an error instead of fewer.
    pub fn new<F: DecoderFactory>(
        factory: &F,
        workers: usize,
        cache_bytes: u64,
        queue_capacity: usize,
    ) -> Result<Self> {
        let shared = Shared {
            scheduler: Scheduler::new(queue_capacity),
            cache: TileCache::new(cache_bytes)?,
            in_flight: HashSet::new(),
            wanted: HashMap::new(),
            discarded_tiles: HashSet::new(),
            failures: HashMap::new(),
            completed: 0,
            failed: 0,
            discarded: 0,
            recovered: 0,
            bytes_read: 0,
            visible_hits: 0,
            visible_misses: 0,
            speculative_hits: 0,
            speculative_misses: 0,
            shutdown: false,
        };

        let inner = Arc::new(Inner {
            state: Mutex::new(shared),
            wake: Condvar::new(),
        });

        // Every decoder gets the same generation handle, so a worker can
        // tell that the viewport moved while its tile was being decoded.
        let generation = self_generation(&inner);

        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let decoder = factory.make_decoder(generation.clone())?;
            let inner = Arc::clone(&inner);
            let handle = std::thread::Builder::new()
                .name("vpng-view-worker".to_string())
                .spawn(move || worker_loop(inner, decoder))?;
            handles.push(handle);
        }

        Ok(Pool {
            inner,
            workers: handles,
        })
    }

    /// The number of worker threads.
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// The current viewport generation.
    pub fn generation(&self) -> u64 {
        self.lock().scheduler.generation()
    }

    /// Whether a generation is the current one.
    pub fn is_current(&self, generation: u64) -> bool {
        self.lock().scheduler.is_current(generation)
    }

    /// A handle to the viewport generation.
    ///
    /// A decoder that runs on a worker thread can use it to report that the
    /// viewport moved while the tile was being decoded.
    pub fn generation_handle(&self) -> crate::scheduler::Generation {
        self.lock().scheduler.generation_handle()
    }

    /// Advance to a new viewport generation.
    ///
    /// Every queued request from an older generation is dropped, and the
    /// results of the decodes that are still running are discarded when
    /// they land.
    pub fn advance_generation(&self) -> u64 {
        let generation = {
            let mut shared = self.lock();
            shared.scheduler.advance_generation()
        };
        self.inner.wake.notify_all();
        generation
    }

    /// Queue a request.
    ///
    /// The tile is looked up in the cache first: a tile that is already
    /// resident is not decoded again. A tile that is being decoded right
    /// now is not decoded a second time either.
    pub fn request(&self, spec: RequestSpec) -> PoolOutcome {
        let mut shared = self.lock();

        // A request from an older viewport is refused before anything
        // else: the viewport it was made for is gone. The scheduler
        // counts the refusal.
        if spec.generation != shared.scheduler.generation() {
            shared.scheduler.submit(spec);
            return PoolOutcome::RejectedStale;
        }

        // The cache lookup counts a hit or a miss, so the hit rate
        // measures how much of the demand the cache satisfied. The
        // visible demand is counted apart from the speculative one: a
        // prefetch that misses is a guess that has not come true, not a
        // failure of the cache.
        let hit = shared.cache.note_access(spec.tile);
        match (spec.class, hit) {
            (RequestClass::Visible, true) => shared.visible_hits += 1,
            (RequestClass::Visible, false) => shared.visible_misses += 1,
            (_, true) => shared.speculative_hits += 1,
            (_, false) => shared.speculative_misses += 1,
        }
        if hit {
            return PoolOutcome::Cached;
        }

        // One decode per tile. The tile is already being decoded — but
        // the decode may be for an older viewport, and the pixels do not
        // depend on the viewport. The tile is marked as wanted under the
        // current generation, so the worker keeps the result when it
        // lands instead of throwing it away.
        if shared.in_flight.contains(&spec.tile) {
            shared.wanted.insert(spec.tile, spec.generation);
            return PoolOutcome::AlreadyInFlight;
        }

        let outcome = shared.scheduler.submit(spec);
        let pool_outcome = match outcome {
            SubmitOutcome::Accepted => PoolOutcome::Queued,
            SubmitOutcome::Promoted => PoolOutcome::Promoted,
            SubmitOutcome::AlreadyPending => PoolOutcome::AlreadyPending,
            SubmitOutcome::RejectedFull => PoolOutcome::RejectedFull,
            SubmitOutcome::RejectedStale => PoolOutcome::RejectedStale,
        };

        if matches!(pool_outcome, PoolOutcome::Queued | PoolOutcome::Promoted) {
            // A new attempt replaces the old failure, if there was one.
            shared.failures.remove(&spec.tile);
            shared.wanted.insert(spec.tile, spec.generation);
        }

        drop(shared);
        if matches!(pool_outcome, PoolOutcome::Queued | PoolOutcome::Promoted) {
            self.inner.wake.notify_all();
        }
        pool_outcome
    }

    /// Whether a tile is being decoded right now.
    pub fn is_in_flight(&self, tile: usize) -> bool {
        self.lock().in_flight.contains(&tile)
    }

    /// Whether a tile has been decoded and is still in the cache.
    pub fn contains(&self, tile: usize) -> bool {
        self.lock().cache.contains(tile)
    }

    /// Run a function over a decoded tile, without copying it.
    ///
    /// The lookup does not count a hit or a miss: the request that
    /// produced the tile already did.
    pub fn with_tile<R>(&self, tile: usize, f: impl FnOnce(&[u8]) -> R) -> Option<R> {
        let shared = self.lock();
        let bytes = shared.cache.peek(tile)?;
        Some(f(bytes))
    }

    /// The decoded pixels of a tile, copied.
    pub fn tile(&self, tile: usize) -> Option<Vec<u8>> {
        self.with_tile(tile, |bytes| bytes.to_vec())
    }

    /// Wait until a tile has been decoded.
    ///
    /// The wait ends when the tile is in the cache, when its decode failed,
    /// or when the timeout expires. A tile that cannot be decoded — because
    /// the queue is empty and nothing is in flight — is an error, not a
    /// wait that never ends.
    pub fn wait_for_tile(&self, tile: usize, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut shared = self.lock();
        loop {
            if shared.cache.contains(tile) {
                return Ok(());
            }
            if let Some(reason) = shared.failures.get(&tile) {
                return Err(Error::corrupt(format!(
                    "tile {tile} could not be decoded: {reason}"
                )));
            }
            if shared.discarded_tiles.contains(&tile) {
                return Err(Error::internal(format!(
                    "tile {tile} was decoded and thrown away: the viewport moved on before it \
                     landed"
                )));
            }
            if shared.scheduler.is_empty() && shared.in_flight.is_empty() {
                return Err(Error::internal(format!(
                    "tile {tile} was never decoded and nothing is in flight"
                )));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::internal(format!(
                    "timed out after {timeout:?} waiting for tile {tile}"
                )));
            }
            shared = self
                .inner
                .wake
                .wait_timeout(shared, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// Wait until the queue is empty and nothing is being decoded.
    pub fn wait_for_idle(&self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut shared = self.lock();
        loop {
            if shared.scheduler.is_empty() && shared.in_flight.is_empty() {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::internal(format!(
                    "timed out after {timeout:?} waiting for the pool to become idle"
                )));
            }
            shared = self
                .inner
                .wake
                .wait_timeout(shared, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// The pool's statistics.
    pub fn stats(&self) -> PoolStats {
        let shared = self.lock();
        PoolStats {
            workers: self.workers.len() as u64,
            completed: shared.completed,
            failed: shared.failed,
            discarded: shared.discarded,
            recovered: shared.recovered,
            bytes_read: shared.bytes_read,
            visible_hits: shared.visible_hits,
            visible_misses: shared.visible_misses,
            speculative_hits: shared.speculative_hits,
            speculative_misses: shared.speculative_misses,
            in_flight: shared.in_flight.len() as u64,
            scheduler: *shared.scheduler.counters(),
            cache: shared.cache.stats(),
        }
    }

    /// The scheduler's counters.
    pub fn scheduler_counters(&self) -> Counters {
        *self.lock().scheduler.counters()
    }

    /// The cache's counters.
    pub fn cache_stats(&self) -> CacheStats {
        self.lock().cache.stats()
    }

    /// Stop the workers and wait for them to finish.
    pub fn shutdown(mut self) {
        {
            let mut shared = self.lock();
            shared.shutdown = true;
        }
        self.inner.wake.notify_all();
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }

    /// Lock the shared state, recovering from a poisoned lock.
    ///
    /// A panic in a worker poisons the mutex; the engine carries on rather
    /// than refusing to work for the rest of the session.
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        {
            let mut shared = self.lock();
            shared.shutdown = true;
        }
        self.inner.wake.notify_all();
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }
}

/// The generation handle of the pool's scheduler.
fn self_generation(inner: &Arc<Inner>) -> Generation {
    inner
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .scheduler
        .generation_handle()
}

/// The worker loop.
fn worker_loop<D: TileDecoder>(inner: Arc<Inner>, mut decoder: D) {
    loop {
        // Take a request. The lock is released before the decode.
        let request = loop {
            let mut shared = inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if shared.shutdown {
                return;
            }
            if let Some(request) = shared.scheduler.dispatch() {
                shared.in_flight.insert(request.tile);
                break request;
            }
            // Nothing to do: wait for work.
            shared = inner
                .wake
                .wait(shared)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        };

        // The decode, with the lock released.
        let started = Instant::now();
        let bytes_before = decoder.bytes_read();
        let result = decoder.decode(request.tile);
        let elapsed = started.elapsed();
        let bytes_after = decoder.bytes_read();

        // Publish the result.
        let mut shared = inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        shared.in_flight.remove(&request.tile);
        shared.bytes_read += bytes_after.saturating_sub(bytes_before);
        match result {
            Ok(pixels) => {
                shared.cache.record_decode(elapsed);

                // The result is kept when the viewport that asked for it is
                // still current, or when the current viewport wants the
                // same tile: the pixels do not depend on the viewport, so
                // a tile the user has panned back onto is still the right
                // tile.
                let current = shared.scheduler.generation();
                let still_wanted = shared.wanted.get(&request.tile) == Some(&current);
                shared.wanted.remove(&request.tile);

                if shared.scheduler.is_current(request.generation) || still_wanted {
                    shared.cache.insert(request.tile, pixels);
                    shared.completed += 1;
                    if !shared.scheduler.is_current(request.generation) {
                        // The decode was for an older viewport, but the
                        // tile is wanted again: the pixels were kept.
                        shared.recovered += 1;
                    }
                } else {
                    // The viewport moved on while this tile was being
                    // decoded, and nothing wants the tile any more. The
                    // pixels are not wanted.
                    shared.scheduler.record_discarded_stale();
                    shared.discarded += 1;
                    shared.discarded_tiles.insert(request.tile);
                }
            }
            Err(err) => {
                shared.wanted.remove(&request.tile);
                shared.failures.insert(request.tile, err.to_string());
                shared.failed += 1;
            }
        }
        drop(shared);
        inner.wake.notify_all();
    }
}
