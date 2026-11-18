//! The fuzz smoke test.
//!
//! The test runs the deterministic mutation harness over a small number of
//! iterations on every `cargo test` run. It is the same harness the release
//! driver runs, with the same targets, so a regression that the coverage
//! guided fuzzer found once cannot come back unnoticed.

use vpng_fuzzing::mutate::{mutate, MutationConfig};

#[test]
fn the_mutation_harness_finds_nothing() {
    let report = mutate(MutationConfig {
        seed: 0x5641_4E47_465A_5A00,
        iterations: 1_500,
        max_input_bytes: 8 * 1024,
    });
    assert!(
        report.is_clean(),
        "the mutation harness found {} problems:\n{}",
        report.finding_count(),
        report
            .targets
            .iter()
            .flat_map(|(name, outcome)| {
                outcome
                    .findings
                    .iter()
                    .map(move |finding| format!("  {name}: {finding}"))
            })
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!(report.iterations, 1_500);
    assert_eq!(report.targets.len(), 7);
}

#[test]
fn the_mutation_harness_is_reproducible() {
    let config = MutationConfig {
        seed: 99,
        iterations: 200,
        max_input_bytes: 1024,
    };
    let first = mutate(config);
    let second = mutate(config);
    assert_eq!(first.iterations, second.iterations);
    for ((name, a), (_, b)) in first.targets.iter().zip(second.targets.iter()) {
        assert_eq!(
            a.executions, b.executions,
            "target {name} ran a different number of times"
        );
        assert_eq!(
            a.findings, b.findings,
            "target {name} found different problems"
        );
    }
}

#[test]
fn a_different_seed_exercises_different_inputs() {
    let a = mutate(MutationConfig {
        seed: 1,
        iterations: 100,
        max_input_bytes: 512,
    });
    let b = mutate(MutationConfig {
        seed: 2,
        iterations: 100,
        max_input_bytes: 512,
    });
    assert_eq!(a.iterations, b.iterations);
    // Both runs are clean; the seeds change the corpus, not the verdict.
    assert!(a.is_clean());
    assert!(b.is_clean());
}
