//! Fuzz the container header parser.
//!
//! Run with:
//!
//! ```text
//! cargo +nightly fuzz run header
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use vpng_fuzzing::targets::header_target;

fuzz_target!(|data: &[u8]| {
    if let Err(finding) = header_target(data) {
        panic!("{finding}");
    }
});
