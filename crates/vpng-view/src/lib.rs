//! `vpng-view` — the interactive viewer.
//!
//! A viewer of a very large image must not decode the whole image, and it
//! must not decode the wrong tiles either. This crate is the engine that
//! decides *which* tiles to decode and *when*: a bounded scheduler with
//! viewport priority, a parallel decoder that works through that schedule,
//! and a predictive prefetch that tries to have the next tile ready before
//! the user asks for it.
//!
//! # The parts
//!
//! * [`Scheduler`] — the bounded request queue. It knows the difference
//!   between a tile the user is looking at, a tile we think they will want
//!   next, and a tile that can wait; it drops the requests that a pan made
//!   worthless; and it never queues the same tile twice.
//! * [`Driver`] — the scheduler with a decoder attached. It is the
//!   single-threaded reference implementation: the order in which tiles
//!   should be decoded, made explicit.
//! * [`Pool`] — the parallel engine. A fixed set of worker threads take
//!   requests from the scheduler, decode them with their own reader, and
//!   publish the results into the bounded cache. The lock is never held
//!   across a decode, and a result that lands after the viewport has moved
//!   on is discarded rather than shown.
//!
//! # What it does not change
//!
//! The binary format is untouched. The viewer reads the same container the
//! decoder reads, through [`vpng_source::TileReader`], and decodes tiles
//! through the same codecs. A tile decoded here is byte-for-byte the tile
//! the ordinary decoder produces — the parallel engine changes *when* a
//! tile is decoded, never *what* it decodes.
//!
//! # Layers
//!
//! A container written for format **v0.4** may hold **layers**: named grids
//! of tiles, each with a rectangle on the canvas, an opacity and — when the
//! format has an 8-bit alpha channel — a mask.
//!
//! The viewer schedules and decodes the **base layer**. It does not
//! composite: the layers are drawn by the caller. The viewer's own geometry
//! is the base layer's partition, so a viewport rectangle is a rectangle on
//! the canvas, and a layer that is smaller than the canvas simply does not
//! cover it.

#![forbid(unsafe_code)]

mod driver;
mod file_decoder;
mod pool;
#[cfg(test)]
mod pool_tests;
mod scheduler;
#[cfg(test)]
mod tests;
mod viewport;
#[cfg(test)]
mod viewport_tests;

pub use driver::{DecodedTile, Driver, Step, TileDecoder};
pub use file_decoder::{FileDecoder, FileDecoderFactory, SourceDecoder, SourceDecoderFactory};
pub use pool::{DecoderFactory, Pool, PoolOutcome, PoolStats};
pub use scheduler::{
    Counters, Generation, Request, RequestClass, RequestSpec, Scheduler, SubmitOutcome,
};
pub use viewport::{Direction, PlanEntry, PlanReport, PrefetchPolicy, ViewPlan, Viewport};
