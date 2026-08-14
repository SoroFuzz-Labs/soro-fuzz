use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use super::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    status: &'static str,
    database: ComponentStatus,
    toolchain: ComponentStatus,
}

#[derive(Serialize)]
pub struct ComponentStatus {
    ok: bool,
    detail: Option<String>,
}

impl ComponentStatus {
    fn ok() -> Self {
        Self {
            ok: true,
            detail: None,
        }
    }

    fn error(detail: impl ToString) -> Self {
        Self {
            ok: false,
            detail: Some(detail.to_string()),
        }
    }

    // contributors: once runner::sandbox lands (build order phase 5), probe
    // the configured Sandbox (e.g. `docker exec <toolchain> cargo --version`)
    // here instead of always reporting unknown.
    fn unknown() -> Self {
        Self {
            ok: false,
            detail: Some("toolchain probing not yet implemented".to_string()),
        }
    }
}

pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let database = match state.store.health_check().await {
        Ok(()) => ComponentStatus::ok(),
        Err(e) => ComponentStatus::error(e),
    };

    let status_code = if database.ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let body = HealthResponse {
        status: if database.ok { "ok" } else { "degraded" },
        database,
        toolchain: ComponentStatus::unknown(),
    };

    (status_code, Json(body))
}
