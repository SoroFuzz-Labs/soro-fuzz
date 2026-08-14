//! Loads and validates the target manifest (`{TARGETS_DIR}/targets.json`,
//! see `examples/targets.json`) — the fixed list of contracts this backend
//! knows how to fuzz.
//!
//! This is the enforcement point for "a campaign references a known target
//! id, never an uploaded blob": `TargetRegistry` is built once at startup
//! from a file a maintainer hand-edits (or a future sync job regenerates)
//! and campaign creation (build order phase 3) can only ever look a target
//! id up in it — there is no code path from an HTTP request to a target
//! that isn't in this list.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetMethodParam {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetMethod {
    pub name: String,
    #[serde(default)]
    pub params: Vec<TargetMethodParam>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub contract_name: String,
    #[serde(default)]
    pub methods: Vec<TargetMethod>,
    #[serde(default)]
    pub available_invariants: Vec<InvariantInfo>,
    /// The `cargo fuzz` target name this maps to, e.g. `counter_fuzz` — see
    /// `fuzz/Cargo.toml`'s `[[bin]]` entries. Consumed by the runner (phase
    /// 5) to build the actual `cargo +nightly fuzz run <name>` invocation.
    pub fuzz_target_name: String,
    #[serde(default)]
    pub known_buggy: bool,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    targets: Vec<Target>,
}

#[derive(Debug, thiserror::Error)]
pub enum TargetsError {
    #[error("failed to read target manifest at {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse target manifest at {path}: {source}")]
    Parse {
        path: String,
        source: serde_json::Error,
    },
    #[error("target manifest is invalid: {0}")]
    Invalid(String),
}

/// The loaded, validated set of targets, keyed by id for O(1) lookup.
#[derive(Debug, Clone)]
pub struct TargetRegistry {
    by_id: HashMap<String, Target>,
    /// Manifest order, so `list()` is stable rather than hash-order.
    order: Vec<String>,
}

impl TargetRegistry {
    pub fn load(targets_dir: &Path) -> Result<Self, TargetsError> {
        let manifest_path = targets_dir.join("targets.json");
        let raw = fs::read_to_string(&manifest_path).map_err(|source| TargetsError::Read {
            path: manifest_path.display().to_string(),
            source,
        })?;
        let manifest: Manifest =
            serde_json::from_str(&raw).map_err(|source| TargetsError::Parse {
                path: manifest_path.display().to_string(),
                source,
            })?;
        Self::from_targets(manifest.targets)
    }

    fn from_targets(targets: Vec<Target>) -> Result<Self, TargetsError> {
        let mut by_id = HashMap::with_capacity(targets.len());
        let mut order = Vec::with_capacity(targets.len());
        for target in targets {
            if target.id.trim().is_empty() {
                return Err(TargetsError::Invalid("target with empty id".to_string()));
            }
            if target.fuzz_target_name.trim().is_empty() {
                return Err(TargetsError::Invalid(format!(
                    "target `{}` has an empty fuzz_target_name",
                    target.id
                )));
            }
            if by_id.contains_key(&target.id) {
                return Err(TargetsError::Invalid(format!(
                    "duplicate target id `{}`",
                    target.id
                )));
            }
            order.push(target.id.clone());
            by_id.insert(target.id.clone(), target);
        }
        Ok(Self { by_id, order })
    }

    pub fn list(&self) -> Vec<&Target> {
        self.order
            .iter()
            .filter_map(|id| self.by_id.get(id))
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<&Target> {
        self.by_id.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: &str) -> Target {
        Target {
            id: id.to_string(),
            name: id.to_string(),
            contract_name: id.to_string(),
            methods: Vec::new(),
            available_invariants: Vec::new(),
            fuzz_target_name: format!("{id}_fuzz"),
            known_buggy: false,
        }
    }

    #[test]
    fn list_preserves_manifest_order() {
        let registry = TargetRegistry::from_targets(vec![target("b"), target("a")]).unwrap();
        let ids: Vec<_> = registry.list().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["b", "a"]);
    }

    #[test]
    fn get_finds_by_id() {
        let registry = TargetRegistry::from_targets(vec![target("counter")]).unwrap();
        assert!(registry.get("counter").is_some());
        assert!(registry.get("nope").is_none());
    }

    #[test]
    fn rejects_empty_id() {
        let mut t = target("counter");
        t.id = String::new();
        assert!(TargetRegistry::from_targets(vec![t]).is_err());
    }

    #[test]
    fn rejects_empty_fuzz_target_name() {
        let mut t = target("counter");
        t.fuzz_target_name = String::new();
        assert!(TargetRegistry::from_targets(vec![t]).is_err());
    }

    #[test]
    fn rejects_duplicate_id() {
        let err =
            TargetRegistry::from_targets(vec![target("counter"), target("counter")]).unwrap_err();
        assert!(matches!(err, TargetsError::Invalid(_)));
    }

    /// Guards the real manifest this backend ships (`examples/targets.json`)
    /// against staying valid, not just the loader logic in isolation.
    #[test]
    fn loads_the_real_manifest() {
        let targets_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let registry =
            TargetRegistry::load(&targets_dir).expect("examples/targets.json should load");

        for id in ["counter", "token", "escrow"] {
            let target = registry
                .get(id)
                .unwrap_or_else(|| panic!("missing target `{id}`"));
            assert_eq!(target.fuzz_target_name, format!("{id}_fuzz"));
            assert!(
                !target.available_invariants.is_empty(),
                "{id} should list its invariants"
            );
        }
        assert_eq!(registry.list().len(), 3);
    }
}
