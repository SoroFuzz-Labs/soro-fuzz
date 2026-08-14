//! Small helpers shared across handler modules.

use uuid::Uuid;

use super::error::ApiError;

pub fn parse_uuid(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw)
        .map_err(|_| ApiError::bad_request("invalid_id", format!("`{raw}` is not a valid id")))
}
