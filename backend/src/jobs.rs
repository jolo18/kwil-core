//! Background jobs. A `jobs` table records every job and its status; the queue only decides
//! how a job gets run: polled by the in-process worker, or pushed by Cloud Tasks.

use crate::state::AppState;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Job {
    pub id: Uuid,
    pub org_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub kind: String,
    pub payload: Value,
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub run_after: DateTime<Utc>,
    pub last_error: Option<String>,
    pub result: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[async_trait]
pub trait JobQueue: Send + Sync {
    /// Arranges for the job to run at or after `at`.
    async fn dispatch(&self, job_id: Uuid, at: DateTime<Utc>) -> Result<()>;
}

/// The in-process worker polls the table, so dispatching needs no action.
pub struct LocalQueue;

#[async_trait]
impl JobQueue for LocalQueue {
    async fn dispatch(&self, _job_id: Uuid, _at: DateTime<Utc>) -> Result<()> {
        Ok(())
    }
}

/// Google Cloud Tasks: each job becomes an HTTP task that calls the worker service.
pub struct CloudTasksQueue {
    /// `projects/<p>/locations/<l>/queues/<q>`
    pub queue: String,
    /// Base URL of the worker service.
    pub worker_url: String,
    pub secret: String,
    pub service_account: Option<String>,
    pub http: reqwest::Client,
}

#[async_trait]
impl JobQueue for CloudTasksQueue {
    async fn dispatch(&self, job_id: Uuid, at: DateTime<Utc>) -> Result<()> {
        use base64::Engine;
        let token = crate::services::gcp_access_token(&self.http).await?;
        let mut body = json!({
            "task": {
                "scheduleTime": at.to_rfc3339(),
                "httpRequest": {
                    "httpMethod": "POST",
                    "url": format!("{}/internal/jobs/{job_id}/run", self.worker_url.trim_end_matches('/')),
                    "headers": { "x-internal-secret": self.secret, "content-type": "application/json" },
                    "body": base64::engine::general_purpose::STANDARD.encode("{}"),
                }
            }
        });
        if let Some(sa) = &self.service_account {
            body["task"]["httpRequest"]["oidcToken"] = json!({ "serviceAccountEmail": sa, "audience": self.worker_url });
        }
        self.http
            .post(format!("https://cloudtasks.googleapis.com/v2/{}/tasks", self.queue))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .context("creating Cloud Task")?
            .error_for_status()?;
        Ok(())
    }
}

pub async fn enqueue(state: &AppState, org_id: Option<Uuid>, site_id: Option<Uuid>, kind: &str, payload: Value) -> Result<Uuid> {
    let id = Uuid::new_v4();
    let now = state.clock.now();
    sqlx::query(
        "insert into jobs (id, org_id, site_id, kind, payload, run_after, created_at, updated_at)
         values ($1, $2, $3, $4, $5, $6, $6, $6)",
    )
    .bind(id)
    .bind(org_id)
    .bind(site_id)
    .bind(kind)
    .bind(&payload)
    .bind(now)
    .execute(&state.db)
    .await?;
    state.queue.dispatch(id, now).await?;
    Ok(id)
}

/// Enqueues a job unless an identical one is already waiting or running.
pub async fn enqueue_once(state: &AppState, org_id: Option<Uuid>, site_id: Option<Uuid>, kind: &str, payload: Value) -> Result<Option<Uuid>> {
    let exists: bool = sqlx::query_scalar(
        "select exists(select 1 from jobs where kind = $1 and payload = $2 and status in ('queued', 'running', 'failed'))",
    )
    .bind(kind)
    .bind(&payload)
    .fetch_one(&state.db)
    .await?;
    if exists {
        return Ok(None);
    }
    Ok(Some(enqueue(state, org_id, site_id, kind, payload).await?))
}

async fn claim(state: &AppState, job_id: Option<Uuid>) -> Result<Option<Job>> {
    let now = state.clock.now();
    let job = match job_id {
        Some(id) => {
            sqlx::query_as::<_, Job>(
                "update jobs set status = 'running', attempts = attempts + 1, updated_at = $2
                 where id = $1 and status in ('queued', 'failed') returning *",
            )
            .bind(id)
            .bind(now)
            .fetch_optional(&state.db)
            .await?
        }
        None => {
            sqlx::query_as::<_, Job>(
                "update jobs set status = 'running', attempts = attempts + 1, updated_at = $1
                 where id = (select id from jobs where status in ('queued', 'failed') and run_after <= $1
                             order by run_after, created_at limit 1 for update skip locked)
                 returning *",
            )
            .bind(now)
            .fetch_optional(&state.db)
            .await?
        }
    };
    Ok(job)
}

async fn finish(state: &AppState, job: &Job, outcome: Result<Value>) -> Result<()> {
    let now = state.clock.now();
    match outcome {
        Ok(result) => {
            sqlx::query("update jobs set status = 'succeeded', result = $2, last_error = null, updated_at = $3 where id = $1")
                .bind(job.id)
                .bind(result)
                .bind(now)
                .execute(&state.db)
                .await?;
        }
        Err(err) => {
            let message = format!("{err:#}");
            tracing::warn!(job = %job.id, kind = %job.kind, attempt = job.attempts, error = %message, "job failed");
            if job.attempts >= job.max_attempts {
                sqlx::query("update jobs set status = 'dead', last_error = $2, updated_at = $3 where id = $1")
                    .bind(job.id)
                    .bind(&message)
                    .bind(now)
                    .execute(&state.db)
                    .await?;
                if let (Some(org), site) = (job.org_id, job.site_id) {
                    crate::workflows::notify(state, org, site, "job_failed", &format!("A {} job failed after {} attempts: {}", job.kind.replace('_', " "), job.attempts, message)).await?;
                }
                crate::workflows::on_job_dead(state, job).await.ok();
            } else {
                let backoff = chrono::Duration::seconds(30 * 2i64.pow(job.attempts as u32));
                let at = now + backoff;
                sqlx::query("update jobs set status = 'failed', last_error = $2, run_after = $3, updated_at = $4 where id = $1")
                    .bind(job.id)
                    .bind(&message)
                    .bind(at)
                    .bind(now)
                    .execute(&state.db)
                    .await?;
                state.queue.dispatch(job.id, at).await?;
            }
        }
    }
    Ok(())
}

async fn process(state: &AppState, job: Job) -> Result<()> {
    let outcome = crate::workflows::run_job(state, &job).await;
    finish(state, &job, outcome).await
}

/// Runs one specific job (the Cloud Tasks path). Returns false if it was not runnable.
pub async fn run_one(state: &AppState, job_id: Uuid) -> Result<bool> {
    match claim(state, Some(job_id)).await? {
        Some(job) => {
            process(state, job).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Runs the next due job, if any (the in-process worker path).
pub async fn run_next(state: &AppState) -> Result<bool> {
    match claim(state, None).await? {
        Some(job) => {
            process(state, job).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Runs due jobs until none are left. Used by tests and the local worker loop.
pub async fn drain(state: &AppState) -> Result<usize> {
    let mut count = 0;
    while run_next(state).await? {
        count += 1;
        if count > 500 {
            anyhow::bail!("job queue did not settle after 500 jobs");
        }
    }
    Ok(count)
}
