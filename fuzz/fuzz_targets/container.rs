//! Fuzz the whole-file container parser.
//!
//! Run with:
//!
//! ```text
//! cargo +nightly fuzz run container
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use vpng_fuzzing::targets::container_target;

fuzz_target!(|data: &[u8]| {
    if let Err(finding) = container_target(data) {
        panic!("{finding}");
    }
});
