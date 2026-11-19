//! `vpng-validation` — the memory validation and editing benchmarks.
//!
//! The crate provides:
//!
//! * [`mem`] — two independent memory measurements: the allocator's own
//!   high-water mark and the operating system's resident-set figures.
//! * [`stream`] — a tile-by-tile writer that produces an ordinary VPNG file
//!   without ever holding the whole image, so a 16K image can be written on
//!   a machine that could not hold it in memory.
//! * [`bench`] — the editing-engine benchmark scenarios.
//! * [`view_bench`] — the interactive-viewing benchmark scenarios: the
//!   first viewport, the pan, the prefetch, and the localized edit, each
//!   measured against the sequential baseline in the same run.
//! * [`json`] — a minimal JSON writer for the reports.
//!
//! The three binaries are:
//!
//! * `vpng-memory` — one validation case per process, so the process's
//!   resident-set high-water mark belongs to that case alone. A driver
//!   script runs the cases and aggregates their reports.
//! * `vpng-editing-bench` — the editing scenarios, in one report.
//! * `vpng-view-bench` — the viewing scenarios, in one report.

#![deny(missing_docs)]

pub mod bench;
pub mod json;
pub mod mem;
pub mod stream;
pub mod view_bench;
