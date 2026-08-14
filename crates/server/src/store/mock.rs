//! An in-memory `Store` for API-layer/worker tests that shouldn't need a
//! live Postgres — `api::campaigns`'s and `worker`'s tests build their
//! `AppState`/`Worker` around this instead of `PgStore`. Test-only: not
//! compiled into the real binary.

use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::model::{
    CampaignFilter, CampaignRecord, CampaignStatus, ClaimedJob, FindingRecord, NewCampaign,
    RunFinish, RunRecord, RunStatus,
};
use super::{Store, StoreError};
use crate::targets::Target;

struct MockRun {
    id: Uuid,
    campaign_id: Uuid,
    status: RunStatus,
    iterations: i64,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    log_tail: Option<String>,
}

struct MockJob {
    id: Uuid,
    campaign_id: Uuid,
    run_id: Uuid,
    claimed_by: Option<String>,
    attempts: i32,
}

#[derive(Default)]
pub struct MockStore {
    campaigns: Mutex<Vec<CampaignRecord>>,
    runs: Mutex<Vec<MockRun>>,
    jobs: Mutex<Vec<MockJob>>,
    findings: Mutex<Vec<FindingRecord>>,
}

impl MockStore {
    fn find_finding_record(
        &self,
        run_id: Uuid,
        id: Uuid,
        invariant: String,
        message: String,
        step_index: i32,
    ) -> FindingRecord {
        let campaigns = self.campaigns.lock().unwrap();
        let campaign = campaigns
            .iter()
            .find(|c| c.run_id == run_id)
            .expect("finding must belong to a run created via create_campaign");
        FindingRecord {
            id,
            run_id,
            campaign_id: campaign.id,
            target_id: campaign.target_id.clone(),
            invariant,
            message,
            step_index,
            sequence: Vec::new(),
            fuzz_target_name: String::new(),
            artifact_path: String::new(),
            requires_sanitizer_thread: false,
            created_at: Utc::now(),
        }
    }
}

#[async_trait]
impl Store for MockStore {
    async fn health_check(&self) -> Result<(), StoreError> {
        Ok(())
    }

    async fn sync_targets(&self, _targets: &[Target]) -> Result<(), StoreError> {
        Ok(())
    }

    async fn create_campaign(&self, input: NewCampaign) -> Result<CampaignRecord, StoreError> {
        let campaign_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();

        let record = CampaignRecord {
            id: campaign_id,
            target_id: input.target_id,
            name: input.name,
            invariant_ids: input.invariant_ids,
            time_budget_secs: input.time_budget_secs,
            status: CampaignStatus::Queued,
            run_id,
            created_at: Utc::now(),
        };
        self.campaigns.lock().unwrap().push(record.clone());

        self.runs.lock().unwrap().push(MockRun {
            id: run_id,
            campaign_id,
            status: RunStatus::Pending,
            iterations: 0,
            started_at: None,
            finished_at: None,
            log_tail: None,
        });

        self.jobs.lock().unwrap().push(MockJob {
            id: Uuid::new_v4(),
            campaign_id,
            run_id,
            claimed_by: None,
            attempts: 0,
        });

        Ok(record)
    }

    async fn list_campaigns(
        &self,
        filter: CampaignFilter,
    ) -> Result<Vec<CampaignRecord>, StoreError> {
        let campaigns = self.campaigns.lock().unwrap();
        let mut matched: Vec<CampaignRecord> = campaigns
            .iter()
            .filter(|c| filter.status.is_none_or(|s| c.status == s))
            .filter(|c| filter.target_id.as_deref().is_none_or(|t| c.target_id == t))
            .filter(|c| filter.since.is_none_or(|since| c.created_at >= since))
            .filter(|c| filter.until.is_none_or(|until| c.created_at <= until))
            .cloned()
            .collect();
        matched.sort_by_key(|c| std::cmp::Reverse(c.created_at));

        let start = (filter.offset.max(0) as usize).min(matched.len());
        let end = start
            .saturating_add(filter.limit.max(0) as usize)
            .min(matched.len());
        Ok(matched[start..end].to_vec())
    }

    async fn get_campaign(&self, id: Uuid) -> Result<Option<CampaignRecord>, StoreError> {
        Ok(self
            .campaigns
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.id == id)
            .cloned())
    }

    async fn cancel_campaign(&self, id: Uuid) -> Result<Option<CampaignRecord>, StoreError> {
        let mut campaigns = self.campaigns.lock().unwrap();
        let Some(campaign) = campaigns.iter_mut().find(|c| c.id == id) else {
            return Ok(None);
        };
        if !campaign.status.is_terminal() {
            campaign.status = CampaignStatus::Cancelled;
            let mut runs = self.runs.lock().unwrap();
            if let Some(run) = runs.iter_mut().find(|r| r.campaign_id == id) {
                run.status = RunStatus::Cancelled;
                run.finished_at = Some(Utc::now());
            }
        }
        Ok(Some(campaign.clone()))
    }

    async fn claim_job(&self, worker_id: &str) -> Result<Option<ClaimedJob>, StoreError> {
        let mut jobs = self.jobs.lock().unwrap();
        let runs = self.runs.lock().unwrap();
        let job = jobs.iter_mut().find(|j| {
            j.claimed_by.is_none()
                && runs
                    .iter()
                    .any(|r| r.id == j.run_id && r.status == RunStatus::Pending)
        });

        Ok(job.map(|job| {
            job.claimed_by = Some(worker_id.to_string());
            job.attempts += 1;
            ClaimedJob {
                job_id: job.id,
                campaign_id: job.campaign_id,
                run_id: job.run_id,
                attempts: job.attempts,
            }
        }))
    }

    async fn start_run(&self, run_id: Uuid, worker_id: &str) -> Result<bool, StoreError> {
        {
            let mut runs = self.runs.lock().unwrap();
            let Some(run) = runs.iter_mut().find(|r| r.id == run_id) else {
                return Ok(false);
            };
            if run.status != RunStatus::Pending {
                return Ok(false);
            }
            run.status = RunStatus::Running;
            run.started_at = Some(Utc::now());
            let _ = worker_id;
        }

        self.findings.lock().unwrap().retain(|f| f.run_id != run_id);

        let mut campaigns = self.campaigns.lock().unwrap();
        if let Some(campaign) = campaigns.iter_mut().find(|c| c.run_id == run_id) {
            if campaign.status == CampaignStatus::Queued {
                campaign.status = CampaignStatus::Running;
            }
        }

        Ok(true)
    }

    async fn finish_run(&self, run_id: Uuid, finish: RunFinish) -> Result<(), StoreError> {
        let transitioned = {
            let mut runs = self.runs.lock().unwrap();
            match runs.iter_mut().find(|r| r.id == run_id) {
                Some(run) if run.status == RunStatus::Running => {
                    run.status = finish.status.run_status();
                    run.iterations = finish.iterations;
                    run.log_tail = finish.log_tail.clone();
                    run.finished_at = Some(Utc::now());
                    true
                }
                _ => false,
            }
        };

        if transitioned {
            let mut campaigns = self.campaigns.lock().unwrap();
            if let Some(campaign) = campaigns.iter_mut().find(|c| c.run_id == run_id) {
                campaign.status = finish.status.campaign_status();
            }
        }

        let mut findings = self.findings.lock().unwrap();
        for new_finding in finish.findings {
            let mut record = self.find_finding_record(
                run_id,
                Uuid::new_v4(),
                new_finding.invariant,
                new_finding.message,
                new_finding.step_index,
            );
            record.sequence = new_finding.sequence;
            record.fuzz_target_name = new_finding.fuzz_target_name;
            record.artifact_path = new_finding.artifact_path;
            record.requires_sanitizer_thread = new_finding.requires_sanitizer_thread;
            findings.push(record);
        }

        Ok(())
    }

    async fn get_run(&self, id: Uuid) -> Result<Option<RunRecord>, StoreError> {
        let runs = self.runs.lock().unwrap();
        let Some(run) = runs.iter().find(|r| r.id == id) else {
            return Ok(None);
        };
        let campaigns = self.campaigns.lock().unwrap();
        let campaign = campaigns
            .iter()
            .find(|c| c.run_id == id)
            .expect("run must belong to a campaign created via create_campaign");
        let findings_count = self
            .findings
            .lock()
            .unwrap()
            .iter()
            .filter(|f| f.run_id == id)
            .count() as i64;

        let elapsed_secs = match run.started_at {
            Some(started) => (run.finished_at.unwrap_or_else(Utc::now) - started)
                .num_seconds()
                .max(0),
            None => 0,
        };

        Ok(Some(RunRecord {
            id: run.id,
            campaign_id: run.campaign_id,
            target_id: campaign.target_id.clone(),
            status: run.status,
            time_budget_secs: campaign.time_budget_secs,
            elapsed_secs,
            iterations: run.iterations,
            findings_count,
            started_at: run.started_at,
            finished_at: run.finished_at,
        }))
    }

    async fn list_findings_for_run(&self, run_id: Uuid) -> Result<Vec<FindingRecord>, StoreError> {
        let mut findings: Vec<FindingRecord> = self
            .findings
            .lock()
            .unwrap()
            .iter()
            .filter(|f| f.run_id == run_id)
            .cloned()
            .collect();
        findings.sort_by_key(|f| f.created_at);
        Ok(findings)
    }

    async fn get_finding(&self, id: Uuid) -> Result<Option<FindingRecord>, StoreError> {
        Ok(self
            .findings
            .lock()
            .unwrap()
            .iter()
            .find(|f| f.id == id)
            .cloned())
    }
}
