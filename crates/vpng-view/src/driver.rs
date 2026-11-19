//! The single-threaded driver.
//!
//! The driver is the scheduler with a decoder attached: it takes the most
//! urgent request and decodes it. It is the reference implementation of the
//! engine's loop — the one the parallel pool is measured against, and the
//! one that says what "the right order" means.
//!
//! It is deliberately single-threaded. A parallel engine must produce the
//! same pixels in a different order, not different pixels, and the only
//! way to be sure of that is to have a version whose order is fixed and
//! obvious.

use vpng_core::Result;

use crate::scheduler::{Request, Scheduler};

/// What the driver decodes with.
///
/// The driver does not know where the bytes come from: a file on disk, a
/// buffer in memory, or a stub in a test.
pub trait TileDecoder {
    /// Decode one tile into pixels.
    fn decode(&mut self, tile: usize) -> Result<Vec<u8>>;

    /// The number of bytes this decoder has read so far.
    ///
    /// The default is zero, for a decoder that reads from somewhere other
    /// than a file. A file-backed decoder reports its reader's count, so
    /// the engine can state the I/O volume it caused rather than estimate
    /// it from the tile count.
    fn bytes_read(&self) -> u64 {
        0
    }
}

/// A decoded tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedTile {
    /// The tile's index in the directory.
    pub tile: usize,
    /// The tile's pixels.
    pub pixels: Vec<u8>,
}

/// What one step of the driver did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The queue was empty; there was nothing to do.
    Idle,
    /// A tile was decoded, and the viewport still wants it.
    Decoded(DecodedTile),
    /// A tile was decoded, but the viewport moved on while it was being
    /// decoded, so the result was thrown away.
    ///
    /// **This is not an error.** The user panned away; the work was simply
    /// no longer wanted. The driver counts it separately from a failure, so
    /// a report cannot mistake "the user panned away" for "the decoder is
    /// broken".
    Discarded,
}

/// The scheduler with a decoder attached.
#[derive(Debug)]
pub struct Driver<D> {
    scheduler: Scheduler,
    decoder: D,
}

impl<D: TileDecoder> Driver<D> {
    /// Build a driver.
    pub fn new(scheduler: Scheduler, decoder: D) -> Self {
        Driver { scheduler, decoder }
    }

    /// The scheduler.
    pub fn scheduler(&self) -> &Scheduler {
        &self.scheduler
    }

    /// The scheduler, mutably.
    pub fn scheduler_mut(&mut self) -> &mut Scheduler {
        &mut self.scheduler
    }

    /// The decoder.
    pub fn decoder(&self) -> &D {
        &self.decoder
    }

    /// Decode one request.
    pub fn step(&mut self) -> Result<Step> {
        let Some(request) = self.scheduler.dispatch() else {
            return Ok(Step::Idle);
        };

        let pixels = self.decoder.decode(request.tile)?;

        if !self.scheduler.is_current(request.generation) {
            // The viewport moved on while this tile was being decoded. The
            // work is not wasted — the pixels were produced — but nobody
            // wants them, so they are dropped.
            self.scheduler.record_discarded_stale();
            return Ok(Step::Discarded);
        }

        Ok(Step::Decoded(DecodedTile {
            tile: request.tile,
            pixels,
        }))
    }

    /// Decode everything in the queue, in priority order.
    pub fn drain(&mut self) -> Result<Vec<DecodedTile>> {
        let mut decoded = Vec::new();
        loop {
            match self.step()? {
                Step::Idle => break,
                Step::Decoded(tile) => decoded.push(tile),
                Step::Discarded => {}
            }
        }
        Ok(decoded)
    }

    /// The request the driver would dispatch next, without dispatching it.
    pub fn peek(&mut self) -> Option<Request> {
        self.scheduler.peek()
    }
}
