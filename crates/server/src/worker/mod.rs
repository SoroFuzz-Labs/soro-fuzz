//! The queue puller: claims `job_queue` rows (via `Store::claim_job`), runs
//! them through a `Runner`, and persists progress + final status. One
//! `Worker` task per concurrent job slot; `spawn_pool` starts
//! `MAX_CONCURRENT_CAMPAIGNS` of them, each polling independently —
//! Postgres's `FOR UPDATE SKIP LOCKED` (see `Store::claim_job`) is what
//! makes concurrent claiming across them (and across separate server
//! processes) safe.
//!
//! contributors: there's no shutdown signal wired into `run_loop` yet — on
//! SIGTERM the HTTP server drains gracefully (see `main.rs`) but a worker
//! mid-job keeps running until that job finishes. Add a cancellation signal
//! here if graceful worker shutdown becomes a requirement; likewise, actually
//! interrupting a job that's already running (rather than just refusing to
//! start one that was cancelled before it was claimed) is build order phase
//! 6's "cancellation" work, not this phase's.

pub mod progress;

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::runner::progress::ProgressEvent;
use crate::runner::{RunFinding, RunJob, RunOutcome, Runner};
use crate::store::{NewFinding, RunFinish, RunFinishStatus, RunStatus, Store};
use crate::targets::TargetRegistry;
use progress::ProgressHub;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Starts `count` workers, each an independent `tokio` task polling
/// `Store::claim_job` in a loop. Returns their `JoinHandle`s so the caller
/// can hold onto them (they otherwise run detached for the life of the
/// process).
pub fn spawn_pool(
    count: usize,
    store: Arc<dyn Store>,
    runner: Arc<dyn Runner>,
    targets: Arc<TargetRegistry>,
    progress: Arc<ProgressHub>,
    fuzz_sanitizer: Option<String>,
) -> Vec<tokio::task::JoinHandle<()>> {
    (0..count.max(1))
        .map(|i| {
            let worker = Worker {
                id: format!("worker-{}-{i}", std::process::id()),
                store: store.clone(),
                runner: runner.clone(),
                targets: targets.clone(),
                progress: progress.clone(),
                fuzz_sanitizer: fuzz_sanitizer.clone(),
            };
            tokio::spawn(worker.run_loop())
        })
        .collect()
}

struct Worker {
    id: String,
    store: Arc<dyn Store>,
    runner: Arc<dyn Runner>,
    targets: Arc<TargetRegistry>,
    progress: Arc<ProgressHub>,
    /// Mirrors `Config::fuzz_sanitizer` — stamped onto every finding this
    /// worker persists (see `docs/api-contract.md`'s
    /// `ReproInfo.requiresSanitizerThread`), since that's a property of how
    /// *this* invocation ran, not of the finding itself.
    fuzz_sanitizer: Option<String>,
}

impl Worker {
    async fn run_loop(self) {
        loop {
            match self.store.claim_job(&self.id).await {
                Ok(Some(job)) => self.process(job.campaign_id, job.run_id).await,
                Ok(None) => tokio::time::sleep(POLL_INTERVAL).await,
                Err(error) => {
                    tracing::error!(worker = %self.id, %error, "failed to claim job");
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
            }
        }
    }

    async fn process(&self, campaign_id: Uuid, run_id: Uuid) {
        let campaign = match self.store.get_campaign(campaign_id).await {
            Ok(Some(campaign)) => campaign,
            Ok(None) => {
                tracing::error!(worker = %self.id, %campaign_id, "claimed job references a missing campaign");
                return;
            }
            Err(error) => {
                tracing::error!(worker = %self.id, %error, "failed to load campaign for claimed job");
                return;
            }
        };

        let Some(target) = self.targets.get(&campaign.target_id) else {
            tracing::error!(worker = %self.id, target_id = %campaign.target_id, "claimed job references an unknown target");
            return;
        };

        match self.store.start_run(run_id, &self.id).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::info!(
                    worker = %self.id, %run_id,
                    "run is no longer pending (likely cancelled before it was claimed); skipping"
                );
                return;
            }
            Err(error) => {
                tracing::error!(worker = %self.id, %error, "failed to start run");
                return;
            }
        }

        let publisher = self.progress.publisher(run_id);
        publisher.emit(ProgressEvent::Status(RunStatus::Running));

        let job = RunJob {
            run_id,
            campaign_id,
            fuzz_target_name: target.fuzz_target_name.clone(),
            invariant_ids: campaign.invariant_ids.clone(),
            time_budget_secs: campaign.time_budget_secs.max(0) as u32,
        };

        let finish = match self.runner.run(job, publisher.clone()).await {
            Ok(result) => {
                let status = match result.outcome {
                    RunOutcome::Completed => RunFinishStatus::Completed,
                    RunOutcome::Cancelled => RunFinishStatus::Cancelled,
                };
                RunFinish {
                    status,
                    iterations: result.iterations,
                    log_tail: result.log_tail,
                    findings: result
                        .findings
                        .into_iter()
                        .map(|f| self.to_new_finding(f, &target.fuzz_target_name))
                        .collect(),
                }
            }
            Err(error) => {
                tracing::error!(worker = %self.id, %run_id, %error, "runner failed");
                RunFinish {
                    status: RunFinishStatus::Failed,
                    iterations: 0,
                    log_tail: Some(error.to_string()),
                    findings: Vec::new(),
                }
            }
        };

        publisher.emit(ProgressEvent::Status(finish.status.run_status()));

        if let Err(error) = self.store.finish_run(run_id, finish).await {
            tracing::error!(worker = %self.id, %run_id, %error, "failed to persist run result");
        }

        self.progress.close(run_id);
    }

    fn to_new_finding(&self, finding: RunFinding, fuzz_target_name: &str) -> NewFinding {
        NewFinding {
            invariant: finding.invariant,
            message: finding.message,
            step_index: finding.step_index,
            sequence: finding.sequence,
            shrunk_input: finding.shrunk_input,
            fuzz_target_name: fuzz_target_name.to_string(),
            artifact_path: finding.artifact_path,
            requires_sanitizer_thread: self.fuzz_sanitizer.as_deref() == Some("thread"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::runner::mock::MockRunner;
    use crate::store::mock::MockStore;
    use crate::store::{CampaignStatus, NewCampaign};

    fn test_worker(store: Arc<dyn Store>, runner: Arc<dyn Runner>) -> Worker {
        let targets_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        Worker {
            id: "test-worker".to_string(),
            store,
            runner,
            targets: Arc::new(
                TargetRegistry::load(&targets_dir).expect("real manifest should load"),
            ),
            progress: Arc::new(ProgressHub::default()),
            fuzz_sanitizer: Some("thread".to_string()),
        }
    }

    async fn new_campaign(store: &Arc<dyn Store>) -> (Uuid, Uuid) {
        let record = store
            .create_campaign(NewCampaign {
                target_id: "counter".to_string(),
                name: "test".to_string(),
                invariant_ids: Vec::new(),
                time_budget_secs: 30,
            })
            .await
            .unwrap();
        (record.id, record.run_id)
    }

    #[tokio::test]
    async fn happy_path_persists_completed_run_and_findings() {
        let store: Arc<dyn Store> = Arc::new(MockStore::default());
        let (campaign_id, run_id) = new_campaign(&store).await;

        let finding = RunFinding {
            invariant: "counter-value-matches-model".to_string(),
            message: "mismatch".to_string(),
            step_index: 2,
            sequence: Vec::new(),
            shrunk_input: None,
            artifact_path: "fuzz/artifacts/counter_fuzz/crash-abc".to_string(),
        };
        let runner: Arc<dyn Runner> = Arc::new(MockRunner::succeeds_with(1234, vec![finding]));
        let worker = test_worker(store.clone(), runner);

        worker.process(campaign_id, run_id).await;

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Completed);
        assert_eq!(run.iterations, 1234);
        assert_eq!(run.findings_count, 1);

        let findings = store.list_findings_for_run(run_id).await.unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].fuzz_target_name, "counter_fuzz");
        assert!(findings[0].requires_sanitizer_thread);

        let campaign = store.get_campaign(campaign_id).await.unwrap().unwrap();
        assert_eq!(campaign.status, CampaignStatus::Completed);
    }

    #[tokio::test]
    async fn runner_failure_marks_run_and_campaign_failed() {
        let store: Arc<dyn Store> = Arc::new(MockStore::default());
        let (campaign_id, run_id) = new_campaign(&store).await;

        let runner: Arc<dyn Runner> = Arc::new(MockRunner::always_fails("boom"));
        let worker = test_worker(store.clone(), runner);

        worker.process(campaign_id, run_id).await;

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Failed);

        let campaign = store.get_campaign(campaign_id).await.unwrap().unwrap();
        assert_eq!(campaign.status, CampaignStatus::Failed);
    }

    #[tokio::test]
    async fn runner_cancellation_marks_run_and_campaign_cancelled() {
        let store: Arc<dyn Store> = Arc::new(MockStore::default());
        let (campaign_id, run_id) = new_campaign(&store).await;

        let runner: Arc<dyn Runner> = Arc::new(MockRunner::cancelled_after(5));
        let worker = test_worker(store.clone(), runner);

        worker.process(campaign_id, run_id).await;

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Cancelled);
        assert_eq!(run.iterations, 5);

        let campaign = store.get_campaign(campaign_id).await.unwrap().unwrap();
        assert_eq!(campaign.status, CampaignStatus::Cancelled);
    }

    #[tokio::test]
    async fn process_skips_a_run_cancelled_before_it_was_claimed() {
        let store: Arc<dyn Store> = Arc::new(MockStore::default());
        let (campaign_id, run_id) = new_campaign(&store).await;

        store.cancel_campaign(campaign_id).await.unwrap();

        // A sentinel: if `process` ever actually invoked the runner despite
        // the cancellation, the run would come out `Failed`, not the
        // untouched `Cancelled` we assert below.
        let runner: Arc<dyn Runner> = Arc::new(MockRunner::always_fails("should never run"));
        let worker = test_worker(store.clone(), runner);

        worker.process(campaign_id, run_id).await;

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(
            run.status,
            RunStatus::Cancelled,
            "process() must not have called the runner"
        );
    }

    #[tokio::test]
    async fn claim_process_pipeline_end_to_end() {
        let store: Arc<dyn Store> = Arc::new(MockStore::default());
        let (_campaign_id, run_id) = new_campaign(&store).await;

        let runner: Arc<dyn Runner> = Arc::new(MockRunner::always_succeeds());
        let worker = test_worker(store.clone(), runner);

        let claimed = store
            .claim_job("test-worker")
            .await
            .unwrap()
            .expect("job should be claimable");
        assert_eq!(claimed.run_id, run_id);

        worker.process(claimed.campaign_id, claimed.run_id).await;

        assert!(
            store.claim_job("test-worker").await.unwrap().is_none(),
            "an already-claimed job must not be reclaimed"
        );
        assert_eq!(
            store.get_run(run_id).await.unwrap().unwrap().status,
            RunStatus::Completed
        );
    }
}
