//! `vpng-fuzzing` — fuzz targets for the VPNG parsers, codecs and editor.
//!
//! # Two ways to run the same targets
//!
//! The target logic lives here, in plain functions that take a byte string
//! and either return [`Ok`] or return a description of the invariant they
//! found broken. The crate offers two drivers:
//!
//! * [`mutate`] — a deterministic, dependency-free mutation harness that
//!   runs on the stable toolchain. It mutates seeds with a seeded PRNG and
//!   runs every target over the result. It needs no nightly compiler and no
//!   `cargo-fuzz`, so it can run in the ordinary CI suite and on any
//!   developer machine.
//! * The `fuzz/` directory holds `cargo-fuzz` targets that call the same
//!   functions. `cargo fuzz run <target>` gives coverage-guided fuzzing on a
//!   machine that has a nightly toolchain and `cargo-fuzz` installed.
//!
//! Both drivers exercise the same code, so a bug found by either is a bug in
//! the library and not in the harness.
//!
//! # What the targets assert
//!
//! A target returns `Err` only when it observes something that must never
//! happen: a parse that accepts bytes it should reject, a round-trip that
//! does not reproduce its input, a decode that produces the wrong number of
//! bytes, a region that does not match the corresponding crop of the full
//! decode. A parse that *rejects* its input is a normal outcome, not a
//! finding: the parsers are supposed to refuse malformed files.

#![forbid(unsafe_code)]

pub mod mutate;
pub mod targets;

pub use mutate::{mutate, MutationReport, TargetOutcome};
pub use targets::{
    container_target, directory_target, editing_target, experimental_codec_target, header_target,
    region_target, tile_payload_target, Target, ALL_TARGETS,
};
