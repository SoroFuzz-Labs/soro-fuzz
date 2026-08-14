//! `GET /targets`, `GET /targets/{id}` — served straight from the
//! in-memory `TargetRegistry` loaded at startup (see `main.rs`), not a DB
//! round trip: the manifest only changes on a redeploy.
//!
//! This module is the one place that maps the internal `targets::Target`
//! shape onto the wire shape `docs/api-contract.md` defines (snake_case
//! Rust fields -> camelCase JSON, and dropping backend-internal fields like
//! `known_buggy` that the frontend contract doesn't include).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use super::AppState;
use crate::targets::{InvariantInfo, Target, TargetMethod, TargetMethodParam};

#[derive(Serialize)]
pub struct TargetMethodParamDto {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

impl From<&TargetMethodParam> for TargetMethodParamDto {
    fn from(p: &TargetMethodParam) -> Self {
        Self {
            name: p.name.clone(),
            ty: p.ty.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct TargetMethodDto {
    pub name: String,
    pub params: Vec<TargetMethodParamDto>,
}

impl From<&TargetMethod> for TargetMethodDto {
    fn from(m: &TargetMethod) -> Self {
        Self {
            name: m.name.clone(),
            params: m.params.iter().map(Into::into).collect(),
        }
    }
}

#[derive(Serialize)]
pub struct InvariantInfoDto {
    pub id: String,
    pub name: String,
    pub description: String,
}

impl From<&InvariantInfo> for InvariantInfoDto {
    fn from(i: &InvariantInfo) -> Self {
        Self {
            id: i.id.clone(),
            name: i.name.clone(),
            description: i.description.clone(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetDto {
    pub id: String,
    pub name: String,
    pub contract_name: String,
    pub methods: Vec<TargetMethodDto>,
    pub available_invariants: Vec<InvariantInfoDto>,
    pub fuzz_target_name: String,
}

impl From<&Target> for TargetDto {
    fn from(t: &Target) -> Self {
        Self {
            id: t.id.clone(),
            name: t.name.clone(),
            contract_name: t.contract_name.clone(),
            methods: t.methods.iter().map(Into::into).collect(),
            available_invariants: t.available_invariants.iter().map(Into::into).collect(),
            fuzz_target_name: t.fuzz_target_name.clone(),
        }
    }
}

pub async fn list_targets(State(state): State<AppState>) -> Json<Vec<TargetDto>> {
    let targets = state
        .targets
        .list()
        .into_iter()
        .map(TargetDto::from)
        .collect();
    Json(targets)
}

pub async fn get_target(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TargetDto>, StatusCode> {
    state
        .targets
        .get(&id)
        .map(|t| Json(TargetDto::from(t)))
        .ok_or(StatusCode::NOT_FOUND)
}
