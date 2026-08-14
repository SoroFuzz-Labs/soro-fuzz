//! `GET /runs/{id}/findings` -> `FindingSummary[]`, `GET /findings/{id}` ->
//! `Finding` (see `docs/api-contract.md`).

use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::error::ApiError;
use super::util::parse_uuid;
use super::AppState;
use crate::store::{FindingRecord, FindingStep};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingSummaryDto {
    pub id: String,
    pub run_id: String,
    pub invariant: String,
    pub message: String,
    pub step_index: i32,
    pub created_at: DateTime<Utc>,
}

impl From<&FindingRecord> for FindingSummaryDto {
    fn from(r: &FindingRecord) -> Self {
        Self {
            id: r.id.to_string(),
            run_id: r.run_id.to_string(),
            invariant: r.invariant.clone(),
            message: r.message.clone(),
            step_index: r.step_index,
            created_at: r.created_at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReproInfoDto {
    pub fuzz_target_name: String,
    pub artifact_path: String,
    pub requires_sanitizer_thread: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingDto {
    pub id: String,
    pub run_id: String,
    pub invariant: String,
    pub message: String,
    pub step_index: i32,
    pub created_at: DateTime<Utc>,
    pub campaign_id: String,
    pub target_id: String,
    pub sequence: Vec<FindingStep>,
    pub repro: ReproInfoDto,
}

impl From<&FindingRecord> for FindingDto {
    fn from(r: &FindingRecord) -> Self {
        Self {
            id: r.id.to_string(),
            run_id: r.run_id.to_string(),
            invariant: r.invariant.clone(),
            message: r.message.clone(),
            step_index: r.step_index,
            created_at: r.created_at,
            campaign_id: r.campaign_id.to_string(),
            target_id: r.target_id.clone(),
            sequence: r.sequence.clone(),
            repro: ReproInfoDto {
                fuzz_target_name: r.fuzz_target_name.clone(),
                artifact_path: r.artifact_path.clone(),
                requires_sanitizer_thread: r.requires_sanitizer_thread,
            },
        }
    }
}

pub async fn list_findings_for_run(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<Vec<FindingSummaryDto>>, ApiError> {
    let run_id = parse_uuid(&run_id)?;
    // 404s if the run itself doesn't exist, so callers can tell "no
    // findings yet" apart from "no such run".
    state
        .store
        .get_run(run_id)
        .await?
        .ok_or_else(|| ApiError::not_found("run_not_found", format!("no run with id {run_id}")))?;

    let findings = state.store.list_findings_for_run(run_id).await?;
    Ok(Json(findings.iter().map(FindingSummaryDto::from).collect()))
}

pub async fn get_finding(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<FindingDto>, ApiError> {
    let id = parse_uuid(&id)?;
    let record = state.store.get_finding(id).await?.ok_or_else(|| {
        ApiError::not_found("finding_not_found", format!("no finding with id {id}"))
    })?;
    Ok(Json(FindingDto::from(&record)))
}
