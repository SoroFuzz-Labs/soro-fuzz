//! The persistence extension point.
//!
//! Everything the API/worker need to read and write is expressed as a
//! `Store` method so both layers stay testable against `mock::MockStore`
//! instead of a live Postgres. This trait grows one cohesive slice at a time
//! as the build order lands rather than all at once:
//!
//! - targets sync + lookup (build order phase 2, done)
//! - campaign create/list/get/cancel (phase 3, done)
//! - run + job_queue claim/update, finding insert (phase 4, done)
//!
//! contributors: add new methods here as a feature needs them, and grow
//! `mock::MockStore` to match so `api::*`'s handler tests keep working
//! without Postgres.

pub mod model;
pub mod postgres;

#[cfg(test)]
pub(crate) mod mock;

pub use model::{
    CampaignFilter, CampaignRecord, CampaignStatus, ClaimedJob, FindingRecord, FindingStep,
    NewCampaign, NewFinding, RunFinish, RunFinishStatus, RunRecord, RunStatus,
};
pub use postgres::PgStore;

use async_trait::async_trait;
use uuid::Uuid;

use crate::targets::Target;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

#[async_trait]
pub trait Store: Send + Sync {
    /// Cheap connectivity probe backing `GET /health`.
    async fn health_check(&self) -> Result<(), StoreError>;

    /// Upserts every target from a freshly loaded `TargetRegistry` into the
    /// `targets` table, called once at startup. This is what makes
    /// `campaigns.target_id`'s foreign key enforceable — a campaign can only
    /// ever reference a target that made it through manifest validation.
    /// Doesn't delete rows for targets removed from the manifest (that would
    /// conflict with existing campaigns' FK); stale rows are a documented
    /// follow-up, not silently dropped history.
    async fn sync_targets(&self, targets: &[Target]) -> Result<(), StoreError>;

    /// Creates a campaign, its one `run` (status `pending`), and a
    /// `job_queue` entry referencing both, in a single transaction — the
    /// "POST /campaigns enqueues a job and returns status=queued
    /// immediately" behavior, and why `CampaignRecord.run_id` is never
    /// optional even though nothing has executed yet. The worker (build
    /// order phase 4) is what claims the job_queue row and starts advancing
    /// the run's status.
    async fn create_campaign(&self, input: NewCampaign) -> Result<CampaignRecord, StoreError>;

    /// Filtered, paginated campaign list, newest first.
    async fn list_campaigns(
        &self,
        filter: CampaignFilter,
    ) -> Result<Vec<CampaignRecord>, StoreError>;

    async fn get_campaign(&self, id: Uuid) -> Result<Option<CampaignRecord>, StoreError>;

    /// Marks a campaign and its run cancelled, unless the campaign is
    /// already in a terminal state (in which case this is a no-op that
    /// returns the current record — cancelling twice isn't an error).
    /// Returns `None` if the campaign doesn't exist.
    async fn cancel_campaign(&self, id: Uuid) -> Result<Option<CampaignRecord>, StoreError>;

    /// Atomically claims the oldest unclaimed `job_queue` row whose run is
    /// still `pending` (a run cancelled before any worker claimed it is
    /// simply never matched here — its job_queue row is left unclaimed
    /// rather than deleted, a documented follow-up like `sync_targets`'s).
    /// Safe to call concurrently from multiple workers/processes: the
    /// Postgres impl uses `FOR UPDATE SKIP LOCKED` so two workers can never
    /// claim the same row.
    async fn claim_job(&self, worker_id: &str) -> Result<Option<ClaimedJob>, StoreError>;

    /// Transitions a claimed run from `pending` to `running` (mirroring the
    /// campaign to `running` too) and clears any findings left over from a
    /// previous attempt at this run — the idempotency guarantee for a
    /// reclaimed/retried job. Returns `false` (doing nothing) if the run
    /// isn't `pending` anymore, e.g. it was cancelled between being claimed
    /// and the worker calling this — the worker should skip running the job
    /// in that case rather than overwrite a terminal state.
    async fn start_run(&self, run_id: Uuid, worker_id: &str) -> Result<bool, StoreError>;

    /// Closes out a run: persists its findings unconditionally (whatever
    /// was found before completion/cancellation/failure is real and worth
    /// keeping), but only transitions the run/campaign status if the run is
    /// still `running` — if a concurrent `cancel_campaign` already moved it
    /// to a terminal state, this is a no-op on status so the worker can
    /// never clobber a cancellation that raced it.
    async fn finish_run(&self, run_id: Uuid, finish: RunFinish) -> Result<(), StoreError>;

    /// The detail behind `GET /runs/{id}`.
    async fn get_run(&self, id: Uuid) -> Result<Option<RunRecord>, StoreError>;

    /// The detail behind `GET /runs/{id}/findings`, oldest first.
    async fn list_findings_for_run(&self, run_id: Uuid) -> Result<Vec<FindingRecord>, StoreError>;

    /// The detail behind `GET /findings/{id}`.
    async fn get_finding(&self, id: Uuid) -> Result<Option<FindingRecord>, StoreError>;
}
