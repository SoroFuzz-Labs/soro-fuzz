//! The real `Store` impl, backed by Postgres via sqlx.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::types::Json;
use uuid::Uuid;

use super::model::{
    CampaignFilter, CampaignRecord, CampaignStatus, ClaimedJob, FindingRecord, FindingStep,
    NewCampaign, RunFinish, RunRecord, RunStatus,
};
use super::{Store, StoreError};
use crate::targets::Target;

/// A `campaigns` row joined to its one `run`'s id — the shape every
/// campaign read query selects, so `CampaignRecord::run_id` is always
/// populated straight from the query rather than a follow-up lookup.
#[derive(sqlx::FromRow)]
struct CampaignWithRunRow {
    id: Uuid,
    target_id: String,
    name: String,
    invariant_ids: Json<Vec<String>>,
    time_budget_secs: i32,
    status: CampaignStatus,
    created_at: DateTime<Utc>,
    run_id: Uuid,
}

impl From<CampaignWithRunRow> for CampaignRecord {
    fn from(row: CampaignWithRunRow) -> Self {
        Self {
            id: row.id,
            target_id: row.target_id,
            name: row.name,
            invariant_ids: row.invariant_ids.0,
            time_budget_secs: row.time_budget_secs,
            status: row.status,
            run_id: row.run_id,
            created_at: row.created_at,
        }
    }
}

const SELECT_CAMPAIGN_WITH_RUN: &str = r#"
    SELECT c.id, c.target_id, c.name, c.invariant_ids, c.time_budget_secs, c.status, c.created_at, r.id AS run_id
    FROM campaigns c
    JOIN runs r ON r.campaign_id = c.id
"#;

/// One `runs` row joined to its campaign's `target_id`/`time_budget_secs`
/// plus the two computed columns `GET /runs/{id}` needs — see
/// `docs/api-contract.md`'s `Run`.
#[derive(sqlx::FromRow)]
struct RunRow {
    id: Uuid,
    campaign_id: Uuid,
    target_id: String,
    status: RunStatus,
    time_budget_secs: i32,
    elapsed_secs: i64,
    iterations: i64,
    findings_count: i64,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}

impl From<RunRow> for RunRecord {
    fn from(row: RunRow) -> Self {
        Self {
            id: row.id,
            campaign_id: row.campaign_id,
            target_id: row.target_id,
            status: row.status,
            time_budget_secs: row.time_budget_secs,
            elapsed_secs: row.elapsed_secs,
            iterations: row.iterations,
            findings_count: row.findings_count,
            started_at: row.started_at,
            finished_at: row.finished_at,
        }
    }
}

const SELECT_RUN: &str = r#"
    SELECT r.id, r.campaign_id, c.target_id, r.status, c.time_budget_secs,
           COALESCE(EXTRACT(EPOCH FROM (COALESCE(r.finished_at, now()) - r.started_at)), 0)::bigint AS elapsed_secs,
           r.iterations,
           (SELECT COUNT(*) FROM findings f WHERE f.run_id = r.id) AS findings_count,
           r.started_at, r.finished_at
    FROM runs r
    JOIN campaigns c ON c.id = r.campaign_id
"#;

/// A `findings` row joined back to its `campaign_id`/`target_id` via `runs`
/// -> `campaigns` — see `docs/api-contract.md`'s `Finding`.
#[derive(sqlx::FromRow)]
struct FindingRow {
    id: Uuid,
    run_id: Uuid,
    campaign_id: Uuid,
    target_id: String,
    invariant: String,
    message: String,
    step_index: i32,
    sequence: Json<Vec<FindingStep>>,
    fuzz_target_name: String,
    artifact_path: String,
    requires_sanitizer_thread: bool,
    created_at: DateTime<Utc>,
}

impl From<FindingRow> for FindingRecord {
    fn from(row: FindingRow) -> Self {
        Self {
            id: row.id,
            run_id: row.run_id,
            campaign_id: row.campaign_id,
            target_id: row.target_id,
            invariant: row.invariant,
            message: row.message,
            step_index: row.step_index,
            sequence: row.sequence.0,
            fuzz_target_name: row.fuzz_target_name,
            artifact_path: row.artifact_path,
            requires_sanitizer_thread: row.requires_sanitizer_thread,
            created_at: row.created_at,
        }
    }
}

const SELECT_FINDING: &str = r#"
    SELECT f.id, f.run_id, r.campaign_id, c.target_id, f.invariant, f.message, f.step_index,
           f.sequence, f.fuzz_target_name, f.artifact_path, f.requires_sanitizer_thread, f.created_at
    FROM findings f
    JOIN runs r ON r.id = f.run_id
    JOIN campaigns c ON c.id = r.campaign_id
"#;

pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    pub async fn connect(database_url: &str) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    /// Runs every pending migration under `./migrations`, embedded at
    /// compile time by `sqlx::migrate!` (no live DB needed to *build* the
    /// server, only to run it).
    pub async fn migrate(&self) -> Result<(), StoreError> {
        sqlx::migrate!().run(&self.pool).await?;
        Ok(())
    }
}

#[async_trait]
impl Store for PgStore {
    async fn health_check(&self) -> Result<(), StoreError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    async fn sync_targets(&self, targets: &[Target]) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        for target in targets {
            sqlx::query(
                r#"
                INSERT INTO targets (id, name, contract_name, methods, available_invariants, fuzz_target_name, known_buggy)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT (id) DO UPDATE SET
                    name = EXCLUDED.name,
                    contract_name = EXCLUDED.contract_name,
                    methods = EXCLUDED.methods,
                    available_invariants = EXCLUDED.available_invariants,
                    fuzz_target_name = EXCLUDED.fuzz_target_name,
                    known_buggy = EXCLUDED.known_buggy
                "#,
            )
            .bind(&target.id)
            .bind(&target.name)
            .bind(&target.contract_name)
            .bind(Json(&target.methods))
            .bind(Json(&target.available_invariants))
            .bind(&target.fuzz_target_name)
            .bind(target.known_buggy)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn create_campaign(&self, input: NewCampaign) -> Result<CampaignRecord, StoreError> {
        let mut tx = self.pool.begin().await?;

        let campaign_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO campaigns (target_id, name, invariant_ids, time_budget_secs)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
        )
        .bind(&input.target_id)
        .bind(&input.name)
        .bind(Json(&input.invariant_ids))
        .bind(input.time_budget_secs)
        .fetch_one(&mut *tx)
        .await?;

        let run_id: Uuid =
            sqlx::query_scalar("INSERT INTO runs (campaign_id) VALUES ($1) RETURNING id")
                .bind(campaign_id)
                .fetch_one(&mut *tx)
                .await?;

        sqlx::query("INSERT INTO job_queue (campaign_id, run_id) VALUES ($1, $2)")
            .bind(campaign_id)
            .bind(run_id)
            .execute(&mut *tx)
            .await?;

        let row: CampaignWithRunRow =
            sqlx::query_as(&format!("{SELECT_CAMPAIGN_WITH_RUN} WHERE c.id = $1"))
                .bind(campaign_id)
                .fetch_one(&mut *tx)
                .await?;

        tx.commit().await?;
        Ok(row.into())
    }

    async fn list_campaigns(
        &self,
        filter: CampaignFilter,
    ) -> Result<Vec<CampaignRecord>, StoreError> {
        let rows: Vec<CampaignWithRunRow> = sqlx::query_as(&format!(
            r#"
            {SELECT_CAMPAIGN_WITH_RUN}
            WHERE ($1::campaign_status IS NULL OR c.status = $1)
              AND ($2::text IS NULL OR c.target_id = $2)
              AND ($3::timestamptz IS NULL OR c.created_at >= $3)
              AND ($4::timestamptz IS NULL OR c.created_at <= $4)
            ORDER BY c.created_at DESC
            LIMIT $5 OFFSET $6
            "#
        ))
        .bind(filter.status)
        .bind(&filter.target_id)
        .bind(filter.since)
        .bind(filter.until)
        .bind(filter.limit)
        .bind(filter.offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get_campaign(&self, id: Uuid) -> Result<Option<CampaignRecord>, StoreError> {
        let row: Option<CampaignWithRunRow> =
            sqlx::query_as(&format!("{SELECT_CAMPAIGN_WITH_RUN} WHERE c.id = $1"))
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.map(Into::into))
    }

    async fn cancel_campaign(&self, id: Uuid) -> Result<Option<CampaignRecord>, StoreError> {
        let mut tx = self.pool.begin().await?;

        let current: Option<CampaignWithRunRow> =
            sqlx::query_as(&format!("{SELECT_CAMPAIGN_WITH_RUN} WHERE c.id = $1"))
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;

        let Some(mut current) = current else {
            return Ok(None);
        };

        if current.status.is_terminal() {
            tx.commit().await?;
            return Ok(Some(current.into()));
        }

        sqlx::query("UPDATE campaigns SET status = 'cancelled', updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE runs SET status = 'cancelled', finished_at = now() WHERE campaign_id = $1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        current.status = CampaignStatus::Cancelled;
        Ok(Some(current.into()))
    }

    async fn claim_job(&self, worker_id: &str) -> Result<Option<ClaimedJob>, StoreError> {
        let claimed: Option<(Uuid, Uuid, Uuid, i32)> = sqlx::query_as(
            r#"
            WITH claimable AS (
                SELECT jq.id
                FROM job_queue jq
                JOIN runs r ON r.id = jq.run_id
                WHERE jq.claimed_by IS NULL AND r.status = 'pending'
                ORDER BY jq.created_at
                LIMIT 1
                FOR UPDATE OF jq SKIP LOCKED
            )
            UPDATE job_queue
            SET claimed_by = $1, claimed_at = now(), attempts = attempts + 1
            FROM claimable
            WHERE job_queue.id = claimable.id
            RETURNING job_queue.id, job_queue.campaign_id, job_queue.run_id, job_queue.attempts
            "#,
        )
        .bind(worker_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(
            claimed.map(|(job_id, campaign_id, run_id, attempts)| ClaimedJob {
                job_id,
                campaign_id,
                run_id,
                attempts,
            }),
        )
    }

    async fn start_run(&self, run_id: Uuid, worker_id: &str) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin().await?;

        let campaign_id: Option<Uuid> = sqlx::query_scalar(
            r#"
            UPDATE runs SET status = 'running', started_at = now(), worker = $2
            WHERE id = $1 AND status = 'pending'
            RETURNING campaign_id
            "#,
        )
        .bind(run_id)
        .bind(worker_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(campaign_id) = campaign_id else {
            tx.commit().await?;
            return Ok(false);
        };

        // Idempotency for a reclaimed/retried job: whatever a previous,
        // interrupted attempt at this run found is stale once we're
        // starting it again.
        sqlx::query("DELETE FROM findings WHERE run_id = $1")
            .bind(run_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("UPDATE campaigns SET status = 'running', updated_at = now() WHERE id = $1 AND status = 'queued'")
            .bind(campaign_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(true)
    }

    async fn finish_run(&self, run_id: Uuid, finish: RunFinish) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;

        let campaign_id: Option<Uuid> = sqlx::query_scalar(
            r#"
            UPDATE runs SET status = $2, iterations = $3, log_tail = $4, finished_at = now()
            WHERE id = $1 AND status = 'running'
            RETURNING campaign_id
            "#,
        )
        .bind(run_id)
        .bind(finish.status.run_status())
        .bind(finish.iterations)
        .bind(&finish.log_tail)
        .fetch_optional(&mut *tx)
        .await?;

        // Only mirror the status if we actually transitioned the run above
        // — if a concurrent cancel already moved it to a terminal state,
        // leave that alone rather than clobbering it with our own outcome.
        if let Some(campaign_id) = campaign_id {
            sqlx::query("UPDATE campaigns SET status = $2, updated_at = now() WHERE id = $1")
                .bind(campaign_id)
                .bind(finish.status.campaign_status())
                .execute(&mut *tx)
                .await?;
        }

        for finding in &finish.findings {
            sqlx::query(
                r#"
                INSERT INTO findings
                    (run_id, invariant, message, step_index, sequence, shrunk_input, fuzz_target_name, artifact_path, requires_sanitizer_thread)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                "#,
            )
            .bind(run_id)
            .bind(&finding.invariant)
            .bind(&finding.message)
            .bind(finding.step_index)
            .bind(Json(&finding.sequence))
            .bind(&finding.shrunk_input)
            .bind(&finding.fuzz_target_name)
            .bind(&finding.artifact_path)
            .bind(finding.requires_sanitizer_thread)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn get_run(&self, id: Uuid) -> Result<Option<RunRecord>, StoreError> {
        let row: Option<RunRow> = sqlx::query_as(&format!("{SELECT_RUN} WHERE r.id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(Into::into))
    }

    async fn list_findings_for_run(&self, run_id: Uuid) -> Result<Vec<FindingRecord>, StoreError> {
        let rows: Vec<FindingRow> = sqlx::query_as(&format!(
            "{SELECT_FINDING} WHERE f.run_id = $1 ORDER BY f.created_at"
        ))
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get_finding(&self, id: Uuid) -> Result<Option<FindingRecord>, StoreError> {
        let row: Option<FindingRow> = sqlx::query_as(&format!("{SELECT_FINDING} WHERE f.id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(Into::into))
    }
}
