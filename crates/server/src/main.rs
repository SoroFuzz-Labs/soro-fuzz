mod api;
mod config;
mod runner;
mod store;
mod targets;
mod worker;

use std::sync::Arc;

use tracing_subscriber::EnvFilter;

use config::{Config, SandboxKind};
use runner::sandbox::{DockerExecSandbox, LocalSandbox, Sandbox};
use runner::subprocess::SubprocessRunner;
use runner::Runner;
use store::PgStore;
use store::Store as _;
use targets::TargetRegistry;
use worker::progress::ProgressHub;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Dev convenience only — real deployments set these directly; a missing
    // .env is not an error.
    dotenvy::dotenv().ok();

    let config = Config::from_env()?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_new(&config.log_level).unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("connecting to database");
    let store = PgStore::connect(&config.database_url).await?;
    store.migrate().await?;
    tracing::info!("migrations applied");

    tracing::info!(dir = %config.targets_dir.display(), "loading target manifest");
    let targets = TargetRegistry::load(&config.targets_dir)?;
    store
        .sync_targets(&targets.list().into_iter().cloned().collect::<Vec<_>>())
        .await?;
    tracing::info!(count = targets.list().len(), "targets synced");

    let store: Arc<dyn store::Store> = Arc::new(store);
    let targets = Arc::new(targets);

    let sandbox: Arc<dyn Sandbox> = match &config.fuzz_sandbox {
        SandboxKind::Local => Arc::new(LocalSandbox),
        SandboxKind::DockerExec { container } => Arc::new(DockerExecSandbox {
            container: container.clone(),
        }),
    };
    // contributors: `runner::mock::MockRunner` remains available for tests
    // (and anywhere a toolchain isn't installed) — swap it in here instead
    // if you need to run this server without nightly/cargo-fuzz.
    let runner: Arc<dyn Runner> = Arc::new(SubprocessRunner::new(
        sandbox,
        config.fuzz_workspace_dir.clone(),
        config.rust_toolchain.clone(),
        config.fuzz_sanitizer.clone(),
        config.job_timeout,
    ));
    let progress = Arc::new(ProgressHub::default());

    tracing::info!(
        count = config.max_concurrent_campaigns,
        "starting worker pool"
    );
    let _workers = worker::spawn_pool(
        config.max_concurrent_campaigns,
        store.clone(),
        runner,
        targets.clone(),
        progress,
        config.fuzz_sanitizer.clone(),
    );

    let state = api::AppState {
        store,
        targets,
        max_time_budget_secs: config.job_timeout.as_secs() as i64,
    };
    let app = api::router(state);

    let listener = tokio::net::TcpListener::bind(config.http_addr).await?;
    tracing::info!(addr = %config.http_addr, "listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received, draining in-flight requests");
}
