//! Interfaces to outside services, each with a real implementation and a local or fake one.

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

// ---------------------------------------------------------------- storage

#[async_trait]
pub trait Storage: Send + Sync {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<()>;
    async fn get(&self, key: &str) -> Result<Vec<u8>>;
}

pub struct LocalStorage {
    pub root: PathBuf,
}

#[async_trait]
impl Storage for LocalStorage {
    async fn put(&self, key: &str, bytes: Vec<u8>, _content_type: &str) -> Result<()> {
        let path = self.root.join(key);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, bytes).await?;
        Ok(())
    }
    async fn get(&self, key: &str) -> Result<Vec<u8>> {
        Ok(tokio::fs::read(self.root.join(key)).await?)
    }
}

#[derive(Default)]
pub struct MemoryStorage {
    files: Mutex<HashMap<String, Vec<u8>>>,
}

#[async_trait]
impl Storage for MemoryStorage {
    async fn put(&self, key: &str, bytes: Vec<u8>, _content_type: &str) -> Result<()> {
        self.files.lock().unwrap().insert(key.to_string(), bytes);
        Ok(())
    }
    async fn get(&self, key: &str) -> Result<Vec<u8>> {
        self.files.lock().unwrap().get(key).cloned().ok_or_else(|| anyhow!("no such file {key}"))
    }
}

/// Google Cloud Storage via the JSON API, authenticated with the Cloud Run service account
/// token from the metadata server.
pub struct GcsStorage {
    pub bucket: String,
    pub http: reqwest::Client,
}

pub async fn gcp_access_token(http: &reqwest::Client) -> Result<String> {
    let v: Value = http
        .get("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token")
        .header("Metadata-Flavor", "Google")
        .send()
        .await
        .context("reading the metadata server token")?
        .error_for_status()?
        .json()
        .await?;
    v["access_token"].as_str().map(|s| s.to_string()).ok_or_else(|| anyhow!("no access token from metadata server"))
}

#[async_trait]
impl Storage for GcsStorage {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<()> {
        let token = gcp_access_token(&self.http).await?;
        self.http
            .post(format!("https://storage.googleapis.com/upload/storage/v1/b/{}/o", self.bucket))
            .query(&[("uploadType", "media"), ("name", key)])
            .bearer_auth(token)
            .header("content-type", if content_type.is_empty() { "application/octet-stream" } else { content_type })
            .body(bytes)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
    async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let token = gcp_access_token(&self.http).await?;
        let encoded: String = url::form_urlencoded::byte_serialize(key.as_bytes()).collect();
        let bytes = self
            .http
            .get(format!("https://storage.googleapis.com/storage/v1/b/{}/o/{}?alt=media", self.bucket, encoded))
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        Ok(bytes.to_vec())
    }
}

// ---------------------------------------------------------------- CDN and custom hostnames

#[derive(Clone, Debug)]
pub struct HostnameStatus {
    pub provider_id: String,
    pub active: bool,
    pub failed: bool,
    pub verification: Value,
}

#[async_trait]
pub trait Cdn: Send + Sync {
    async fn add_hostname(&self, hostname: &str) -> Result<HostnameStatus>;
    async fn hostname_status(&self, provider_id: &str) -> Result<HostnameStatus>;
    async fn remove_hostname(&self, provider_id: &str) -> Result<()>;
    async fn purge(&self, urls: &[String]) -> Result<()>;
}

/// Cloudflare for SaaS custom hostnames and cache purging.
pub struct Cloudflare {
    pub token: String,
    pub zone_id: String,
    pub http: reqwest::Client,
}

impl Cloudflare {
    fn url(&self, path: &str) -> String {
        format!("https://api.cloudflare.com/client/v4/zones/{}/{}", self.zone_id, path)
    }
    fn status(result: &Value) -> HostnameStatus {
        let status = result["status"].as_str().unwrap_or("pending");
        let ssl = result["ssl"]["status"].as_str().unwrap_or("pending");
        HostnameStatus {
            provider_id: result["id"].as_str().unwrap_or_default().to_string(),
            active: status == "active" && ssl == "active",
            failed: matches!(status, "blocked" | "moved" | "deleted"),
            verification: json!({
                "status": status,
                "ssl_status": ssl,
                "ownership_verification": result["ownership_verification"],
                "ssl_validation": result["ssl"]["validation_records"],
            }),
        }
    }
    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value> {
        let v: Value = req.bearer_auth(&self.token).send().await?.json().await?;
        if v["success"].as_bool() != Some(true) {
            bail!("Cloudflare error: {}", v["errors"]);
        }
        Ok(v["result"].clone())
    }
}

#[async_trait]
impl Cdn for Cloudflare {
    async fn add_hostname(&self, hostname: &str) -> Result<HostnameStatus> {
        let result = self
            .send(self.http.post(self.url("custom_hostnames")).json(&json!({
                "hostname": hostname,
                "ssl": { "method": "http", "type": "dv" }
            })))
            .await?;
        Ok(Self::status(&result))
    }
    async fn hostname_status(&self, provider_id: &str) -> Result<HostnameStatus> {
        let result = self.send(self.http.get(self.url(&format!("custom_hostnames/{provider_id}")))).await?;
        Ok(Self::status(&result))
    }
    async fn remove_hostname(&self, provider_id: &str) -> Result<()> {
        self.send(self.http.delete(self.url(&format!("custom_hostnames/{provider_id}")))).await?;
        Ok(())
    }
    async fn purge(&self, urls: &[String]) -> Result<()> {
        for chunk in urls.chunks(30) {
            self.send(self.http.post(self.url("purge_cache")).json(&json!({ "files": chunk }))).await?;
        }
        Ok(())
    }
}

/// No CDN: hostnames become active immediately and purges are recorded only.
#[derive(Default)]
pub struct FakeCdn {
    pub purged: Mutex<Vec<String>>,
    pub hostnames: Mutex<Vec<String>>,
}

#[async_trait]
impl Cdn for FakeCdn {
    async fn add_hostname(&self, hostname: &str) -> Result<HostnameStatus> {
        self.hostnames.lock().unwrap().push(hostname.to_string());
        Ok(HostnameStatus { provider_id: format!("fake-{hostname}"), active: false, failed: false, verification: json!({ "status": "pending" }) })
    }
    async fn hostname_status(&self, provider_id: &str) -> Result<HostnameStatus> {
        Ok(HostnameStatus { provider_id: provider_id.to_string(), active: true, failed: false, verification: json!({ "status": "active" }) })
    }
    async fn remove_hostname(&self, provider_id: &str) -> Result<()> {
        self.hostnames.lock().unwrap().retain(|h| format!("fake-{h}") != provider_id);
        Ok(())
    }
    async fn purge(&self, urls: &[String]) -> Result<()> {
        self.purged.lock().unwrap().extend(urls.iter().cloned());
        Ok(())
    }
}

// ---------------------------------------------------------------- GitHub

#[async_trait]
pub trait GitHubApi: Send + Sync {
    /// Creates or updates one file on a branch.
    async fn put_file(&self, token: &str, repo: &str, branch: &str, path: &str, content: &str, message: &str) -> Result<()>;
}

pub struct GitHub {
    pub http: reqwest::Client,
}

#[async_trait]
impl GitHubApi for GitHub {
    async fn put_file(&self, token: &str, repo: &str, branch: &str, path: &str, content: &str, message: &str) -> Result<()> {
        use base64::Engine;
        let url = format!("https://api.github.com/repos/{repo}/contents/{path}");
        let existing = self
            .http
            .get(&url)
            .query(&[("ref", branch)])
            .bearer_auth(token)
            .header("user-agent", "kwil")
            .header("accept", "application/vnd.github+json")
            .send()
            .await?;
        let sha = if existing.status().is_success() {
            existing.json::<Value>().await?["sha"].as_str().map(|s| s.to_string())
        } else {
            None
        };
        let mut body = json!({
            "message": message,
            "content": base64::engine::general_purpose::STANDARD.encode(content),
            "branch": branch,
        });
        if let Some(sha) = sha {
            body["sha"] = json!(sha);
        }
        let resp = self
            .http
            .put(&url)
            .bearer_auth(token)
            .header("user-agent", "kwil")
            .header("accept", "application/vnd.github+json")
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("GitHub returned {}: {}", resp.status(), resp.text().await.unwrap_or_default());
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct FakeGitHub {
    pub files: Mutex<Vec<(String, String, String)>>,
}

#[async_trait]
impl GitHubApi for FakeGitHub {
    async fn put_file(&self, _token: &str, repo: &str, _branch: &str, path: &str, content: &str, _message: &str) -> Result<()> {
        self.files.lock().unwrap().push((repo.to_string(), path.to_string(), content.to_string()));
        Ok(())
    }
}

// ---------------------------------------------------------------- email

#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(&self, to: &str, subject: &str, body: &str) -> Result<()>;
}

/// Leaves messages in the `email_outbox` table only.
pub struct OutboxMailer;

#[async_trait]
impl Mailer for OutboxMailer {
    async fn send(&self, to: &str, subject: &str, _body: &str) -> Result<()> {
        tracing::info!(to, subject, "email left in outbox (no mail provider configured)");
        Ok(())
    }
}

/// Sends through the Resend HTTP API.
pub struct ResendMailer {
    pub api_key: String,
    pub from: String,
    pub http: reqwest::Client,
}

#[async_trait]
impl Mailer for ResendMailer {
    async fn send(&self, to: &str, subject: &str, body: &str) -> Result<()> {
        self.http
            .post("https://api.resend.com/emails")
            .bearer_auth(&self.api_key)
            .json(&json!({ "from": self.from, "to": [to], "subject": subject, "text": body }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

// ---------------------------------------------------------------- web fetching

#[derive(Clone, Debug)]
pub struct FetchedPage {
    pub status: u16,
    pub body: String,
}

#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn get(&self, url: &str) -> Result<FetchedPage>;
}

pub struct HttpFetcher {
    pub http: reqwest::Client,
}

impl HttpFetcher {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("KwilBot/1.0 (+content library crawler)")
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .expect("http client"),
        }
    }
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Fetcher for HttpFetcher {
    async fn get(&self, url: &str) -> Result<FetchedPage> {
        let resp = self.http.get(url).send().await?;
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        Ok(FetchedPage { status, body })
    }
}

/// Serves pages from memory; any other URL is a 404.
#[derive(Default)]
pub struct FakeFetcher {
    pub pages: Mutex<HashMap<String, String>>,
    pub requested: Mutex<Vec<String>>,
}

impl FakeFetcher {
    pub fn set(&self, url: &str, body: &str) {
        self.pages.lock().unwrap().insert(url.to_string(), body.to_string());
    }
}

#[async_trait]
impl Fetcher for FakeFetcher {
    async fn get(&self, url: &str) -> Result<FetchedPage> {
        self.requested.lock().unwrap().push(url.to_string());
        Ok(match self.pages.lock().unwrap().get(url) {
            Some(body) => FetchedPage { status: 200, body: body.clone() },
            None => FetchedPage { status: 404, body: String::new() },
        })
    }
}
