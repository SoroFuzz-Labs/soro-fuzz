//! `GET /runs/{id}`.

use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::error::ApiError;
use super::util::parse_uuid;
use super::AppState;
use crate::store::{RunRecord, RunStatus};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDto {
    pub id: String,
    pub campaign_id: String,
    pub target_id: String,
    pub status: RunStatus,
    pub time_budget_secs: i32,
    pub elapsed_secs: i64,
    pub iterations: i64,
    pub findings_count: i64,
    pub started_at: Option<DateTime<Utc>>,
    /// `docs/api-contract.md` names this `endedAt`, not the auto-camelCased
    /// `finishedAt` the Rust field name would otherwise produce.
    #[serde(rename = "endedAt")]
    pub finished_at: Option<DateTime<Utc>>,
}

impl From<&RunRecord> for RunDto {
    fn from(r: &RunRecord) -> Self {
        Self {
            id: r.id.to_string(),
            campaign_id: r.campaign_id.to_string(),
            target_id: r.target_id.clone(),
            status: r.status,
            time_budget_secs: r.time_budget_secs,
            elapsed_secs: r.elapsed_secs,
            iterations: r.iterations,
            findings_count: r.findings_count,
            started_at: r.started_at,
            finished_at: r.finished_at,
        }
    }
}

pub async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunDto>, ApiError> {
    let id = parse_uuid(&id)?;
    let record = state
        .store
        .get_run(id)
        .await?
        .ok_or_else(|| ApiError::not_found("run_not_found", format!("no run with id {id}")))?;
    Ok(Json(RunDto::from(&record)))
}
