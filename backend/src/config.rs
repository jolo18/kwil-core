use anyhow::{bail, Context, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Env {
    Development,
    Production,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub env: Env,
    pub database_url: String,
    pub api_port: u16,
    pub blog_port: u16,
    /// Public URL of the web app, used in emails and digests.
    pub app_url: String,
    pub cors_origin: String,
    /// Sites are served at `<slug>.<blog_base_domain>`.
    pub blog_base_domain: String,
    /// Shared secret for Cloud Scheduler and Cloud Tasks calls to `/internal/*`.
    pub internal_secret: String,
    pub auth_mode: String,
    pub clerk_jwks_url: Option<String>,
    pub clerk_issuer: Option<String>,
    pub clerk_webhook_secret: Option<String>,
    pub ai_provider: String,
    pub openrouter_api_key: Option<String>,
    pub queue: String,
    pub cloud_tasks_queue: Option<String>,
    pub worker_url: Option<String>,
    /// Service account Cloud Tasks uses to call the private worker service.
    pub cloud_tasks_service_account: Option<String>,
    pub storage: String,
    pub storage_dir: String,
    pub gcs_bucket: Option<String>,
    pub cdn: String,
    pub cloudflare_api_token: Option<String>,
    pub cloudflare_zone_id: Option<String>,
    pub cloudflare_cname_target: String,
    pub mailer: String,
    pub resend_api_key: Option<String>,
    pub mail_from: String,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn var_or(name: &str, default: &str) -> String {
    var(name).unwrap_or_else(|| default.to_string())
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let env = match var_or("APP_ENV", "development").as_str() {
            "production" => Env::Production,
            "development" => Env::Development,
            other => bail!("APP_ENV must be development or production, got {other}"),
        };
        let config = Config {
            env,
            database_url: var("DATABASE_URL").context("DATABASE_URL is required")?,
            api_port: var_or("API_PORT", &var_or("PORT", "8080")).parse()?,
            blog_port: var_or("BLOG_PORT", "8081").parse()?,
            app_url: var_or("APP_URL", "http://localhost:5173"),
            cors_origin: var_or("CORS_ORIGIN", "http://localhost:5173"),
            blog_base_domain: var_or("BLOG_BASE_DOMAIN", "localhost"),
            internal_secret: var_or("INTERNAL_SECRET", "dev-internal-secret"),
            auth_mode: var_or("AUTH_MODE", "dev"),
            clerk_jwks_url: var("CLERK_JWKS_URL"),
            clerk_issuer: var("CLERK_ISSUER"),
            clerk_webhook_secret: var("CLERK_WEBHOOK_SECRET"),
            ai_provider: var_or("AI_PROVIDER", "demo"),
            openrouter_api_key: var("OPENROUTER_API_KEY"),
            queue: var_or("QUEUE", "local"),
            cloud_tasks_queue: var("CLOUD_TASKS_QUEUE"),
            worker_url: var("WORKER_URL"),
            cloud_tasks_service_account: var("CLOUD_TASKS_SERVICE_ACCOUNT"),
            storage: var_or("STORAGE", "local"),
            storage_dir: var_or("STORAGE_DIR", "./data/uploads"),
            gcs_bucket: var("GCS_BUCKET"),
            cdn: var_or("CDN", "none"),
            cloudflare_api_token: var("CLOUDFLARE_API_TOKEN"),
            cloudflare_zone_id: var("CLOUDFLARE_ZONE_ID"),
            cloudflare_cname_target: var_or("CLOUDFLARE_CNAME_TARGET", "sites.example.com"),
            mailer: var_or("MAILER", "outbox"),
            resend_api_key: var("RESEND_API_KEY"),
            mail_from: var_or("MAIL_FROM", "Kwil <noreply@example.com>"),
        };
        config.validate()?;
        Ok(config)
    }

    /// Refuses unsafe combinations, most importantly the development login in production.
    pub fn validate(&self) -> Result<()> {
        if self.env == Env::Production {
            if self.auth_mode == "dev" {
                bail!("AUTH_MODE=dev is refused when APP_ENV=production");
            }
            if self.internal_secret == "dev-internal-secret" {
                bail!("INTERNAL_SECRET must be set in production");
            }
        }
        if self.auth_mode == "clerk" && self.clerk_jwks_url.is_none() {
            bail!("AUTH_MODE=clerk needs CLERK_JWKS_URL");
        }
        if self.ai_provider == "openrouter" && self.openrouter_api_key.is_none() {
            bail!("AI_PROVIDER=openrouter needs OPENROUTER_API_KEY");
        }
        Ok(())
    }

    /// Configuration used by tests and as a base for local development.
    pub fn for_tests(database_url: &str) -> Self {
        Config {
            env: Env::Development,
            database_url: database_url.to_string(),
            api_port: 0,
            blog_port: 0,
            app_url: "http://app.test".into(),
            cors_origin: "http://app.test".into(),
            blog_base_domain: "blog.test".into(),
            internal_secret: "test-secret".into(),
            auth_mode: "dev".into(),
            clerk_jwks_url: None,
            clerk_issuer: None,
            clerk_webhook_secret: None,
            ai_provider: "scripted".into(),
            openrouter_api_key: None,
            queue: "local".into(),
            cloud_tasks_queue: None,
            worker_url: None,
            cloud_tasks_service_account: None,
            storage: "memory".into(),
            storage_dir: String::new(),
            gcs_bucket: None,
            cdn: "fake".into(),
            cloudflare_api_token: None,
            cloudflare_zone_id: None,
            cloudflare_cname_target: "sites.blog.test".into(),
            mailer: "outbox".into(),
            resend_api_key: None,
            mail_from: "test@blog.test".into(),
        }
    }
}
