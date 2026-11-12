use crate::ai::llm::{LlmProvider, OpenRouter};
use crate::auth::{build_authenticator, Authenticator};
use crate::clock::{Clock, SystemClock};
use crate::config::Config;
use crate::jobs::{CloudTasksQueue, JobQueue, LocalQueue};
use crate::services::*;
use anyhow::{bail, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::path::PathBuf;
use std::sync::Arc;

/// Everything a handler or job needs. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub clock: Arc<dyn Clock>,
    pub llm: Arc<dyn LlmProvider>,
    pub queue: Arc<dyn JobQueue>,
    pub auth: Arc<dyn Authenticator>,
    pub storage: Arc<dyn Storage>,
    pub cdn: Arc<dyn Cdn>,
    pub github: Arc<dyn GitHubApi>,
    pub mailer: Arc<dyn Mailer>,
    pub fetcher: Arc<dyn Fetcher>,
}

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub async fn connect(database_url: &str) -> Result<PgPool> {
    let pool = PgPoolOptions::new().max_connections(10).connect(database_url).await?;
    MIGRATOR.run(&pool).await?;
    Ok(pool)
}

impl AppState {
    /// Builds the real services described by the configuration.
    pub async fn from_config(config: Config) -> Result<Self> {
        let db = connect(&config.database_url).await?;
        let http = reqwest::Client::new();
        let llm: Arc<dyn LlmProvider> = match config.ai_provider.as_str() {
            "openrouter" => Arc::new(OpenRouter::new(config.openrouter_api_key.clone().unwrap_or_default())),
            "demo" => Arc::new(crate::ai::demo::Demo),
            other => bail!("unknown AI_PROVIDER {other}"),
        };
        let queue: Arc<dyn JobQueue> = match config.queue.as_str() {
            "local" => Arc::new(LocalQueue),
            "cloudtasks" => Arc::new(CloudTasksQueue {
                queue: config.cloud_tasks_queue.clone().unwrap_or_default(),
                worker_url: config.worker_url.clone().unwrap_or_default(),
                secret: config.internal_secret.clone(),
                service_account: config.cloud_tasks_service_account.clone(),
                http: http.clone(),
            }),
            other => bail!("unknown QUEUE {other}"),
        };
        let storage: Arc<dyn Storage> = match config.storage.as_str() {
            "local" => Arc::new(LocalStorage { root: PathBuf::from(&config.storage_dir) }),
            "gcs" => Arc::new(GcsStorage { bucket: config.gcs_bucket.clone().unwrap_or_default(), http: http.clone() }),
            "memory" => Arc::new(MemoryStorage::default()),
            other => bail!("unknown STORAGE {other}"),
        };
        let cdn: Arc<dyn Cdn> = match config.cdn.as_str() {
            "cloudflare" => Arc::new(Cloudflare {
                token: config.cloudflare_api_token.clone().unwrap_or_default(),
                zone_id: config.cloudflare_zone_id.clone().unwrap_or_default(),
                http: http.clone(),
            }),
            _ => Arc::new(FakeCdn::default()),
        };
        let mailer: Arc<dyn Mailer> = match config.mailer.as_str() {
            "resend" => Arc::new(ResendMailer {
                api_key: config.resend_api_key.clone().unwrap_or_default(),
                from: config.mail_from.clone(),
                http: http.clone(),
            }),
            _ => Arc::new(OutboxMailer),
        };
        Ok(AppState {
            auth: build_authenticator(&config)?,
            db,
            clock: Arc::new(SystemClock),
            llm,
            queue,
            storage,
            cdn,
            github: Arc::new(GitHub { http: http.clone() }),
            mailer,
            fetcher: Arc::new(HttpFetcher::new()),
            config: Arc::new(config),
        })
    }
}
