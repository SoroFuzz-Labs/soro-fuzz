//! A `Runner` that reports a canned result instantly instead of actually
//! compiling/running anything. `main.rs` now defaults to
//! `subprocess::SubprocessRunner`; this stays around as the `Runner` the
//! worker's own tests (`worker::tests`) run against, and as a drop-in for
//! running this server without a nightly/cargo-fuzz toolchain installed.

use async_trait::async_trait;

use super::progress::{ProgressEvent, ProgressPublisher};
use super::{RunFinding, RunJob, RunOutcome, RunResult, Runner, RunnerError};
use crate::store::RunStatus;

// `Cancel`/`Fail` and the constructors that produce them are only reached
// from `worker`'s tests today (the real binary only ever calls
// `always_succeeds`) — real, deliberate test surface, not leftover code.
#[allow(dead_code)]
#[derive(Clone)]
enum Behavior {
    Succeed {
        iterations: i64,
        findings: Vec<RunFinding>,
    },
    Cancel {
        iterations: i64,
    },
    Fail(String),
}

/// contributors: this always finishes instantly with a fixed result — it
/// exists to prove the worker/store plumbing end to end, not to simulate
/// fuzzing. Don't add timing/randomness here; that's what the real runner
/// (phase 5) and its own tests are for.
#[derive(Clone)]
pub struct MockRunner {
    behavior: Behavior,
}

impl MockRunner {
    #[allow(dead_code)] // exercised by worker::tests; SubprocessRunner is main.rs's default now
    pub fn always_succeeds() -> Self {
        Self {
            behavior: Behavior::Succeed {
                iterations: 0,
                findings: Vec::new(),
            },
        }
    }

    #[allow(dead_code)] // exercised by worker::tests
    pub fn succeeds_with(iterations: i64, findings: Vec<RunFinding>) -> Self {
        Self {
            behavior: Behavior::Succeed {
                iterations,
                findings,
            },
        }
    }

    #[allow(dead_code)] // exercised by worker::tests
    pub fn always_fails(message: impl Into<String>) -> Self {
        Self {
            behavior: Behavior::Fail(message.into()),
        }
    }

    #[allow(dead_code)] // exercised by worker::tests
    pub fn cancelled_after(iterations: i64) -> Self {
        Self {
            behavior: Behavior::Cancel { iterations },
        }
    }
}

#[async_trait]
impl Runner for MockRunner {
    async fn run(
        &self,
        _job: RunJob,
        progress: ProgressPublisher,
    ) -> Result<RunResult, RunnerError> {
        progress.emit(ProgressEvent::Status(RunStatus::Running));

        match &self.behavior {
            Behavior::Succeed {
                iterations,
                findings,
            } => {
                progress.emit(ProgressEvent::Iterations {
                    count: (*iterations).max(0) as u64,
                    elapsed_secs: 0,
                });
                for finding in findings {
                    progress.emit(ProgressEvent::Finding {
                        invariant: finding.invariant.clone(),
                        step_index: finding.step_index.max(0) as u32,
                    });
                }
                Ok(RunResult {
                    outcome: RunOutcome::Completed,
                    iterations: *iterations,
                    findings: findings.clone(),
                    log_tail: Some("mock runner: completed".to_string()),
                })
            }
            Behavior::Cancel { iterations } => Ok(RunResult {
                outcome: RunOutcome::Cancelled,
                iterations: *iterations,
                findings: Vec::new(),
                log_tail: Some("mock runner: cancelled".to_string()),
            }),
            Behavior::Fail(message) => Err(RunnerError::Failed(message.clone())),
        }
    }
}
