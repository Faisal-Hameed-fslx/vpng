//! Fuzz the experimental codec decoders.
//!
//! Run with:
//!
//! ```text
//! cargo +nightly fuzz run experimental_codec
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use vpng_fuzzing::targets::experimental_codec_target;

fuzz_target!(|data: &[u8]| {
    if let Err(finding) = experimental_codec_target(data) {
        panic!("{finding}");
    }
});
