//! The bounded request scheduler.
//!
//! A viewer of a huge image does not decode it in order. It decodes the
//! tiles the user is looking at, first; the tiles they are probably about
//! to look at, second; and nothing else until it has to. When the user
//! pans, the requests for the old viewport are worthless, and the scheduler
//! drops them rather than spending the CPU on tiles nobody will see.
//!
//! # The rules
//!
//! * **Priority.** A request is dispatched in the order
//!   `(class, distance, sequence)`: the class first — a visible tile before
//!   a prefetched one, a prefetched one before a background one — then the
//!   distance from the centre of the viewport, then the order in which the
//!   request was made. The last key means that two equally urgent requests
//!   are dispatched in the order they arrived, so nothing is starved.
//! * **A bounded queue.** The queue holds at most *N* requests. A full
//!   queue refuses a `Prefetch` or `Background` request and does not block
//!   the caller. A `Visible` request is never refused: when the queue is
//!   full it displaces the least urgent request in it, because the tile the
//!   user is looking at matters more than the tile we guessed they might
//!   want.
//! * **Cancellation by generation.** Every request carries the *viewport
//!   generation* it was issued under. A new viewport advances the
//!   generation, and every queued request from an older generation is
//!   dropped before anything else is dispatched. A request that is already
//!   running is allowed to finish — stopping a decode half-way would throw
//!   the work away — but its result is discarded if its generation is stale.
//! * **One request per tile.** A tile that is already queued is not queued
//!   again. If the new request is more urgent than the old one, the queued
//!   request is *promoted* rather than duplicated.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A shared handle to the viewport generation.
///
/// A worker that decodes a tile needs to know, when it finishes, whether
/// the viewport it was decoding for is still the current one. The handle
/// lets it ask without locking the whole scheduler — and it lets a decoder
/// that runs on another thread report that the user has panned.
#[derive(Debug, Clone)]
pub struct Generation(Arc<AtomicU64>);

impl Generation {
    /// The current generation.
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    /// Advance to the next generation and return it.
    pub fn advance(&self) -> u64 {
        self.0.fetch_add(1, Ordering::AcqRel) + 1
    }
}

/// What a request is for.
///
/// The order of the variants is the order of priority: `Visible` first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RequestClass {
    /// The tile is in the viewport: the user is looking at it now.
    Visible,
    /// The tile is likely to be needed next.
    Prefetch,
    /// The tile is wanted, but not urgently.
    Background,
}

/// A request for one tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The tile's index in the directory.
    pub tile: usize,
    /// How urgent the request is.
    pub class: RequestClass,
    /// The viewport generation the request was issued under.
    pub generation: u64,
    /// The tile's distance from the centre of the viewport, in tiles.
    pub distance: u32,
    /// The request's arrival order, used to break ties.
    pub sequence: u64,
}

/// A request as the caller states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestSpec {
    /// The tile's index in the directory.
    pub tile: usize,
    /// How urgent the request is.
    pub class: RequestClass,
    /// The tile's distance from the centre of the viewport, in tiles.
    pub distance: u32,
    /// The viewport generation the request belongs to. A request from an
    /// older generation is refused: the viewport it was made for is gone.
    pub generation: u64,
}

/// What the scheduler did with a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// The request was queued.
    Accepted,
    /// The tile was already queued; the queued request was made more
    /// urgent instead of being duplicated.
    Promoted,
    /// The tile was already queued at the same urgency or a better one.
    AlreadyPending,
    /// The queue was full and the request was not urgent enough to
    /// displace anything.
    RejectedFull,
    /// The request belonged to an older viewport generation.
    RejectedStale,
}

/// The scheduler's counters.
///
/// Every number here is a count of something that happened, so a report can
/// state it rather than estimate it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// The requests submitted.
    pub submitted: u64,
    /// The requests queued.
    pub accepted: u64,
    /// The queued requests that were made more urgent.
    pub promoted: u64,
    /// The requests that arrived for a tile that was already queued.
    pub deduplicated: u64,
    /// The requests refused because the queue was full.
    pub rejected_full: u64,
    /// The requests refused because their viewport generation was old.
    pub rejected_stale: u64,
    /// The requests handed to a worker.
    pub dispatched: u64,
    /// The queued requests dropped because the viewport moved on.
    pub cancelled: u64,
    /// The decoded results discarded because the viewport moved on.
    pub discarded_stale: u64,
}

/// One entry in the heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Entry {
    class: RequestClass,
    distance: u32,
    sequence: u64,
    tile: usize,
}

/// The bounded request queue.
#[derive(Debug)]
pub struct Scheduler {
    /// The maximum number of queued requests.
    capacity: usize,
    /// The current viewport generation.
    generation: Generation,
    /// The last sequence number handed out.
    sequence: u64,
    /// The queued requests, by tile.
    pending: HashMap<usize, Request>,
    /// The heap of queued requests.
    ///
    /// Entries are removed lazily: a request that is promoted or cancelled
    /// leaves its old entry behind, and the entry is skipped when it is
    /// popped.
    heap: BinaryHeap<Reverse<Entry>>,
    /// The counters.
    counters: Counters,
}

impl Scheduler {
    /// A scheduler that queues at most `capacity` requests.
    ///
    /// A capacity of zero is legal: every request is refused.
    pub fn new(capacity: usize) -> Self {
        Scheduler {
            capacity,
            generation: Generation(Arc::new(AtomicU64::new(0))),
            sequence: 0,
            pending: HashMap::new(),
            heap: BinaryHeap::new(),
            counters: Counters::default(),
        }
    }

    /// The maximum number of queued requests.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The current viewport generation.
    pub fn generation(&self) -> u64 {
        self.generation.get()
    }

    /// A handle to the viewport generation, for a decoder that runs
    /// elsewhere.
    pub fn generation_handle(&self) -> Generation {
        self.generation.clone()
    }

    /// The number of queued requests.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// The counters.
    pub fn counters(&self) -> &Counters {
        &self.counters
    }

    /// Whether a tile is queued.
    pub fn contains(&self, tile: usize) -> bool {
        self.pending.contains_key(&tile)
    }

    /// The queued request for a tile.
    pub fn request(&self, tile: usize) -> Option<&Request> {
        self.pending.get(&tile)
    }

    /// Whether a generation is the current one.
    ///
    /// A worker uses this to decide whether the result it just produced is
    /// still wanted.
    pub fn is_current(&self, generation: u64) -> bool {
        generation == self.generation.get()
    }

    /// Queue a request.
    pub fn submit(&mut self, spec: RequestSpec) -> SubmitOutcome {
        self.counters.submitted += 1;

        if spec.generation != self.generation() {
            self.counters.rejected_stale += 1;
            return SubmitOutcome::RejectedStale;
        }

        // A tile that is already queued is not queued twice.
        if let Some(existing) = self.pending.get(&spec.tile) {
            let existing = existing.clone();
            if spec.class < existing.class {
                // The new request is more urgent: promote the queued one.
                self.sequence += 1;
                let request = Request {
                    tile: spec.tile,
                    class: spec.class,
                    generation: self.generation(),
                    distance: spec.distance,
                    sequence: self.sequence,
                };
                self.pending.insert(spec.tile, request.clone());
                self.push(request);
                self.counters.promoted += 1;
                return SubmitOutcome::Promoted;
            }
            self.counters.deduplicated += 1;
            return SubmitOutcome::AlreadyPending;
        }

        if self.pending.len() >= self.capacity {
            if spec.class != RequestClass::Visible {
                // The queue is full and this request is not urgent enough
                // to displace anything. The caller is not blocked.
                self.counters.rejected_full += 1;
                return SubmitOutcome::RejectedFull;
            }
            // A visible request displaces the least urgent request in the
            // queue: the tile the user is looking at matters more than the
            // tile we guessed they might want.
            if let Some(victim) = self.least_urgent() {
                self.pending.remove(&victim);
                self.counters.cancelled += 1;
            } else {
                self.counters.rejected_full += 1;
                return SubmitOutcome::RejectedFull;
            }
        }

        self.sequence += 1;
        let request = Request {
            tile: spec.tile,
            class: spec.class,
            generation: self.generation(),
            distance: spec.distance,
            sequence: self.sequence,
        };
        self.pending.insert(spec.tile, request.clone());
        self.push(request);
        self.counters.accepted += 1;
        SubmitOutcome::Accepted
    }

    /// Take the most urgent request, if there is one.
    ///
    /// Requests from an older generation are dropped rather than
    /// dispatched.
    pub fn dispatch(&mut self) -> Option<Request> {
        while let Some(Reverse(entry)) = self.heap.pop() {
            match self.pending.get(&entry.tile) {
                Some(request) if request.sequence == entry.sequence => {
                    let request = request.clone();
                    self.pending.remove(&entry.tile);
                    self.counters.dispatched += 1;
                    return Some(request);
                }
                // A stale heap entry: the request was promoted, cancelled,
                // or already dispatched.
                _ => continue,
            }
        }
        None
    }

    /// The most urgent request, without taking it.
    pub fn peek(&mut self) -> Option<Request> {
        let mut held: Vec<Entry> = Vec::new();
        let mut found = None;
        while let Some(Reverse(entry)) = self.heap.pop() {
            match self.pending.get(&entry.tile) {
                Some(request) if request.sequence == entry.sequence => {
                    found = Some(request.clone());
                    held.push(entry);
                    break;
                }
                // A stale heap entry: the request was promoted, cancelled,
                // or already dispatched.
                _ => continue,
            }
        }
        for entry in held {
            self.heap.push(Reverse(entry));
        }
        found
    }

    /// Advance to a new viewport generation.
    ///
    /// Every queued request from an older generation is dropped: the
    /// viewport they were made for is gone.
    pub fn advance_generation(&mut self) -> u64 {
        let next = self.generation.advance();
        self.cancel_stale();
        next
    }

    /// Drop every queued request from an older generation.
    pub fn cancel_stale(&mut self) {
        let current = self.generation();
        let stale: Vec<usize> = self
            .pending
            .iter()
            .filter(|(_, request)| request.generation < current)
            .map(|(tile, _)| *tile)
            .collect();
        for tile in stale {
            self.pending.remove(&tile);
            self.counters.cancelled += 1;
        }
    }

    /// Count a decoded result that was thrown away because the viewport
    /// moved on while the decode was running.
    pub fn record_discarded_stale(&mut self) {
        self.counters.discarded_stale += 1;
    }

    /// Push a request onto the heap.
    fn push(&mut self, request: Request) {
        self.heap.push(Reverse(Entry {
            class: request.class,
            distance: request.distance,
            sequence: request.sequence,
            tile: request.tile,
        }));
    }

    /// The least urgent queued request: the one a visible request displaces.
    ///
    /// The queue is bounded and small, so a scan is cheaper than keeping a
    /// second structure in step with the first.
    fn least_urgent(&self) -> Option<usize> {
        self.pending
            .iter()
            .max_by_key(|(_, request)| (request.class, request.distance, request.sequence))
            .map(|(tile, _)| *tile)
    }
}
