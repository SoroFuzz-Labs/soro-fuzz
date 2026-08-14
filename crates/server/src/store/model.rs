//! Record/input types the `Store` trait's methods pass around, kept out of
//! `postgres.rs` so they don't drag sqlx specifics into callers, and so a
//! future mock store (or a different backend) can implement `Store` against
//! the same shapes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Mirrors Postgres's `campaign_status` enum (see migration 0002) and
/// `docs/api-contract.md`'s `CampaignStatus`. Note the initial state is
/// `queued` — `RunStatus` below uses `pending` for the same moment; they're
/// deliberately different vocabularies, not a typo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, Deserialize)]
#[sqlx(type_name = "campaign_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum CampaignStatus {
    Queued,
    Running,
    Completed,
    Cancelled,
    Failed,
}

impl CampaignStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

#[derive(Debug, Clone)]
pub struct NewCampaign {
    pub target_id: String,
    pub name: String,
    pub invariant_ids: Vec<String>,
    pub time_budget_secs: i32,
}

#[derive(Debug, Clone)]
pub struct CampaignRecord {
    pub id: Uuid,
    pub target_id: String,
    pub name: String,
    pub invariant_ids: Vec<String>,
    pub time_budget_secs: i32,
    pub status: CampaignStatus,
    /// The one run this campaign started at creation time (see
    /// `Store::create_campaign`) — always present, never a second one.
    pub run_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct CampaignFilter {
    pub status: Option<CampaignStatus>,
    pub target_id: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub limit: i64,
    pub offset: i64,
}

/// Mirrors Postgres's `run_status` enum (migration 0003) and
/// `docs/api-contract.md`'s `RunStatus`. Initial state `pending` — see the
/// note on `CampaignStatus` for why that's not `queued` too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, Deserialize)]
#[sqlx(type_name = "run_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Pending,
    Running,
    Completed,
    Cancelled,
    Failed,
}

/// A job_queue row claimed by a worker — everything `Worker::process` needs
/// to look up the campaign and start the run.
#[derive(Debug, Clone)]
pub struct ClaimedJob {
    #[allow(dead_code)] // not read yet; kept for a future retry/requeue path
    pub job_id: Uuid,
    pub campaign_id: Uuid,
    pub run_id: Uuid,
    #[allow(dead_code)] // surfaced once the worker logs/reports retry counts
    pub attempts: i32,
}

/// The queryable detail behind `GET /runs/{id}` — mirrors
/// `docs/api-contract.md`'s `Run`. `elapsed_secs`/`findings_count` are
/// computed by the query, not stored columns.
#[derive(Debug, Clone)]
pub struct RunRecord {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub target_id: String,
    pub status: RunStatus,
    pub time_budget_secs: i32,
    pub elapsed_secs: i64,
    pub iterations: i64,
    pub findings_count: i64,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// One step of a finding's reproduction sequence — mirrors
/// `docs/api-contract.md`'s `FindingStep` exactly, since this is stored
/// as-is in `findings.sequence` (jsonb) and served as-is by
/// `GET /findings/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingStep {
    pub index: u32,
    pub method: String,
    pub args: Vec<FindingStepArg>,
    pub authorized_by: Vec<String>,
    pub advance_time_secs: Option<u32>,
    pub outcome: FindingStepOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingStepArg {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FindingStepOutcome {
    Ok,
    DeclaredError { code: u32 },
    Rejected { detail: String },
    UndeclaredPanic { detail: String },
}

/// What `Store::finish_run` needs to persist a finding — matches
/// `findings`' columns except `run_id` (an argument to `finish_run`, not
/// repeated per finding) and the joined `campaign_id`/`target_id` `GET
/// /findings/{id}` adds back on read (see `FindingRecord`).
#[derive(Debug, Clone)]
pub struct NewFinding {
    pub invariant: String,
    pub message: String,
    pub step_index: i32,
    pub sequence: Vec<FindingStep>,
    pub shrunk_input: Option<Vec<u8>>,
    pub fuzz_target_name: String,
    pub artifact_path: String,
    pub requires_sanitizer_thread: bool,
}

/// The queryable detail behind `GET /findings/{id}` /
/// `GET /runs/{id}/findings` — mirrors `docs/api-contract.md`'s `Finding`.
#[derive(Debug, Clone)]
pub struct FindingRecord {
    pub id: Uuid,
    pub run_id: Uuid,
    pub campaign_id: Uuid,
    pub target_id: String,
    pub invariant: String,
    pub message: String,
    pub step_index: i32,
    pub sequence: Vec<FindingStep>,
    pub fuzz_target_name: String,
    pub artifact_path: String,
    pub requires_sanitizer_thread: bool,
    pub created_at: DateTime<Utc>,
}

/// The terminal state a `Runner::run` call ends in, from the worker's point
/// of view — `Failed` covers both a `RunnerError` and (until phase 5 gives
/// the runner a real crash/timeout parser) any execution the runner itself
/// couldn't complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunFinishStatus {
    Completed,
    Cancelled,
    Failed,
}

impl RunFinishStatus {
    pub fn run_status(self) -> RunStatus {
        match self {
            Self::Completed => RunStatus::Completed,
            Self::Cancelled => RunStatus::Cancelled,
            Self::Failed => RunStatus::Failed,
        }
    }

    pub fn campaign_status(self) -> CampaignStatus {
        match self {
            Self::Completed => CampaignStatus::Completed,
            Self::Cancelled => CampaignStatus::Cancelled,
            Self::Failed => CampaignStatus::Failed,
        }
    }
}

/// Everything `Store::finish_run` needs to close out a run in one call,
/// whether the runner succeeded, was cancelled, or errored.
#[derive(Debug, Clone)]
pub struct RunFinish {
    pub status: RunFinishStatus,
    pub iterations: i64,
    pub log_tail: Option<String>,
    pub findings: Vec<NewFinding>,
}
