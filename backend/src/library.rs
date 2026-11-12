//! The knowledge library: atomic entries with sources, versions and approval gates.

use crate::error::{AppError, AppResult};
use crate::jobs::{enqueue, enqueue_once};
use crate::models::{Entry, Site};
use crate::state::AppState;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

/// Who may approve a proposed change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Gate {
    /// Housekeeping that adds no new claim; applied by rule.
    Rule,
    /// Checkable against a source the site controls; approved by a separate verifier call.
    Ai,
    /// Everything else; a person decides.
    Person,
}

impl Gate {
    pub fn as_str(&self) -> &'static str {
        match self {
            Gate::Rule => "rule",
            Gate::Ai => "ai",
            Gate::Person => "person",
        }
    }
}

/// Source kinds the site itself controls.
pub fn is_controlled_source(source_kind: &str) -> bool {
    matches!(source_kind, "crawl" | "upload")
}

/// Facts that may be machine-verified against a quoted passage.
pub fn is_checkable_kind(kind: &str) -> bool {
    matches!(kind, "fact" | "feature")
}

/// Decides which gate a proposed entry must pass. Pure, so the policy is easy to test.
pub fn gate_for(kind: &str, source_kind: &str, has_quote: bool, strict: bool, replaces_person_approved: bool) -> (Gate, &'static str) {
    if strict {
        return (Gate::Person, "this site requires a person to approve every change");
    }
    if replaces_person_approved {
        return (Gate::Person, "it would change an entry a person approved");
    }
    if !is_controlled_source(source_kind) {
        return (Gate::Person, match source_kind {
            "third_party" => "it comes from a third-party source",
            "draft_edit" => "it was inferred from your edits",
            _ => "only a person can approve opinions, positioning and anything said on your behalf",
        });
    }
    if !is_checkable_kind(kind) {
        return (Gate::Person, "only a person can approve opinions, positioning, audience and stories");
    }
    if !has_quote {
        return (Gate::Person, "there is no quoted passage to check it against");
    }
    (Gate::Ai, "a product fact from your own site with a quoted passage")
}

fn normalise(body: &str) -> String {
    body.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches('.').to_string()
}

pub async fn load_site(state: &AppState, org_id: Uuid, site_id: Uuid) -> AppResult<Site> {
    sqlx::query_as::<_, Site>("select * from sites where id = $1 and org_id = $2")
        .bind(site_id)
        .bind(org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("site"))
}

pub async fn site_by_id(state: &AppState, site_id: Uuid) -> Result<Site> {
    Ok(sqlx::query_as::<_, Site>("select * from sites where id = $1").bind(site_id).fetch_one(&state.db).await?)
}

pub async fn load_entry(state: &AppState, org_id: Uuid, entry_id: Uuid) -> AppResult<Entry> {
    sqlx::query_as::<_, Entry>("select * from entries where id = $1 and org_id = $2")
        .bind(entry_id)
        .bind(org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("entry"))
}

/// Records a piece of source text, split into passages that entries can quote.
pub async fn create_source(state: &AppState, site: &Site, kind: &str, reference: &str, title: &str, passages: &[String]) -> Result<(Uuid, Vec<Uuid>)> {
    let source_id = Uuid::new_v4();
    sqlx::query("insert into sources (id, org_id, site_id, kind, reference, title, created_at) values ($1, $2, $3, $4, $5, $6, $7)")
        .bind(source_id)
        .bind(site.org_id)
        .bind(site.id)
        .bind(kind)
        .bind(reference)
        .bind(title)
        .bind(state.clock.now())
        .execute(&state.db)
        .await?;
    let mut ids = Vec::new();
    for (i, text) in passages.iter().enumerate() {
        let id = Uuid::new_v4();
        sqlx::query("insert into passages (id, org_id, site_id, source_id, ordinal, text) values ($1, $2, $3, $4, $5, $6)")
            .bind(id)
            .bind(site.org_id)
            .bind(site.id)
            .bind(source_id)
            .bind(i as i32)
            .bind(text)
            .execute(&state.db)
            .await?;
        ids.push(id);
    }
    Ok((source_id, ids))
}

pub struct NewEntry {
    pub kind: String,
    pub body: String,
    pub source_id: Option<Uuid>,
    pub source_kind: String,
    pub passage_id: Option<Uuid>,
    pub quote: String,
    pub replaces: Option<Uuid>,
}

async fn insert_version(tx: &mut sqlx::PgConnection, entry: &Entry, changed_by: &str, at: chrono::DateTime<chrono::Utc>) -> Result<()> {
    sqlx::query(
        "insert into entry_versions (id, entry_id, version, kind, body, status, approved_by, changed_by, changed_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(Uuid::new_v4())
    .bind(entry.id)
    .bind(entry.version)
    .bind(&entry.kind)
    .bind(&entry.body)
    .bind(&entry.status)
    .bind(&entry.approved_by)
    .bind(changed_by)
    .bind(at)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn insert_entry(
    state: &AppState,
    site: &Site,
    kind: &str,
    body: &str,
    status: &str,
    approved_by: Option<&str>,
    gate: Gate,
    gate_reason: &str,
    replaces: Option<Uuid>,
    changed_by: &str,
) -> Result<Entry> {
    let now = state.clock.now();
    let mut tx = state.db.begin().await?;
    sqlx::query("select pg_advisory_xact_lock(hashtext($1::text))").bind(site.id).execute(&mut *tx).await?;
    let num: i32 = sqlx::query_scalar("select coalesce(max(num), 0) + 1 from entries where site_id = $1")
        .bind(site.id)
        .fetch_one(&mut *tx)
        .await?;
    let entry = sqlx::query_as::<_, Entry>(
        "insert into entries (id, org_id, site_id, num, kind, body, status, approved_by, approved_at, gate, gate_reason, replaces_entry_id, created_at, updated_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8, case when $8::text is null then null else $9 end, $10, $11, $12, $9, $9) returning *",
    )
    .bind(Uuid::new_v4())
    .bind(site.org_id)
    .bind(site.id)
    .bind(num)
    .bind(kind)
    .bind(body)
    .bind(status)
    .bind(approved_by)
    .bind(now)
    .bind(gate.as_str())
    .bind(gate_reason)
    .bind(replaces)
    .fetch_one(&mut *tx)
    .await?;
    insert_version(&mut tx, &entry, changed_by, now).await?;
    tx.commit().await?;
    Ok(entry)
}

/// Proposes an entry from a source. Exact duplicates are dropped by rule. Returns the new
/// entry, or `None` if it duplicated an existing one.
pub async fn propose_entry(state: &AppState, site: &Site, new: NewEntry) -> Result<Option<Entry>> {
    if !crate::models::ENTRY_KINDS.contains(&new.kind.as_str()) || new.body.trim().is_empty() {
        return Ok(None);
    }
    let body = crate::text::truncate_words(new.body.trim(), 80);
    let existing: Vec<(Uuid, String)> = sqlx::query_as("select id, body from entries where site_id = $1 and status in ('proposed', 'approved')")
        .bind(site.id)
        .fetch_all(&state.db)
        .await?;
    if let Some((dup_id, _)) = existing.iter().find(|(_, b)| normalise(b) == normalise(&body)) {
        // Rule gate: a duplicate adds no new claim, but the new source still supports the old entry.
        if let Some(source_id) = new.source_id {
            sqlx::query("insert into entry_sources (entry_id, source_id, passage_id, quote) values ($1, $2, $3, $4) on conflict do nothing")
                .bind(dup_id)
                .bind(source_id)
                .bind(new.passage_id)
                .bind(&new.quote)
                .execute(&state.db)
                .await?;
        }
        return Ok(None);
    }
    let replaces_person = match new.replaces {
        Some(id) => sqlx::query_scalar::<_, Option<String>>("select approved_by from entries where id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?
            .flatten()
            .as_deref()
            == Some("person"),
        None => false,
    };
    let (gate, reason) = gate_for(&new.kind, &new.source_kind, !new.quote.trim().is_empty(), site.strict_gates, replaces_person);
    let entry = insert_entry(state, site, &new.kind, &body, "proposed", None, gate, reason, new.replaces, "ai").await?;
    if let Some(source_id) = new.source_id {
        sqlx::query("insert into entry_sources (entry_id, source_id, passage_id, quote) values ($1, $2, $3, $4) on conflict do nothing")
            .bind(entry.id)
            .bind(source_id)
            .bind(new.passage_id)
            .bind(&new.quote)
            .execute(&state.db)
            .await?;
    }
    enqueue(state, Some(site.org_id), Some(site.id), "detect_conflicts", json!({ "entry_id": entry.id })).await?;
    if gate == Gate::Ai {
        enqueue(state, Some(site.org_id), Some(site.id), "verify_entry", json!({ "entry_id": entry.id })).await?;
    }
    Ok(Some(entry))
}

/// A person adds an entry directly; it is approved by that person.
pub async fn add_manual_entry(state: &AppState, site: &Site, kind: &str, body: &str) -> AppResult<Entry> {
    if !crate::models::ENTRY_KINDS.contains(&kind) {
        return Err(AppError::bad_request(format!("unknown entry kind {kind}")));
    }
    if body.trim().is_empty() {
        return Err(AppError::bad_request("entry body is empty"));
    }
    if crate::text::word_count(body) > 120 {
        return Err(AppError::bad_request("an entry should be one short claim (under about 80 words)"));
    }
    let entry = insert_entry(state, site, kind, body.trim(), "approved", Some("person"), Gate::Person, "added by a person", None, "person").await?;
    let (source_id, _) = create_source(state, site, "manual", "", "Added by hand", &[]).await?;
    sqlx::query("insert into entry_sources (entry_id, source_id, quote) values ($1, $2, '')")
        .bind(entry.id)
        .bind(source_id)
        .execute(&state.db)
        .await?;
    after_approval(state, &entry).await?;
    Ok(entry)
}

/// Approves a proposed entry. AI and rule approvals can never touch a person-approved entry.
pub async fn approve(state: &AppState, entry: &Entry, by: Gate, changed_by: &str) -> AppResult<Entry> {
    if entry.status != "proposed" {
        return Err(AppError::conflict(format!("entry E{} is already {}", entry.num, entry.status)));
    }
    if by != Gate::Person && entry.gate == "person" {
        return Err(AppError::forbidden("this change needs a person to approve it"));
    }
    if let Some(target) = entry.replaces_entry_id {
        let target_by: Option<String> = sqlx::query_scalar("select approved_by from entries where id = $1").bind(target).fetch_one(&state.db).await?;
        if by != Gate::Person && target_by.as_deref() == Some("person") {
            return Err(AppError::forbidden("AI cannot replace an entry a person approved"));
        }
    }
    let now = state.clock.now();
    let mut tx = state.db.begin().await?;
    let updated = sqlx::query_as::<_, Entry>(
        "update entries set status = 'approved', approved_by = $2, approved_at = $3, version = version + 1, updated_at = $3
         where id = $1 and status = 'proposed' returning *",
    )
    .bind(entry.id)
    .bind(by.as_str())
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::conflict("entry changed while approving"))?;
    insert_version(&mut tx, &updated, changed_by, now).await?;
    if let Some(target) = entry.replaces_entry_id {
        let old = sqlx::query_as::<_, Entry>("update entries set status = 'superseded', version = version + 1, updated_at = $2 where id = $1 returning *")
            .bind(target)
            .bind(now)
            .fetch_one(&mut *tx)
            .await?;
        insert_version(&mut tx, &old, changed_by, now).await?;
    }
    tx.commit().await?;
    after_approval(state, &updated).await?;
    Ok(updated)
}

pub async fn reject(state: &AppState, entry: &Entry, changed_by: &str) -> AppResult<Entry> {
    if entry.status != "proposed" {
        return Err(AppError::conflict(format!("entry E{} is already {}", entry.num, entry.status)));
    }
    let now = state.clock.now();
    let mut tx = state.db.begin().await?;
    let updated = sqlx::query_as::<_, Entry>("update entries set status = 'rejected', version = version + 1, updated_at = $2 where id = $1 returning *")
        .bind(entry.id)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
    insert_version(&mut tx, &updated, changed_by, now).await?;
    tx.commit().await?;
    Ok(updated)
}

/// A person edits an entry. The edit is approved by that person.
pub async fn edit(state: &AppState, entry: &Entry, kind: Option<&str>, body: Option<&str>) -> AppResult<Entry> {
    let kind = kind.unwrap_or(&entry.kind);
    if !crate::models::ENTRY_KINDS.contains(&kind) {
        return Err(AppError::bad_request(format!("unknown entry kind {kind}")));
    }
    let body = body.map(|b| b.trim()).unwrap_or(&entry.body);
    if body.is_empty() {
        return Err(AppError::bad_request("entry body is empty"));
    }
    let now = state.clock.now();
    let status = if entry.status == "proposed" { "approved" } else { entry.status.as_str() };
    let mut tx = state.db.begin().await?;
    let updated = sqlx::query_as::<_, Entry>(
        "update entries set kind = $2, body = $3, status = $4, approved_by = 'person', approved_at = $5, gate = 'person', version = version + 1, updated_at = $5
         where id = $1 returning *",
    )
    .bind(entry.id)
    .bind(kind)
    .bind(body)
    .bind(status)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    insert_version(&mut tx, &updated, "person", now).await?;
    tx.commit().await?;
    if updated.status == "approved" {
        after_approval(state, &updated).await?;
    }
    Ok(updated)
}

/// Restores an earlier version as a new version.
pub async fn revert(state: &AppState, entry: &Entry, version: i32) -> AppResult<Entry> {
    let old = sqlx::query_as::<_, crate::models::EntryVersion>("select * from entry_versions where entry_id = $1 and version = $2")
        .bind(entry.id)
        .bind(version)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("version"))?;
    let now = state.clock.now();
    let mut tx = state.db.begin().await?;
    let updated = sqlx::query_as::<_, Entry>(
        "update entries set kind = $2, body = $3, status = $4, approved_by = case when $4 = 'approved' then 'person' else approved_by end,
         version = version + 1, updated_at = $5 where id = $1 returning *",
    )
    .bind(entry.id)
    .bind(&old.kind)
    .bind(&old.body)
    .bind(&old.status)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    insert_version(&mut tx, &updated, "person", now).await?;
    tx.commit().await?;
    refresh_profile(state, updated.site_id).await?;
    Ok(updated)
}

/// Work that follows any approval: tagging, graph extraction, the profile, and held pieces.
async fn after_approval(state: &AppState, entry: &Entry) -> Result<()> {
    refresh_profile(state, entry.site_id).await?;
    enqueue(state, Some(entry.org_id), Some(entry.site_id), "entry_approved", json!({ "entry_id": entry.id })).await?;
    enqueue_once(state, Some(entry.org_id), Some(entry.site_id), "recheck_held", json!({ "site_id": entry.site_id })).await?;
    Ok(())
}

/// Recompiles the site profile from the core approved entries. It is deterministic so the
/// text stays identical between calls and can be cached by the model provider.
pub async fn refresh_profile(state: &AppState, site_id: Uuid) -> Result<String> {
    let site = site_by_id(state, site_id).await?;
    let entries = sqlx::query_as::<_, Entry>(
        "select * from entries where site_id = $1 and status = 'approved' and kind in ('positioning', 'audience', 'avoid', 'feature', 'opinion')
         order by num",
    )
    .bind(site_id)
    .fetch_all(&state.db)
    .await?;
    let mut md = format!("# {}\n\n", site.name);
    if !site.product_description.is_empty() {
        md.push_str(&format!("{}\n\n", site.product_description.trim()));
    }
    if !site.audience.is_empty() {
        md.push_str(&format!("Audience: {}\n\n", site.audience.trim()));
    }
    let sections = [
        ("positioning", "Positioning", 8),
        ("audience", "Audience", 6),
        ("feature", "Main features", 10),
        ("opinion", "Opinions", 8),
        ("avoid", "Phrasing to avoid", 10),
    ];
    for (kind, heading, limit) in sections {
        let items: Vec<&Entry> = entries.iter().filter(|e| e.kind == kind).take(limit).collect();
        if items.is_empty() {
            continue;
        }
        md.push_str(&format!("## {heading}\n\n"));
        for e in items {
            md.push_str(&format!("- [E{}] {}\n", e.num, e.body));
        }
        md.push('\n');
    }
    if md != site.profile_md {
        sqlx::query("update sites set profile_md = $2, profile_updated_at = $3 where id = $1")
            .bind(site_id)
            .bind(&md)
            .bind(state.clock.now())
            .execute(&state.db)
            .await?;
    }
    Ok(md)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opinions_always_need_a_person() {
        assert_eq!(gate_for("opinion", "crawl", true, false, false).0, Gate::Person);
        assert_eq!(gate_for("positioning", "upload", true, false, false).0, Gate::Person);
    }

    #[test]
    fn checkable_facts_from_controlled_sources_go_to_ai() {
        assert_eq!(gate_for("fact", "crawl", true, false, false).0, Gate::Ai);
        assert_eq!(gate_for("feature", "upload", true, false, false).0, Gate::Ai);
    }

    #[test]
    fn facts_need_a_person_without_evidence_or_from_elsewhere() {
        assert_eq!(gate_for("fact", "crawl", false, false, false).0, Gate::Person);
        assert_eq!(gate_for("fact", "third_party", true, false, false).0, Gate::Person);
        assert_eq!(gate_for("fact", "interview", true, false, false).0, Gate::Person);
        assert_eq!(gate_for("fact", "draft_edit", true, false, false).0, Gate::Person);
    }

    #[test]
    fn strict_sites_and_person_approved_targets_need_a_person() {
        assert_eq!(gate_for("fact", "crawl", true, true, false).0, Gate::Person);
        assert_eq!(gate_for("fact", "crawl", true, false, true).0, Gate::Person);
    }
}
