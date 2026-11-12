//! The model provider interface, with the OpenRouter implementation and a scripted fake.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Strongest model: drafting.
    Draft,
    /// Mid-tier: extracting proposed entries, verifying, reviewing.
    Mid,
    /// Cheapest: tagging and classifying.
    Cheap,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Draft => "draft",
            Tier::Mid => "mid",
            Tier::Cheap => "cheap",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LlmRequest {
    /// Machine name of the task, such as `draft` or `extract_entries`.
    pub task: String,
    pub tier: Tier,
    pub model: String,
    pub system: String,
    /// Stable text sent identically on every call for a site (the site profile), marked for caching.
    pub cached_prefix: Option<String>,
    pub user: String,
    /// The structured input the prompt was built from. Fakes read this instead of the prose.
    pub context: Value,
}

#[derive(Clone, Debug, Default)]
pub struct LlmResponse {
    pub text: String,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cost_usd: f64,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse>;
}

/// OpenRouter chat completions, asking for JSON output and per-call cost.
pub struct OpenRouter {
    api_key: String,
    http: reqwest::Client,
    base_url: String,
}

impl OpenRouter {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(300)).build().expect("http client"),
            base_url: "https://openrouter.ai/api/v1".into(),
        }
    }
}

#[async_trait]
impl LlmProvider for OpenRouter {
    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse> {
        let mut system_parts = vec![json!({ "type": "text", "text": req.system })];
        if let Some(prefix) = &req.cached_prefix {
            system_parts.push(json!({
                "type": "text",
                "text": prefix,
                "cache_control": { "type": "ephemeral" }
            }));
        }
        let body = json!({
            "model": req.model,
            "messages": [
                { "role": "system", "content": system_parts },
                { "role": "user", "content": req.user }
            ],
            "response_format": { "type": "json_object" },
            "usage": { "include": true }
        });
        let resp = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .header("X-Title", "Kwil")
            .json(&body)
            .send()
            .await
            .context("calling OpenRouter")?;
        let status = resp.status();
        let value: Value = resp.json().await.context("reading OpenRouter response")?;
        if !status.is_success() {
            return Err(anyhow!("OpenRouter returned {status}: {value}"));
        }
        let text = value["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow!("OpenRouter response had no content: {value}"))?
            .to_string();
        let usage = &value["usage"];
        Ok(LlmResponse {
            text,
            input_tokens: usage["prompt_tokens"].as_i64().unwrap_or(0) as i32,
            output_tokens: usage["completion_tokens"].as_i64().unwrap_or(0) as i32,
            cost_usd: usage["cost"].as_f64().unwrap_or(0.0),
        })
    }
}

/// Test fake: returns queued responses per task, and falls back to the deterministic demo
/// provider when nothing is queued. Records every request it receives.
pub struct Scripted {
    queued: Mutex<HashMap<String, VecDeque<Value>>>,
    pub calls: Mutex<Vec<LlmRequest>>,
    pub cost_per_call: Mutex<f64>,
    fallback: crate::ai::demo::Demo,
}

impl Default for Scripted {
    fn default() -> Self {
        Self {
            queued: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
            cost_per_call: Mutex::new(0.01),
            fallback: crate::ai::demo::Demo,
        }
    }
}

impl Scripted {
    pub fn push(&self, task: &str, response: Value) {
        self.queued.lock().unwrap().entry(task.to_string()).or_default().push_back(response);
    }
    pub fn calls_for(&self, task: &str) -> Vec<LlmRequest> {
        self.calls.lock().unwrap().iter().filter(|c| c.task == task).cloned().collect()
    }
    pub fn set_cost(&self, cost: f64) {
        *self.cost_per_call.lock().unwrap() = cost;
    }
}

#[async_trait]
impl LlmProvider for Scripted {
    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse> {
        self.calls.lock().unwrap().push(req.clone());
        let queued = self.queued.lock().unwrap().get_mut(&req.task).and_then(|q| q.pop_front());
        let mut resp = match queued {
            Some(v) => LlmResponse { text: v.to_string(), ..Default::default() },
            None => self.fallback.complete(req).await?,
        };
        resp.cost_usd = *self.cost_per_call.lock().unwrap();
        resp.input_tokens = 100;
        resp.output_tokens = 100;
        Ok(resp)
    }
}
