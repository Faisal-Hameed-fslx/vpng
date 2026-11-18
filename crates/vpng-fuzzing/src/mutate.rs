//! The deterministic mutation harness.
//!
//! The harness is a small, self-contained fuzzer: it builds a corpus of
//! seeds, mutates them with a seeded PRNG, and runs every target over the
//! result. It needs no nightly compiler and no `cargo-fuzz`, so it can run
//! in the ordinary CI suite.
//!
//! # Why a deterministic harness as well as `cargo-fuzz`
//!
//! `cargo-fuzz` gives coverage-guided fuzzing, which finds more bugs, but it
//! needs a nightly toolchain and a sanitizer-capable target. The stable
//! mutation harness always runs: in CI on every push, on every platform the
//! project supports, with the same seed producing the same corpus. A
//! regression that the coverage-guided fuzzer found once cannot be
//! reintroduced silently, because the deterministic harness checks the same
//! invariants on every build.
//!
//! # The report
//!
//! [`MutationReport`] records, for every target, how many inputs it saw,
//! how many it accepted, and every finding. A finding is an invariant
//! violation or a panic; the input that caused it is kept so it can be
//! replayed.

use std::panic::{catch_unwind, AssertUnwindSafe};

use vpng_codecs::DecodeContext;
use vpng_core::{Limits, PixelFormat};
use vpng_decoder::Document;
use vpng_encoder::{encode, EncodeOptions};
use vpng_testkit::Rng;

use crate::targets::{fuzz_limits, run_target, Target, ALL_TARGETS};

/// What one target did with the inputs it saw.
#[derive(Debug, Clone, Default)]
pub struct TargetOutcome {
    /// The number of inputs the target saw.
    pub executions: u64,
    /// The number of inputs the target accepted (the count is target
    /// specific: for the container target it is the number of files that
    /// parsed).
    pub accepted: u64,
    /// The number of invariants the target found broken.
    pub findings: Vec<String>,
}

/// The result of a mutation run.
#[derive(Debug, Clone, Default)]
pub struct MutationReport {
    /// The seed the run used.
    pub seed: u64,
    /// The number of inputs the run generated.
    pub iterations: u64,
    /// What each target did.
    pub targets: Vec<(Target, TargetOutcome)>,
}

impl MutationReport {
    /// The number of findings across every target.
    pub fn finding_count(&self) -> usize {
        self.targets
            .iter()
            .map(|(_, outcome)| outcome.findings.len())
            .sum()
    }

    /// Whether the run found anything.
    pub fn is_clean(&self) -> bool {
        self.finding_count() == 0
    }
}

/// How many mutations the harness applies per input.
///
/// Every mutation is applied to a copy of the seed, so the seed corpus stays
/// intact and the run stays reproducible.
#[derive(Debug, Clone, Copy)]
pub struct MutationConfig {
    /// The seed for the PRNG.
    pub seed: u64,
    /// How many inputs to generate.
    pub iterations: u64,
    /// The largest input the harness builds.
    pub max_input_bytes: usize,
}

impl Default for MutationConfig {
    fn default() -> Self {
        MutationConfig {
            seed: 0x5641_4E47_465A_5A00,
            iterations: 20_000,
            max_input_bytes: 64 * 1024,
        }
    }
}

/// The seeds the harness starts from.
///
/// The corpus is built from real VPNG files (so the targets see structurally
/// valid input), from the raw payloads inside them (so the codec targets see
/// realistic compressed data), and from random bytes (so the parsers see
/// nonsense).
fn seed_corpus() -> Vec<Vec<u8>> {
    let mut corpus: Vec<Vec<u8>> = Vec::new();

    let limits = fuzz_limits();
    for (width, height, format, tile) in [
        (64u32, 64u32, PixelFormat::rgba8(), 16u32),
        (100, 60, PixelFormat::rgb8(), 32),
        (40, 40, PixelFormat::gray8(), 16),
        (48, 32, PixelFormat::rgba16(), 16),
    ] {
        let image = vpng_testkit::gradient(width, height, format, 3);
        if let Ok(encoded) = encode(&image, &EncodeOptions::default().with_tile_size(tile, tile)) {
            corpus.push(encoded.bytes.clone());
            // The individual payloads are seeds too: they exercise the codec
            // decoders without the container around them.
            if let Ok(document) = Document::parse(&encoded.bytes, &limits) {
                for index in 0..document.tile_count() as usize {
                    if let Ok(payload) = document.tile_payload(index) {
                        corpus.push(payload.to_vec());
                    }
                }
            }
            // The header and the directory are seeds as well.
            corpus.push(encoded.bytes[..64.min(encoded.bytes.len())].to_vec());
        }
    }

    // Pure noise, so the parsers meet bytes that are not a file at all.
    let mut rng = Rng::new(0x1234_5678_9ABC_DEF0);
    for size in [1usize, 7, 64, 65, 1024, 4096] {
        let mut bytes = vec![0u8; size];
        rng.fill(&mut bytes);
        corpus.push(bytes);
    }

    corpus
}

/// Mutate one seed.
fn mutate_one(seed: &[u8], rng: &mut Rng, max_bytes: usize) -> Vec<u8> {
    if seed.is_empty() {
        return Vec::new();
    }
    let mut data = seed.to_vec();
    let rounds = 1 + rng.next_u64() % 8;
    for _ in 0..rounds {
        match rng.next_u64() % 8 {
            // Flip one bit.
            0 => {
                let index = rng.next_u64() as usize % data.len();
                let bit = rng.next_u64() % 8;
                data[index] ^= 1 << bit;
            }
            // Replace one byte.
            1 => {
                let index = rng.next_u64() as usize % data.len();
                data[index] = rng.next_u8();
            }
            // Truncate.
            2 => {
                let keep = rng.next_u64() as usize % data.len();
                data.truncate(keep);
                if data.is_empty() {
                    data.push(0);
                }
            }
            // Extend with noise.
            3 => {
                let extra = 1 + rng.next_u64() as usize % 64;
                let start = data.len();
                data.resize(start + extra, 0);
                rng.fill(&mut data[start..]);
            }
            // Swap two bytes.
            4 => {
                let a = rng.next_u64() as usize % data.len();
                let b = rng.next_u64() as usize % data.len();
                data.swap(a, b);
            }
            // Write a run of a single byte.
            5 => {
                let value = rng.next_u8();
                let start = rng.next_u64() as usize % data.len();
                let length = 1 + rng.next_u64() as usize % 32;
                for byte in data[start..].iter_mut().take(length) {
                    *byte = value;
                }
            }
            // Set a byte to zero or to all ones.
            6 => {
                let index = rng.next_u64() as usize % data.len();
                data[index] = if rng.next_u64() % 2 == 0 { 0 } else { 0xFF };
            }
            // Splice in a chunk of noise.
            _ => {
                let length = 1 + rng.next_u64() as usize % 16;
                let start = rng.next_u64() as usize % data.len();
                let mut chunk = vec![0u8; length];
                rng.fill(&mut chunk);
                for (offset, byte) in chunk.into_iter().enumerate() {
                    if start + offset < data.len() {
                        data[start + offset] = byte;
                    }
                }
            }
        }
        if data.len() > max_bytes {
            data.truncate(max_bytes);
        }
    }
    data
}

/// Run the mutation harness.
///
/// The harness never panics on a finding: a panic inside a target is caught
/// and recorded as a finding, so the run continues and reports every problem
/// it can reach in one pass.
pub fn mutate(config: MutationConfig) -> MutationReport {
    let corpus = seed_corpus();
    let mut rng = Rng::new(config.seed);
    let mut report = MutationReport {
        seed: config.seed,
        iterations: 0,
        targets: ALL_TARGETS
            .iter()
            .map(|name| (*name, TargetOutcome::default()))
            .collect(),
    };

    for iteration in 0..config.iterations {
        let base = &corpus[(rng.next_u64() as usize) % corpus.len()];
        let input = mutate_one(base, &mut rng, config.max_input_bytes);
        report.iterations += 1;

        for (name, outcome) in report.targets.iter_mut() {
            outcome.executions += 1;
            let result = catch_unwind(AssertUnwindSafe(|| run_target(name, &input)));
            match result {
                Ok(Ok(())) => {}
                Ok(Err(finding)) => {
                    outcome.findings.push(format!(
                        "iteration {iteration}: {finding} [input {} bytes, head {:02X?}]",
                        input.len(),
                        &input[..input.len().min(8)]
                    ));
                }
                Err(payload) => {
                    let message = panic_message(&payload);
                    outcome.findings.push(format!(
                        "iteration {iteration}: panic: {message} [input {} bytes, head {:02X?}]",
                        input.len(),
                        &input[..input.len().min(8)]
                    ));
                }
            }
        }
    }

    report
}

/// The message of a caught panic.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Decode a payload with every stable codec, for the report's codec list.
///
/// The helper is used by the binary to print which codecs were exercised.
pub fn stable_codec_names() -> Vec<&'static str> {
    vpng_codecs::IMPLEMENTED_CODEC_IDS
        .iter()
        .filter_map(|id| vpng_codecs::codec_name(*id))
        .collect()
}

/// The limits the harness runs under.
pub fn harness_limits() -> Limits {
    fuzz_limits()
}

/// A decode context for the report.
pub fn harness_context(format: PixelFormat) -> DecodeContext {
    DecodeContext {
        limits: harness_limits(),
        format,
    }
}
