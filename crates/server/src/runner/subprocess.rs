//! The real `Runner`: builds and runs
//! `cargo +<toolchain> fuzz run [--sanitizer=<flag>] <target> --
//! -max_total_time=<budget>` through the configured `Sandbox`, then parses
//! its output via `runner::output`.
//!
//! contributors: full command-sequence decoding (`RunFinding::sequence`) is
//! the natural next extension point here — see `runner::output`'s doc
//! comment for why it's out of scope today and how a target-specific
//! decoder subprocess would plug in at the finding-construction site below.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::output;
use super::progress::{LogLevel, ProgressEvent, ProgressPublisher};
use super::sandbox::Sandbox;
use super::{RunFinding, RunJob, RunOutcome, RunResult, Runner, RunnerError};

/// Bytes of stdout+stderr kept as `RunResult::log_tail` — enough to see
/// what happened without storing an unbounded fuzzer log per run.
const LOG_TAIL_CHARS: usize = 4000;

pub struct SubprocessRunner {
    sandbox: Arc<dyn Sandbox>,
    fuzz_workspace_dir: PathBuf,
    rust_toolchain: String,
    fuzz_sanitizer: Option<String>,
    job_timeout: Duration,
}

impl SubprocessRunner {
    pub fn new(
        sandbox: Arc<dyn Sandbox>,
        fuzz_workspace_dir: PathBuf,
        rust_toolchain: String,
        fuzz_sanitizer: Option<String>,
        job_timeout: Duration,
    ) -> Self {
        Self {
            sandbox,
            fuzz_workspace_dir,
            rust_toolchain,
            fuzz_sanitizer,
            job_timeout,
        }
    }

    fn build_args(&self, job: &RunJob) -> Vec<String> {
        let mut args = vec![
            format!("+{}", self.rust_toolchain),
            "fuzz".to_string(),
            "run".to_string(),
        ];
        if let Some(sanitizer) = &self.fuzz_sanitizer {
            args.push(format!("--sanitizer={sanitizer}"));
        }
        args.push(job.fuzz_target_name.clone());
        args.push("--".to_string());
        args.push(format!("-max_total_time={}", job.time_budget_secs));
        args
    }
}

#[async_trait]
impl Runner for SubprocessRunner {
    async fn run(
        &self,
        job: RunJob,
        progress: ProgressPublisher,
    ) -> Result<RunResult, RunnerError> {
        let args = self.build_args(&job);

        progress.emit(ProgressEvent::Log {
            line: format!("cargo {}", args.join(" ")),
            level: LogLevel::Info,
        });

        let output = self
            .sandbox
            .run("cargo", &args, &self.fuzz_workspace_dir, self.job_timeout)
            .await
            .map_err(|e| RunnerError::Failed(e.to_string()))?;

        let combined = format!("{}\n{}", output.stdout, output.stderr);

        if output.timed_out {
            // The sandbox's hard timeout (JOB_TIMEOUT_SECONDS) tripped
            // rather than libFuzzer's own `-max_total_time` — something ran
            // long enough to need killing, which is a runner-level failure,
            // not a contract finding.
            return Err(RunnerError::Failed(format!(
                "sandbox timeout ({:?}) exceeded before `cargo fuzz run` finished; tail: {}",
                self.job_timeout,
                output::tail(&combined, LOG_TAIL_CHARS)
            )));
        }

        let parsed = output::parse(&combined, output.exit_code, output.timed_out);

        progress.emit(ProgressEvent::Iterations {
            count: parsed.iterations.max(0) as u64,
            elapsed_secs: job.time_budget_secs as u64,
        });

        let findings = match parsed.finding {
            Some(violation) => {
                progress.emit(ProgressEvent::Finding {
                    invariant: violation.invariant.clone(),
                    step_index: violation.step_index.max(0) as u32,
                });
                vec![RunFinding {
                    invariant: violation.invariant,
                    message: violation.message,
                    step_index: violation.step_index,
                    // See this module's doc comment: decoding the raw
                    // fuzzer input into a readable step-by-step sequence
                    // needs a target-specific `Arbitrary` decoder this
                    // server deliberately doesn't link. `artifact_path`
                    // below is enough to reproduce the finding locally even
                    // without it (see docs/api-contract.md's `ReproInfo`).
                    sequence: Vec::new(),
                    shrunk_input: None,
                    artifact_path: parsed.artifact_path.unwrap_or_default(),
                }]
            }
            None => Vec::new(),
        };

        Ok(RunResult {
            outcome: RunOutcome::Completed,
            iterations: parsed.iterations,
            findings,
            log_tail: Some(output::tail(&combined, LOG_TAIL_CHARS)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(sanitizer: Option<&str>) -> SubprocessRunner {
        SubprocessRunner::new(
            Arc::new(crate::runner::sandbox::LocalSandbox),
            PathBuf::from("/fake/fuzz"),
            "nightly".to_string(),
            sanitizer.map(str::to_string),
            Duration::from_secs(60),
        )
    }

    fn job() -> RunJob {
        RunJob {
            run_id: uuid::Uuid::new_v4(),
            campaign_id: uuid::Uuid::new_v4(),
            fuzz_target_name: "counter_fuzz".to_string(),
            invariant_ids: vec!["counter-value-matches-model".to_string()],
            time_budget_secs: 30,
        }
    }

    #[test]
    fn builds_the_documented_command_without_a_sanitizer() {
        let args = runner(None).build_args(&job());
        assert_eq!(
            args,
            vec![
                "+nightly",
                "fuzz",
                "run",
                "counter_fuzz",
                "--",
                "-max_total_time=30"
            ]
        );
    }

    #[test]
    fn builds_the_documented_command_with_a_sanitizer() {
        let args = runner(Some("thread")).build_args(&job());
        assert_eq!(
            args,
            vec![
                "+nightly",
                "fuzz",
                "run",
                "--sanitizer=thread",
                "counter_fuzz",
                "--",
                "-max_total_time=30",
            ]
        );
    }
}
