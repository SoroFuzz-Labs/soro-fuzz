//! The axum layer: thin handlers over `Store`/`Runner` query logic, wired
//! into one `Router` in `router`. Each resource gets its own module as it's
//! built out (health, targets, campaigns, runs, findings, SSE stream now —
//! see the build order in the README).

mod campaigns;
mod error;
mod findings;
mod health;
mod runs;
mod stream;
mod targets;
mod util;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use crate::store::Store;
use crate::targets::TargetRegistry;
use crate::worker::progress::ProgressHub;

/// Shared handler state. Cheap to `Clone`: every field is an `Arc` clone or
/// a plain integer, never a deep copy.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub targets: Arc<TargetRegistry>,
    /// The same `ProgressHub` the worker pool publishes into (see
    /// `main.rs`) — `api::stream` subscribes to it per run.
    pub progress: Arc<ProgressHub>,
    /// Upper bound for `CreateCampaignRequest.timeBudgetSecs` — mirrors
    /// `Config::job_timeout`, since a budget beyond the sandbox's hard kill
    /// timeout would just get truncated anyway (see `api::campaigns`).
    pub max_time_budget_secs: i64,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/targets", get(targets::list_targets))
        .route("/targets/{id}", get(targets::get_target))
        .route(
            "/campaigns",
            post(campaigns::create_campaign).get(campaigns::list_campaigns),
        )
        .route("/campaigns/{id}", get(campaigns::get_campaign))
        .route("/campaigns/{id}/cancel", post(campaigns::cancel_campaign))
        .route("/runs/{id}", get(runs::get_run))
        .route("/runs/{id}/findings", get(findings::list_findings_for_run))
        .route("/runs/{id}/stream", get(stream::stream_run))
        .route("/findings/{id}", get(findings::get_finding))
        .with_state(state)
}
