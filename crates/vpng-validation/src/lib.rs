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
//! * [`encode_bench`] — the parallel-encoder benchmark scenarios: the speed
//!   of each worker count, and the SHA-256 that proves every worker count
//!   wrote the same file.
//! * [`photo_bench`] — the multi-run photographic benchmark: a latency
//!   matrix over the tile size and the worker count, every case repeated,
//!   with the median and the p95.
//! * [`preview_bench`] — the preview-pyramid benchmark: what a pyramid buys
//!   (the first preview) and what it costs (encode time, file size, and the
//!   memory it needs to see the whole picture).
//! * [`json`] — a minimal JSON writer for the reports.
//!
//! The six binaries are:
//!
//! * `vpng-memory` — one validation case per process, so the process's
//!   resident-set high-water mark belongs to that case alone. A driver
//!   script runs the cases and aggregates their reports.
//! * `vpng-editing-bench` — the editing scenarios, in one report.
//! * `vpng-view-bench` — the viewing scenarios, in one report.
//! * `vpng-encode-bench` — the parallel-encoder scenarios, in one report.
//! * `vpng-photo-bench` — the multi-run photographic latency matrix, in one
//!   report.
//! * `vpng-preview-bench` — the preview-pyramid scenarios, in one report.

#![deny(missing_docs)]

pub mod bench;
pub mod encode_bench;
pub mod json;
pub mod mem;
pub mod photo_bench;
pub mod preview_bench;
pub mod stream;
pub mod view_bench;
