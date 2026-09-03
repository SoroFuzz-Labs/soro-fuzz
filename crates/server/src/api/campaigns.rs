//! `POST /campaigns`, `GET /campaigns`, `GET /campaigns/{id}`,
//! `POST /campaigns/{id}/cancel`.
//!
//! Validation lives here, not in the store: a target id that isn't in the
//! `TargetRegistry`, an invariant id the target doesn't offer, or a time
//! budget outside sane bounds should never reach a query — this is the one
//! place that enforces "a campaign references a known target, never
//! anything else" at the HTTP boundary.

use std::collections::HashSet;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::error::ApiError;
use super::util::parse_uuid;
use super::AppState;
use crate::store::{CampaignFilter, CampaignRecord, CampaignStatus, NewCampaign};

/// Default/max page size for `GET /campaigns`. Not part of
/// `docs/api-contract.md` (which documents no pagination params), but
/// accepting optional `limit`/`offset` query params is purely additive —
/// a frontend that never sends them gets `DEFAULT_LIST_LIMIT` rows, newest
/// first, same as today.
const DEFAULT_LIST_LIMIT: i64 = 50;
const MAX_LIST_LIMIT: i64 = 200;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCampaignRequest {
    pub target_id: String,
    pub name: String,
    #[serde(default)]
    pub invariant_ids: Vec<String>,
    pub time_budget_secs: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignDto {
    pub id: String,
    pub target_id: String,
    pub name: String,
    pub invariant_ids: Vec<String>,
    pub time_budget_secs: i32,
    pub status: CampaignStatus,
    pub run_id: String,
    pub created_at: DateTime<Utc>,
}

impl From<&CampaignRecord> for CampaignDto {
    fn from(r: &CampaignRecord) -> Self {
        Self {
            id: r.id.to_string(),
            target_id: r.target_id.clone(),
            name: r.name.clone(),
            invariant_ids: r.invariant_ids.clone(),
            time_budget_secs: r.time_budget_secs,
            status: r.status,
            run_id: r.run_id.to_string(),
            created_at: r.created_at,
        }
    }
}

pub async fn create_campaign(
    State(state): State<AppState>,
    Json(body): Json<CreateCampaignRequest>,
) -> Result<(StatusCode, Json<CampaignDto>), ApiError> {
    let target = state.targets.get(&body.target_id).ok_or_else(|| {
        ApiError::bad_request(
            "unknown_target",
            format!("no target with id `{}`", body.target_id),
        )
    })?;

    if body.name.trim().is_empty() {
        return Err(ApiError::bad_request(
            "invalid_name",
            "name must not be empty",
        ));
    }

    let available: HashSet<&str> = target
        .available_invariants
        .iter()
        .map(|i| i.id.as_str())
        .collect();
    for invariant_id in &body.invariant_ids {
        if !available.contains(invariant_id.as_str()) {
            return Err(ApiError::bad_request(
                "unknown_invariant",
                format!(
                    "target `{}` has no invariant `{invariant_id}`",
                    body.target_id
                ),
            ));
        }
    }

    if body.time_budget_secs < 1 || body.time_budget_secs > state.max_time_budget_secs {
        return Err(ApiError::bad_request(
            "invalid_time_budget",
            format!(
                "timeBudgetSecs must be between 1 and {} (the configured JOB_TIMEOUT_SECONDS)",
                state.max_time_budget_secs
            ),
        ));
    }

    let record = state
        .store
        .create_campaign(NewCampaign {
            target_id: body.target_id,
            name: body.name,
            invariant_ids: body.invariant_ids,
            time_budget_secs: body.time_budget_secs as i32,
        })
        .await?;

    Ok((StatusCode::CREATED, Json(CampaignDto::from(&record))))
}

#[derive(Deserialize)]
pub struct ListCampaignsQuery {
    pub status: Option<String>,
    #[serde(rename = "targetId")]
    pub target_id: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list_campaigns(
    State(state): State<AppState>,
    Query(query): Query<ListCampaignsQuery>,
) -> Result<Json<Vec<CampaignDto>>, ApiError> {
    let status = query
        .status
        .as_deref()
        .map(parse_campaign_status)
        .transpose()?;
    let since = query.since.as_deref().map(parse_datetime).transpose()?;
    let until = query.until.as_deref().map(parse_datetime).transpose()?;

    let limit = query
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);
    let offset = query.offset.unwrap_or(0).max(0);

    let filter = CampaignFilter {
        status,
        target_id: query.target_id,
        since,
        until,
        limit,
        offset,
    };

    let records = state.store.list_campaigns(filter).await?;
    Ok(Json(records.iter().map(CampaignDto::from).collect()))
}

pub async fn get_campaign(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<CampaignDto>, ApiError> {
    let id = parse_uuid(&id)?;
    let record = state.store.get_campaign(id).await?.ok_or_else(|| {
        ApiError::not_found("campaign_not_found", format!("no campaign with id {id}"))
    })?;
    Ok(Json(CampaignDto::from(&record)))
}

pub async fn cancel_campaign(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<CampaignDto>, ApiError> {
    let id = parse_uuid(&id)?;
    let record = state.store.cancel_campaign(id).await?.ok_or_else(|| {
        ApiError::not_found("campaign_not_found", format!("no campaign with id {id}"))
    })?;
    Ok(Json(CampaignDto::from(&record)))
}

fn parse_campaign_status(raw: &str) -> Result<CampaignStatus, ApiError> {
    match raw {
        "queued" => Ok(CampaignStatus::Queued),
        "running" => Ok(CampaignStatus::Running),
        "completed" => Ok(CampaignStatus::Completed),
        "cancelled" => Ok(CampaignStatus::Cancelled),
        "failed" => Ok(CampaignStatus::Failed),
        other => Err(ApiError::bad_request(
            "invalid_status",
            format!("`{other}` is not a valid status"),
        )),
    }
}

fn parse_datetime(raw: &str) -> Result<DateTime<Utc>, ApiError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| {
            ApiError::bad_request(
                "invalid_date",
                format!("`{raw}` is not a valid ISO 8601 timestamp"),
            )
        })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::AppState;
    use crate::api::router;
    use crate::store::mock::MockStore;
    use crate::targets::TargetRegistry;
    use crate::worker::progress::ProgressHub;

    fn test_state() -> AppState {
        let targets_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        AppState {
            store: Arc::new(MockStore::default()),
            targets: Arc::new(
                TargetRegistry::load(&targets_dir).expect("real manifest should load"),
            ),
            progress: Arc::new(ProgressHub::default()),
            max_time_budget_secs: 600,
        }
    }

    fn create_body(
        target_id: &str,
        name: &str,
        invariant_ids: &[&str],
        time_budget_secs: i64,
    ) -> Body {
        Body::from(
            serde_json::json!({
                "targetId": target_id,
                "name": name,
                "invariantIds": invariant_ids,
                "timeBudgetSecs": time_budget_secs,
            })
            .to_string(),
        )
    }

    async fn json_body(response: axum::response::Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn create_campaign_rejects_unknown_target() {
        let app = router(test_state());
        let response = app
            .oneshot(
                Request::post("/campaigns")
                    .header("content-type", "application/json")
                    .body(create_body("nope", "x", &[], 10))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_campaign_rejects_unknown_invariant() {
        let app = router(test_state());
        let response = app
            .oneshot(
                Request::post("/campaigns")
                    .header("content-type", "application/json")
                    .body(create_body("counter", "x", &["not-a-real-invariant"], 10))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_campaign_rejects_time_budget_out_of_range() {
        let app = router(test_state());

        let too_low = app
            .clone()
            .oneshot(
                Request::post("/campaigns")
                    .header("content-type", "application/json")
                    .body(create_body("counter", "x", &[], 0))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(too_low.status(), StatusCode::BAD_REQUEST);

        let too_high = app
            .oneshot(
                Request::post("/campaigns")
                    .header("content-type", "application/json")
                    .body(create_body("counter", "x", &[], 10_000))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(too_high.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_then_get_round_trips() {
        let app = router(test_state());

        let created = app
            .clone()
            .oneshot(
                Request::post("/campaigns")
                    .header("content-type", "application/json")
                    .body(create_body(
                        "counter",
                        "smoke test",
                        &["counter-value-matches-model"],
                        30,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let created_body = json_body(created).await;
        assert_eq!(created_body["status"], "queued");
        assert_eq!(created_body["targetId"], "counter");
        assert!(created_body["runId"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));

        let id = created_body["id"].as_str().unwrap();
        let fetched = app
            .oneshot(
                Request::get(format!("/campaigns/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(fetched.status(), StatusCode::OK);
        let fetched_body = json_body(fetched).await;
        assert_eq!(fetched_body["id"], created_body["id"]);
        assert_eq!(fetched_body["name"], "smoke test");
    }

    #[tokio::test]
    async fn get_missing_campaign_returns_404() {
        let app = router(test_state());
        let response = app
            .oneshot(
                Request::get(format!("/campaigns/{}", uuid::Uuid::new_v4()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_campaign_rejects_malformed_id() {
        let app = router(test_state());
        let response = app
            .oneshot(
                Request::get("/campaigns/not-a-uuid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn cancel_is_idempotent() {
        let app = router(test_state());

        let created = app
            .clone()
            .oneshot(
                Request::post("/campaigns")
                    .header("content-type", "application/json")
                    .body(create_body("counter", "cancel me", &[], 30))
                    .unwrap(),
            )
            .await
            .unwrap();
        let id = json_body(created).await["id"].as_str().unwrap().to_string();

        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    Request::post(format!("/campaigns/{id}/cancel"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = json_body(response).await;
            assert_eq!(body["status"], "cancelled");
        }
    }

    #[tokio::test]
    async fn list_filters_by_target_id() {
        let app = router(test_state());

        for (target, name) in [("counter", "a"), ("token", "b")] {
            app.clone()
                .oneshot(
                    Request::post("/campaigns")
                        .header("content-type", "application/json")
                        .body(create_body(target, name, &[], 30))
                        .unwrap(),
                )
                .await
                .unwrap();
        }

        let response = app
            .oneshot(
                Request::get("/campaigns?targetId=token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        let campaigns = body.as_array().unwrap();
        assert_eq!(campaigns.len(), 1);
        assert_eq!(campaigns[0]["targetId"], "token");
    }
}
