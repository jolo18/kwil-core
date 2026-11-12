use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Site {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub slug: String,
    pub product_description: String,
    pub audience: String,
    pub website_url: Option<String>,
    pub cadence_per_week: i32,
    pub monthly_budget_usd: f64,
    pub drafting_paused: bool,
    pub model_draft: String,
    pub model_mid: String,
    pub model_cheap: String,
    pub review_mode: String,
    pub strict_gates: bool,
    pub author_name: String,
    pub branding: Value,
    pub github_repo: Option<String>,
    pub github_branch: String,
    pub github_folder: String,
    #[serde(skip_serializing)]
    pub github_token: Option<String>,
    pub profile_md: String,
    pub profile_updated_at: Option<DateTime<Utc>>,
    pub last_crawled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

pub const ENTRY_KINDS: &[&str] = &["fact", "feature", "audience", "positioning", "opinion", "story", "quote", "competitor", "avoid"];

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Entry {
    pub id: Uuid,
    pub org_id: Uuid,
    pub site_id: Uuid,
    pub num: i32,
    pub kind: String,
    pub body: String,
    pub status: String,
    pub approved_by: Option<String>,
    pub approved_at: Option<DateTime<Utc>>,
    pub gate: String,
    pub gate_reason: String,
    pub replaces_entry_id: Option<Uuid>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct EntryVersion {
    pub id: Uuid,
    pub entry_id: Uuid,
    pub version: i32,
    pub kind: String,
    pub body: String,
    pub status: String,
    pub approved_by: Option<String>,
    pub changed_by: String,
    pub changed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct EntrySource {
    pub source_id: Uuid,
    pub kind: String,
    pub reference: String,
    pub title: String,
    pub quote: String,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct EntryLink {
    pub id: Uuid,
    pub from_entry: Uuid,
    pub to_entry: Uuid,
    pub kind: String,
    pub status: String,
    pub note: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Topic {
    pub id: Uuid,
    pub org_id: Uuid,
    pub site_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub slug: String,
    pub description: String,
    pub status: String,
    pub position: i32,
    pub created_at: DateTime<Utc>,
}

pub const CONTENT_TYPES: &[&str] = &["how_to", "buyer_guide", "alternatives", "comparison", "listicle", "explainer", "report", "opinion", "case_study"];
/// Content types that make a product company look like a review site when overused.
pub const COMPARISON_TYPES: &[&str] = &["alternatives", "comparison", "listicle"];

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Piece {
    pub id: Uuid,
    pub org_id: Uuid,
    pub site_id: Uuid,
    pub topic_id: Option<Uuid>,
    pub content_type: String,
    pub title: String,
    pub angle: String,
    pub target_keyword: Option<String>,
    pub timeliness: String,
    pub scheduled_for: NaiveDate,
    pub status: String,
    pub hold_reason: String,
    pub brief: Option<Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Question {
    pub id: Uuid,
    pub org_id: Uuid,
    pub site_id: Uuid,
    pub piece_id: Option<Uuid>,
    pub kind: String,
    pub topic: String,
    pub prompt: String,
    pub answer: Option<String>,
    pub answered_at: Option<DateTime<Utc>>,
    pub position: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Post {
    pub id: Uuid,
    pub org_id: Uuid,
    pub site_id: Uuid,
    pub piece_id: Option<Uuid>,
    pub topic_id: Option<Uuid>,
    pub content_type: String,
    pub title: String,
    pub slug: String,
    pub meta_description: String,
    pub body_md: String,
    pub ai_draft_md: String,
    pub status: String,
    pub review_notes: String,
    pub timeliness: String,
    pub author_name: String,
    pub publish_at: Option<DateTime<Utc>>,
    pub published_at: Option<DateTime<Utc>>,
    pub last_updated_at: DateTime<Utc>,
    pub github_committed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct PostRevision {
    pub id: Uuid,
    pub post_id: Uuid,
    pub title: String,
    pub body_md: String,
    pub author: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct FixedPage {
    pub id: Uuid,
    pub org_id: Uuid,
    pub site_id: Uuid,
    pub slug: String,
    pub title: String,
    pub body_md: String,
    pub nav_position: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize, Deserialize)]
pub struct Entity {
    pub id: Uuid,
    pub site_id: Uuid,
    pub kind: String,
    pub name: String,
    pub topic_id: Option<Uuid>,
    pub status: String,
    pub approved_by: Option<String>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Edge {
    pub id: Uuid,
    pub from_entity: Uuid,
    pub to_entity: Uuid,
    pub kind: String,
    pub status: String,
    pub approved_by: Option<String>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Domain {
    pub id: Uuid,
    pub site_id: Uuid,
    pub hostname: String,
    pub status: String,
    pub provider_id: Option<String>,
    pub verification: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Notification {
    pub id: Uuid,
    pub site_id: Option<Uuid>,
    pub kind: String,
    pub message: String,
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Upload {
    pub id: Uuid,
    pub site_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub status: String,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Digest {
    pub id: Uuid,
    pub site_id: Uuid,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub items: Value,
    pub created_at: DateTime<Utc>,
}
