//! One binary, four roles:
//!   kwil api     the JSON API (also accepts Cloud Scheduler and Cloud Tasks calls)
//!   kwil worker  runs jobs: polls the queue locally, or serves Cloud Tasks pushes
//!   kwil blog    the public, read-only blog service
//!   kwil dev     all of the above in one process, for local development

use kwil::config::Config;
use kwil::state::AppState;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    kwil::init_tracing();
    let role = std::env::args().nth(1).unwrap_or_else(|| "dev".into());
    let config = Config::from_env()?;
    let state = AppState::from_config(config.clone()).await?;
    match role.as_str() {
        "api" => kwil::serve(kwil::api::router(state), config.api_port).await,
        "blog" => kwil::serve(kwil::blog::router(state), config.blog_port).await,
        "worker" => {
            if config.queue == "local" {
                tokio::spawn(kwil::run_worker_loop(state.clone(), Duration::from_secs(60)));
            }
            kwil::serve(kwil::api::router(state), config.api_port).await
        }
        "dev" => {
            tokio::spawn(kwil::run_worker_loop(state.clone(), Duration::from_secs(60)));
            let blog = kwil::serve(kwil::blog::router(state.clone()), config.blog_port);
            let api = kwil::serve(kwil::api::router(state), config.api_port);
            tokio::try_join!(api, blog)?;
            Ok(())
        }
        other => anyhow::bail!("unknown role {other}; use api, worker, blog or dev"),
    }
}
