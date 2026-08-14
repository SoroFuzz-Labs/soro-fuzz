//! Env-based configuration. Every field here corresponds to a documented
//! entry in the README's config table — keep the two in sync.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required env var {0}")]
    Missing(&'static str),
    #[error("invalid value for {name}: {value:?} ({reason})")]
    Invalid {
        name: &'static str,
        value: String,
        reason: String,
    },
}

/// How a fuzz job's subprocess is isolated. `local` runs `cargo fuzz`
/// directly (only safe on a dev box that already has the toolchain and
/// nothing else worth protecting); `docker-exec` runs it inside an
/// already-running sidecar container via `docker exec`, which is what
/// docker-compose's `toolchain` service is for.
///
/// contributors: add a new sandbox strategy by extending this enum, parsing
/// it in `parse_sandbox`, and adding a matching impl of
/// `runner::sandbox::Sandbox` (e.g. an `nsjail` variant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxKind {
    Local,
    DockerExec { container: String },
}

// default_budget_secs isn't read yet: `CreateCampaignRequest.timeBudgetSecs`
// is required so nothing server-side needs it; it's validated now in case a
// future endpoint surfaces it to the frontend as a form default. Loaded and
// validated now so `Config::from_env` is the one place startup fails on bad
// env, ahead of the features that consume them.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub http_addr: SocketAddr,
    pub targets_dir: PathBuf,
    pub default_budget_secs: u32,
    pub max_concurrent_campaigns: usize,
    pub rust_toolchain: String,
    /// `cargo fuzz`'s `--sanitizer` flag, e.g. `Some("thread")` to work
    /// around the macOS cargo-fuzz linking issue. `None` omits the flag.
    pub fuzz_sanitizer: Option<String>,
    pub fuzz_sandbox: SandboxKind,
    /// Where the `fuzz/` cargo-fuzz workspace can be found *from the
    /// sandbox's point of view* — a host path for `SandboxKind::Local`, or
    /// wherever the `docker-exec` sandbox's target container mounts it for
    /// `SandboxKind::DockerExec` (docker-compose is responsible for making
    /// those consistent). Not in the original config table this project
    /// started from — added because "TARGETS_DIR" (the contracts manifest
    /// location) and "where the separate fuzz driver workspace lives" turned
    /// out to be two different paths once the real runner needed to `cd`
    /// somewhere and run `cargo fuzz`. Defaults to `{TARGETS_DIR}/../fuzz`,
    /// which is where it lives in this repo.
    pub fuzz_workspace_dir: PathBuf,
    pub job_timeout: Duration,
    pub log_level: String,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let targets_dir = PathBuf::from(required("TARGETS_DIR")?);
        let default_fuzz_workspace_dir = targets_dir.join("..").join("fuzz");

        Ok(Self {
            database_url: required("DATABASE_URL")?,
            http_addr: parse_default("HTTP_ADDR", "0.0.0.0:8080")?,
            default_budget_secs: parse_default("DEFAULT_BUDGET_SECONDS", "60")?,
            max_concurrent_campaigns: parse_default("MAX_CONCURRENT_CAMPAIGNS", "2")?,
            rust_toolchain: env::var("RUST_TOOLCHAIN").unwrap_or_else(|_| "nightly".to_string()),
            fuzz_sanitizer: env::var("FUZZ_SANITIZER").ok().filter(|s| !s.is_empty()),
            fuzz_sandbox: parse_sandbox(
                env::var("FUZZ_SANDBOX").unwrap_or_else(|_| "local".to_string()),
            )?,
            fuzz_workspace_dir: env::var("FUZZ_WORKSPACE_DIR")
                .map(PathBuf::from)
                .unwrap_or(default_fuzz_workspace_dir),
            job_timeout: Duration::from_secs(parse_default("JOB_TIMEOUT_SECONDS", "600")?),
            log_level: env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
            targets_dir,
        })
    }
}

fn required(name: &'static str) -> Result<String, ConfigError> {
    env::var(name).map_err(|_| ConfigError::Missing(name))
}

fn parse_default<T>(name: &'static str, default: &str) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let raw = env::var(name).unwrap_or_else(|_| default.to_string());
    raw.parse::<T>().map_err(|e| ConfigError::Invalid {
        name,
        value: raw.clone(),
        reason: e.to_string(),
    })
}

fn parse_sandbox(raw: String) -> Result<SandboxKind, ConfigError> {
    match raw.split_once(':') {
        None if raw == "local" => Ok(SandboxKind::Local),
        Some(("docker-exec", container)) if !container.is_empty() => Ok(SandboxKind::DockerExec {
            container: container.to_string(),
        }),
        _ => Err(ConfigError::Invalid {
            name: "FUZZ_SANDBOX",
            value: raw,
            reason: "expected `local` or `docker-exec:<container-name>`".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_parses_local() {
        assert_eq!(
            parse_sandbox("local".to_string()).unwrap(),
            SandboxKind::Local
        );
    }

    #[test]
    fn sandbox_parses_docker_exec() {
        assert_eq!(
            parse_sandbox("docker-exec:toolchain".to_string()).unwrap(),
            SandboxKind::DockerExec {
                container: "toolchain".to_string()
            }
        );
    }

    #[test]
    fn sandbox_rejects_unknown_value() {
        assert!(parse_sandbox("nsjail".to_string()).is_err());
    }

    #[test]
    fn sandbox_rejects_empty_container_name() {
        assert!(parse_sandbox("docker-exec:".to_string()).is_err());
    }
}
