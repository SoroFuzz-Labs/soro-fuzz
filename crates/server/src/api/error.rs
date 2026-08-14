//! The `ApiError` shape every handler that can fail returns, matching
//! `docs/api-contract.md`'s `ApiError` (`{ error, message }`) so the
//! frontend gets one consistent error body regardless of which handler
//! failed or why.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use crate::store::StoreError;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    error: &'static str,
    message: String,
}

#[derive(Serialize)]
struct ApiErrorBody {
    error: &'static str,
    message: String,
}

impl ApiError {
    pub fn bad_request(error: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error,
            message: message.into(),
        }
    }

    pub fn not_found(error: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            error,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: "internal_error",
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ApiErrorBody {
            error: self.error,
            message: self.message,
        };
        (self.status, Json(body)).into_response()
    }
}

/// Store failures are never the caller's fault and never described to them
/// in detail — the real error goes to the logs, the client gets a generic
/// 500. contributors: if a future `StoreError` variant *should* surface as
/// a 4xx (e.g. a uniqueness violation the caller could fix), match it here
/// explicitly rather than adding special cases at call sites.
impl From<StoreError> for ApiError {
    fn from(err: StoreError) -> Self {
        tracing::error!(error = %err, "store error");
        ApiError::internal("an internal error occurred")
    }
}
