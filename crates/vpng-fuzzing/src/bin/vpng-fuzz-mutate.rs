//! The deterministic fuzzing driver.
//!
//! The binary runs the mutation harness over every target and writes a JSON
//! report. It exits with a non-zero status when the harness finds anything,
//! so CI fails on a regression.
//!
//! ```text
//! cargo run --release -p vpng-fuzzing --bin vpng-fuzz-mutate -- --iterations 50000
//! ```

use std::io::Write;
use std::process::ExitCode;

use vpng_fuzzing::mutate::{MutationConfig, MutationReport};
use vpng_fuzzing::targets::ALL_TARGETS;

fn main() -> ExitCode {
    let mut config = MutationConfig::default();
    let mut output: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--iterations" => {
                let value = args.next().unwrap_or_default();
                match value.parse::<u64>() {
                    Ok(value) => config.iterations = value,
                    Err(_) => {
                        eprintln!("--iterations needs a number");
                        return ExitCode::from(2);
                    }
                }
            }
            "--seed" => {
                let value = args.next().unwrap_or_default();
                match value.parse::<u64>() {
                    Ok(value) => config.seed = value,
                    Err(_) => {
                        eprintln!("--seed needs a number");
                        return ExitCode::from(2);
                    }
                }
            }
            "--output" => output = args.next(),
            "--help" | "-h" => {
                println!("usage: vpng-fuzz-mutate [--iterations N] [--seed N] [--output FILE]");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument {other}");
                return ExitCode::from(2);
            }
        }
    }

    let started = std::time::Instant::now();
    let report = vpng_fuzzing::mutate::mutate(config);
    let elapsed = started.elapsed();

    let document = render(&report, elapsed);
    match &output {
        Some(path) => {
            if let Err(error) = std::fs::write(path, &document) {
                eprintln!("could not write {path}: {error}");
                return ExitCode::from(2);
            }
            println!("wrote {path}");
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(document.as_bytes());
        }
    }

    if report.is_clean() {
        println!(
            "clean: {} iterations over {} targets in {:.1}s",
            report.iterations,
            report.targets.len(),
            elapsed.as_secs_f64()
        );
        ExitCode::SUCCESS
    } else {
        eprintln!("findings: {}", report.finding_count());
        ExitCode::FAILURE
    }
}

/// Render the report as JSON.
fn render(report: &MutationReport, elapsed: std::time::Duration) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"tool\": \"vpng-fuzz-mutate\",\n");
    out.push_str("  \"schema\": 1,\n");
    out.push_str(&format!("  \"seed\": {},\n", report.seed));
    out.push_str(&format!("  \"iterations\": {},\n", report.iterations));
    out.push_str(&format!("  \"elapsed_ms\": {},\n", elapsed.as_millis()));
    out.push_str(&format!(
        "  \"targets\": [\"{}\"],\n",
        ALL_TARGETS.join("\", \"")
    ));
    out.push_str("  \"per_target\": [\n");
    for (index, (name, outcome)) in report.targets.iter().enumerate() {
        out.push_str(&format!(
            "    {{\"name\": \"{name}\", \"executions\": {}, \"findings\": {}}}",
            outcome.executions,
            outcome.findings.len()
        ));
        if index + 1 < report.targets.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ],\n");
    out.push_str(&format!(
        "  \"finding_count\": {},\n",
        report.finding_count()
    ));
    out.push_str("  \"findings\": [");
    let mut first = true;
    for (name, outcome) in &report.targets {
        for finding in &outcome.findings {
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str(&format!(
                "\n    {{\"target\": \"{name}\", \"finding\": {}}}",
                json_string(finding)
            ));
        }
    }
    if !first {
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    out
}

/// Escape a string for JSON.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
