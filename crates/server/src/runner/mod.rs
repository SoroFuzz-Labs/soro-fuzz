//! The `Runner` extension point: executes one campaign's fuzz budget and
//! reports what happened. `worker::Worker` is the only caller — it never
//! talks to a subprocess/sandbox directly, only through this trait, which
//! is what makes execution swappable and mockable.
//!
//! `subprocess::SubprocessRunner` is the real implementation, wired into
//! `main.rs` as the default: it prepares the `cargo +nightly fuzz run`
//! invocation, runs it through the configured `sandbox::Sandbox`, and
//! parses the result via `output::parse`. `mock::MockRunner` still exists
//! for tests that shouldn't need a toolchain (and would need one — see the
//! README's Windows/toolchain caveats, unchanged by this crate).

pub mod mock;
pub mod output;
pub mod progress;
pub mod sandbox;
pub mod subprocess;

use async_trait::async_trait;
use uuid::Uuid;

use crate::store::FindingStep;
use progress::ProgressPublisher;

/// Everything a `Runner` needs to execute one campaign's budget. Built by
/// the worker from a claimed job's campaign + target.
///
/// `run_id`/`campaign_id`/`invariant_ids` aren't read by any `Runner` impl
/// today: `SubprocessRunner` only needs `fuzz_target_name`/
/// `time_budget_secs` to build its command line. `invariant_ids` in
/// particular is a known gap, not an oversight — `soro-fuzz-core`'s
/// `Harness` has no runtime invariant-selection mechanism today (each
/// `fuzz/fuzz_targets/*.rs` registers a fixed set at compile time via
/// `.with_invariant(..)`), so a campaign's invariant selection is currently
/// enforced only at creation-time validation (`api::campaigns`), not at
/// fuzz-run time. Wiring real runtime selection through would be engine
/// work (`crates/core`), out of scope for this backend.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RunJob {
    pub run_id: Uuid,
    pub campaign_id: Uuid,
    pub fuzz_target_name: String,
    pub invariant_ids: Vec<String>,
    pub time_budget_secs: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Completed,
    Cancelled,
}

/// One reproducible failing case the runner found. Producing
/// `artifact_path`/`shrunk_input` is the runner's job — how a finding gets
/// minimized/located is specific to how it executed (real implementation: a
/// cargo-fuzz crash artifact path; mock: a canned value).
#[derive(Debug, Clone)]
pub struct RunFinding {
    pub invariant: String,
    pub message: String,
    pub step_index: i32,
    pub sequence: Vec<FindingStep>,
    pub shrunk_input: Option<Vec<u8>>,
    pub artifact_path: String,
}

#[derive(Debug, Clone)]
pub struct RunResult {
    pub outcome: RunOutcome,
    pub iterations: i64,
    pub findings: Vec<RunFinding>,
    pub log_tail: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("runner failed: {0}")]
    Failed(String),
}

#[async_trait]
pub trait Runner: Send + Sync {
    async fn run(&self, job: RunJob, progress: ProgressPublisher)
        -> Result<RunResult, RunnerError>;
}
