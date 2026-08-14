//! Parses `cargo +nightly fuzz run`'s (libFuzzer's) captured stdout/stderr
//! into a structured result. This is the extension point for a different
//! fuzz engine's output shape — contributors: add a new parser module here
//! (or a `FUZZ_ENGINE`-style switch in `subprocess.rs`) rather than
//! stretching this one to understand two formats.
//!
//! What this deliberately does *not* do: decode the raw fuzzer input bytes
//! back into a readable command sequence (`FindingStep[]`). That decoding
//! needs the target contract's own `Arbitrary` impl (`Run<Command>` in
//! `soro-fuzz-core`), which lives in the target's crate — and this server
//! deliberately never links a contract crate (see `crates/server/Cargo.toml`).
//! What it parses instead is the panic text our harness itself produces:
//! `soro_fuzz_core::Violation`'s `Display` impl always reads
//! `` invariant `{name}` violated at step {N}: {message} `` (see
//! `crates/core/src/invariant.rs`), and every finding — whether a specific
//! `Invariant::check` failure or the harness's own unconditional
//! `no-undeclared-panic` guarantee — panics with exactly that text. Matching
//! it here recovers the invariant name, step index, and message without
//! needing to link anything; the one thing it can't recover is the full
//! step-by-step sequence, which is why `RunFinding::sequence` stays empty
//! until a target-specific decoder exists (see `subprocess.rs`'s comment at
//! the finding-construction site).

use std::sync::LazyLock;

use regex::Regex;

static VIOLATION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"invariant `([^`]+)` violated at step (\d+): (.+)").unwrap());
static DONE_RUNS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Done (\d+) runs? in").unwrap());
static COUNTER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^#(\d+)\b").unwrap());
static ARTIFACT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Test unit written to (\S+)").unwrap());

/// A recovered `soro_fuzz_core::Violation`, in the shape `RunFinding` wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedViolation {
    pub invariant: String,
    pub step_index: i32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRun {
    pub iterations: i64,
    pub crashed: bool,
    pub artifact_path: Option<String>,
    pub finding: Option<ParsedViolation>,
}

/// `combined_output` is stdout and stderr concatenated — libFuzzer/Rust's
/// panic machinery split which stream gets what across platforms and
/// versions, so this parser deliberately doesn't care which one it's
/// reading from.
pub fn parse(combined_output: &str, exit_code: Option<i32>, timed_out: bool) -> ParsedRun {
    let iterations = parse_iterations(combined_output);
    let artifact_path = parse_artifact_path(combined_output);
    // A sandbox-level timeout (JOB_TIMEOUT_SECONDS) is the runner's problem,
    // not a contract finding — `subprocess.rs` turns `timed_out` into a
    // `RunnerError` before a finding would ever be built from this.
    let crashed = !timed_out && exit_code.is_some_and(|code| code != 0);

    let finding = if crashed {
        Some(
            parse_violation(combined_output).unwrap_or_else(|| ParsedViolation {
                invariant: "undeclared_panic".to_string(),
                step_index: 0,
                message: tail(combined_output, 2000),
            }),
        )
    } else {
        None
    };

    ParsedRun {
        iterations,
        crashed,
        artifact_path,
        finding,
    }
}

fn parse_iterations(text: &str) -> i64 {
    if let Some(caps) = DONE_RUNS_RE.captures(text) {
        if let Ok(n) = caps[1].parse() {
            return n;
        }
    }
    COUNTER_RE
        .captures_iter(text)
        .filter_map(|c| c[1].parse::<i64>().ok())
        .max()
        .unwrap_or(0)
}

fn parse_artifact_path(text: &str) -> Option<String> {
    ARTIFACT_RE.captures(text).map(|c| c[1].to_string())
}

fn parse_violation(text: &str) -> Option<ParsedViolation> {
    let caps = VIOLATION_RE.captures(text)?;
    Some(ParsedViolation {
        invariant: caps[1].to_string(),
        step_index: caps[2].parse().ok()?,
        message: caps[3].trim().to_string(),
    })
}

/// The last `max_chars` characters of `text`, trimmed — used both as the
/// undeclared-panic fallback message and as `RunResult::log_tail`.
pub fn tail(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.trim().to_string();
    }
    let byte_start = text.len() - max_chars;
    let start = (byte_start..=text.len())
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(text.len());
    text[start..].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN_COMPLETION: &str = r#"
INFO: Seed: 1234567890
INFO: Loaded 1 modules
#2     INITED cov: 12 ft: 12 corp: 1/1b exec/s: 0 rss: 25Mb
#128   NEW    cov: 34 ft: 40 corp: 5/12b exec/s: 0 rss: 26Mb
#4096  pulse  cov: 34 ft: 40 corp: 5/12b exec/s: 4096 rss: 30Mb
Done 100000 runs in 30 second(s)
"#;

    const INVARIANT_VIOLATION: &str = r#"
INFO: Seed: 42
#8192  NEW    cov: 50 ft: 55 corp: 9/40b exec/s: 8192 rss: 32Mb
thread '<unnamed>' panicked at fuzz_targets/counter_fuzz.rs:14:13:
invariant `counter-value-matches-model` violated at step 3: on-chain count 5 != model's expected count 4
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
==12345== ERROR: libFuzzer: deadly signal
    #0 0x55d0a1 in __sanitizer_print_stack_trace
    #1 0x4f2b33 in fuzzer::PrintStackTrace()
artifact_prefix='./'; Test unit written to ./artifacts/counter_fuzz/crash-2916ff5c0e0028e8b5b8f5b90bbe3025cee01e10
"#;

    const UNDECLARED_PANIC: &str = r#"
INFO: Seed: 7
#16    NEW    cov: 10 ft: 10 corp: 2/4b exec/s: 0 rss: 24Mb
thread '<unnamed>' panicked at fuzz_targets/counter_fuzz.rs:14:13:
invariant `no-undeclared-panic` violated at step 0: attempt to add with overflow
==999== ERROR: libFuzzer: deadly signal
artifact_prefix='./'; Test unit written to ./artifacts/counter_fuzz/crash-abcdef0123456789
"#;

    const NON_VIOLATION_CRASH: &str = r#"
INFO: Seed: 1
#3     NEW    cov: 3 ft: 3 corp: 2/2b exec/s: 0 rss: 20Mb
==777== ERROR: libFuzzer: out-of-memory (malloc(4294967296))
   To change the out-of-memory limit use -rss_limit_mb=<N>
artifact_prefix='./'; Test unit written to ./artifacts/counter_fuzz/crash-oom0000000000000
"#;

    #[test]
    fn clean_completion_has_no_finding_and_reads_iterations_from_done_line() {
        let parsed = parse(CLEAN_COMPLETION, Some(0), false);
        assert_eq!(parsed.iterations, 100_000);
        assert!(!parsed.crashed);
        assert!(parsed.finding.is_none());
        assert!(parsed.artifact_path.is_none());
    }

    #[test]
    fn missing_done_line_falls_back_to_the_highest_counter_seen() {
        let text = "#2 INITED\n#128 NEW\n#4096 pulse\n";
        let parsed = parse(text, Some(0), false);
        assert_eq!(parsed.iterations, 4096);
    }

    #[test]
    fn invariant_violation_is_parsed_with_name_step_and_message() {
        let parsed = parse(INVARIANT_VIOLATION, Some(1), false);

        assert!(parsed.crashed);
        let finding = parsed.finding.expect("should have parsed a finding");
        assert_eq!(finding.invariant, "counter-value-matches-model");
        assert_eq!(finding.step_index, 3);
        assert_eq!(
            finding.message,
            "on-chain count 5 != model's expected count 4"
        );

        assert_eq!(
            parsed.artifact_path.as_deref(),
            Some("./artifacts/counter_fuzz/crash-2916ff5c0e0028e8b5b8f5b90bbe3025cee01e10")
        );
    }

    #[test]
    fn undeclared_panic_violation_is_parsed_like_any_other_invariant() {
        let parsed = parse(UNDECLARED_PANIC, Some(1), false);
        let finding = parsed.finding.expect("should have parsed a finding");
        assert_eq!(finding.invariant, "no-undeclared-panic");
        assert_eq!(finding.step_index, 0);
        assert_eq!(finding.message, "attempt to add with overflow");
    }

    #[test]
    fn a_crash_with_no_recognizable_violation_text_falls_back_to_undeclared_panic() {
        let parsed = parse(NON_VIOLATION_CRASH, Some(1), false);
        let finding = parsed.finding.expect("should still record a finding");
        assert_eq!(finding.invariant, "undeclared_panic");
        assert!(
            finding.message.contains("out-of-memory"),
            "message was: {:?}",
            finding.message
        );
        assert!(parsed.artifact_path.is_some());
    }

    #[test]
    fn a_timeout_is_never_treated_as_a_crash_even_with_a_nonzero_exit_code() {
        let parsed = parse(INVARIANT_VIOLATION, Some(1), true);
        assert!(!parsed.crashed);
        assert!(parsed.finding.is_none());
    }

    #[test]
    fn a_missing_exit_code_is_not_treated_as_a_crash() {
        let parsed = parse(CLEAN_COMPLETION, None, false);
        assert!(!parsed.crashed);
    }

    #[test]
    fn tail_returns_the_whole_string_when_shorter_than_the_limit() {
        assert_eq!(tail("short", 100), "short");
    }

    #[test]
    fn tail_truncates_to_a_char_boundary_without_panicking() {
        let text = "a".repeat(50) + "🦀🦀🦀" + &"b".repeat(50);
        let truncated = tail(&text, 10);
        assert!(truncated.len() <= 13); // a couple bytes of slack for the multi-byte crab
        assert!(truncated.ends_with('b'));
    }
}
