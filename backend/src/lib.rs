//! Kwil: a multi-tenant SEO content platform grounded in a per-site knowledge library.

pub mod ai;
pub mod api;
pub mod auth;
pub mod blog;
pub mod clock;
pub mod config;
pub mod error;
pub mod ingest;
pub mod jobs;
pub mod library;
pub mod models;
pub mod retrieval;
pub mod services;
pub mod state;
pub mod structure;
pub mod text;
pub mod workflows;

use std::time::Duration;

pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn,tower_http=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// The in-process worker: runs due jobs continuously and the scheduled tick every minute.
pub async fn run_worker_loop(state: state::AppState, tick_every: Duration) {
    let mut last_tick = std::time::Instant::now() - tick_every;
    loop {
        if last_tick.elapsed() >= tick_every {
            if let Err(e) = workflows::tick(&state).await {
                tracing::error!(error = %e, "scheduled tick failed");
            }
            last_tick = std::time::Instant::now();
        }
        match jobs::run_next(&state).await {
            Ok(true) => continue,
            Ok(false) => tokio::time::sleep(Duration::from_millis(500)).await,
            Err(e) => {
                tracing::error!(error = %e, "worker error");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

pub async fn serve(router: axum::Router, port: u16) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("listening on {}", listener.local_addr()?);
    axum::serve(listener, router).await?;
    Ok(())
}
