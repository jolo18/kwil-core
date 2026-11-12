#![allow(dead_code)]
//! Test harness: a fresh database per test, the real routers, and fakes at the edges only.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use kwil::ai::llm::Scripted;
use kwil::auth::DevAuth;
use kwil::clock::FixedClock;
use kwil::config::Config;
use kwil::jobs::LocalQueue;
use kwil::services::*;
use kwil::state::AppState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

pub struct TestApp {
    pub state: AppState,
    pub api: Router,
    pub blog: Router,
    pub clock: Arc<FixedClock>,
    pub llm: Arc<Scripted>,
    pub cdn: Arc<FakeCdn>,
    pub github: Arc<FakeGitHub>,
    pub fetcher: Arc<FakeFetcher>,
}

pub struct Who {
    pub user: &'static str,
    pub org: &'static str,
    pub role: &'static str,
}

pub const OWNER: Who = Who { user: "dev-user", org: "dev-org", role: "owner" };
pub const OTHER_TENANT: Who = Who { user: "mallory", org: "other-org", role: "owner" };

/// Monday 16 November 2026, 08:00 UTC.
pub fn start_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 16, 8, 0, 0).unwrap()
}

impl TestApp {
    pub async fn new() -> TestApp {
        Self::with_auth(Arc::new(DevAuth)).await
    }

    pub async fn with_auth(auth: Arc<dyn kwil::auth::Authenticator>) -> TestApp {
        let admin = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://postgres@localhost:5432/postgres".into());
        let name = format!("kwil_test_{}", Uuid::new_v4().simple());
        let pool = sqlx::PgPool::connect(&admin).await.expect("connect to test postgres");
        sqlx::query(sqlx::AssertSqlSafe(format!("create database {name}"))).execute(&pool).await.unwrap();
        pool.close().await;
        let url = match admin.rfind('/') {
            Some(i) => format!("{}/{}", &admin[..i], name),
            None => panic!("TEST_DATABASE_URL must end with a database name"),
        };
        let db = kwil::state::connect(&url).await.expect("migrate test database");
        let clock = Arc::new(FixedClock::new(start_time()));
        let llm = Arc::new(Scripted::default());
        let cdn = Arc::new(FakeCdn::default());
        let github = Arc::new(FakeGitHub::default());
        let fetcher = Arc::new(FakeFetcher::default());
        let state = AppState {
            db,
            config: Arc::new(Config::for_tests(&url)),
            clock: clock.clone(),
            llm: llm.clone(),
            queue: Arc::new(LocalQueue),
            auth,
            storage: Arc::new(MemoryStorage::default()),
            cdn: cdn.clone(),
            github: github.clone(),
            mailer: Arc::new(OutboxMailer),
            fetcher: fetcher.clone(),
        };
        TestApp { api: kwil::api::router(state.clone()), blog: kwil::blog::router(state.clone()), state, clock, llm, cdn, github, fetcher }
    }

    pub async fn request_as(&self, who: &Who, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("x-dev-user", who.user)
            .header("x-dev-org", who.org)
            .header("x-dev-role", who.role);
        let body = match body {
            Some(b) => {
                req = req.header("content-type", "application/json");
                Body::from(b.to_string())
            }
            None => Body::empty(),
        };
        let resp = self.api.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()));
        (status, value)
    }

    pub async fn raw(&self, req: Request<Body>) -> (StatusCode, String) {
        let resp = self.api.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    pub async fn get(&self, path: &str) -> Value {
        let (status, v) = self.request_as(&OWNER, "GET", path, None).await;
        assert!(status.is_success(), "GET {path} returned {status}: {v}");
        v
    }

    pub async fn post(&self, path: &str, body: Value) -> Value {
        let (status, v) = self.request_as(&OWNER, "POST", path, Some(body)).await;
        assert!(status.is_success(), "POST {path} returned {status}: {v}");
        v
    }

    pub async fn patch(&self, path: &str, body: Value) -> Value {
        let (status, v) = self.request_as(&OWNER, "PATCH", path, Some(body)).await;
        assert!(status.is_success(), "PATCH {path} returned {status}: {v}");
        v
    }

    pub async fn blog_get(&self, host: &str, path: &str) -> (StatusCode, String) {
        let req = Request::builder().uri(path).header("host", host).body(Body::empty()).unwrap();
        let resp = self.blog.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    pub async fn run_jobs(&self) -> usize {
        kwil::jobs::drain(&self.state).await.expect("jobs ran")
    }

    /// The scheduled tick followed by all the work it created.
    pub async fn tick(&self) {
        let req = Request::builder().method("POST").uri("/internal/cron/tick").header("x-internal-secret", "test-secret").body(Body::empty()).unwrap();
        let (status, body) = self.raw(req).await;
        assert_eq!(status, StatusCode::OK, "tick failed: {body}");
        self.run_jobs().await;
    }

    pub async fn advance_to(&self, at: chrono::DateTime<Utc>) {
        self.clock.set(at);
        self.tick().await;
    }

    pub async fn job_errors(&self) -> Vec<(String, String)> {
        sqlx::query_as("select kind, coalesce(last_error, '') from jobs where status in ('failed', 'dead')").fetch_all(&self.state.db).await.unwrap()
    }
}

pub fn answers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("product", "Ledgerly sends invoices automatically when a project milestone is marked done. Agencies stop chasing payments by email because reminders go out on a schedule. Invoices sync to accounting software within a minute."),
        ("audience", "Small agencies with five to fifty people who bill clients by project. The agency owner usually does invoicing at night and hates every minute of it."),
        ("pain_points", "Customers tell us invoicing takes their agencies a full day every month. Late payments are the biggest cash flow problem for small agencies."),
        ("positioning", "Ledgerly is the invoicing tool for agencies that bill by milestone, not by the hour."),
        ("opinions", "We think time tracking is the wrong basis for agency invoicing. Agencies should bill for outcomes and milestones, never for hours."),
        ("competitors", "Buyers compare us with FreshBooks and Harvest for agency invoicing. Unlike Harvest, Ledgerly does not require time tracking before agencies can send invoices."),
        ("stories", "One design agency cut their invoicing time from eight hours to forty minutes a month after switching to Ledgerly."),
        ("avoid", "Never call the product cheap or promise guaranteed payment times to agency customers."),
    ]
}

/// Creates a site, answers the whole interview and approves every proposed entry.
pub async fn onboarded_site(app: &TestApp) -> String {
    let site = app
        .post("/api/sites", json!({ "name": "Ledgerly", "product_description": "Milestone invoicing for agencies.", "audience": "Small agencies", "website_url": "https://ledgerly.test" }))
        .await;
    let site_id = site["id"].as_str().unwrap().to_string();
    let questions = app.get(&format!("/api/sites/{site_id}/questions?open=true")).await;
    let answers = answers();
    for q in questions["questions"].as_array().unwrap() {
        let topic = q["topic"].as_str().unwrap();
        let answer = answers.iter().find(|(t, _)| *t == topic).map(|(_, a)| *a).unwrap();
        app.post(&format!("/api/questions/{}/answer", q["id"].as_str().unwrap()), json!({ "answer": answer })).await;
    }
    app.run_jobs().await;
    approve_all_entries(app, &site_id).await;
    site_id
}

pub async fn approve_all_entries(app: &TestApp, site_id: &str) -> usize {
    let proposed = app.get(&format!("/api/sites/{site_id}/entries?status=proposed")).await;
    let ids: Vec<Value> = proposed["entries"].as_array().unwrap().iter().map(|e| e["id"].clone()).collect();
    let n = ids.len();
    app.post(&format!("/api/sites/{site_id}/entries/approve"), json!({ "ids": ids })).await;
    app.run_jobs().await;
    n
}

/// Onboards a site, then proposes and approves its topic map.
pub async fn site_with_topics(app: &TestApp) -> String {
    let site_id = onboarded_site(app).await;
    app.post(&format!("/api/sites/{site_id}/topics/propose"), json!({})).await;
    app.run_jobs().await;
    app.post(&format!("/api/sites/{site_id}/topics/approve"), json!({})).await;
    app.run_jobs().await;
    site_id
}

/// Plans and approves a calendar, then advances to the first date and drafts it.
pub async fn first_draft(app: &TestApp, site_id: &str) -> (Value, Value) {
    app.post(&format!("/api/sites/{site_id}/calendar/propose"), json!({})).await;
    app.run_jobs().await;
    app.post(&format!("/api/sites/{site_id}/calendar/approve"), json!({})).await;
    let cal = app.get(&format!("/api/sites/{site_id}/calendar")).await;
    let first = cal["pieces"][0].clone();
    let date = chrono::NaiveDate::parse_from_str(first["scheduled_for"].as_str().unwrap(), "%Y-%m-%d").unwrap();
    app.advance_to(Utc.from_utc_datetime(&date.and_hms_opt(6, 0, 0).unwrap())).await;
    let posts = app.get(&format!("/api/sites/{site_id}/posts?status=in_review")).await;
    let post = posts["posts"].as_array().unwrap().first().cloned().unwrap_or(Value::Null);
    (first, post)
}
