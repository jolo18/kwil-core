//! Background job handlers and the scheduled tick that creates due work.

use crate::ai::llm::Tier;
use crate::ai::{self, Scope};
use crate::jobs::{enqueue, enqueue_once, Job};
use crate::library::{self, create_source, refresh_profile, site_by_id, Gate, NewEntry};
use crate::models::*;
use crate::retrieval;
use crate::state::AppState;
use crate::text::{cited_nums, slugify, strip_citations};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{Datelike, Duration, NaiveDate, TimeZone, Utc, Weekday};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

fn id_from(payload: &Value, key: &str) -> Result<Uuid> {
    payload[key].as_str().and_then(|s| Uuid::parse_str(s).ok()).ok_or_else(|| anyhow!("job payload is missing {key}"))
}

fn scope(site: &Site) -> Scope {
    Scope { org_id: site.org_id, site_id: site.id }
}

pub async fn run_job(state: &AppState, job: &Job) -> Result<Value> {
    let p = &job.payload;
    match job.kind.as_str() {
        "interview_answered" => interview_answered(state, id_from(p, "question_id")?).await,
        "extract_source" => extract_source(state, id_from(p, "source_id")?).await,
        "detect_conflicts" => detect_conflicts(state, id_from(p, "entry_id")?).await,
        "verify_entry" => verify_entry(state, id_from(p, "entry_id")?).await,
        "entry_approved" => entry_approved(state, id_from(p, "entry_id")?).await,
        "retag_site" => retag_site(state, id_from(p, "site_id")?).await,
        "recheck_held" => recheck_held(state, id_from(p, "site_id")?).await,
        "propose_topics" => propose_topics(state, id_from(p, "site_id")?).await,
        "propose_calendar" => propose_calendar(state, id_from(p, "site_id")?).await,
        "draft_piece" => draft_piece(state, id_from(p, "piece_id")?).await,
        "auto_review" => auto_review(state, id_from(p, "post_id")?).await,
        "publish_post" => publish_post(state, id_from(p, "post_id")?).await,
        "github_commit" => github_commit(state, id_from(p, "post_id")?).await,
        "learn_from_edit" => learn_from_edit(state, id_from(p, "post_id")?).await,
        "learn_from_post" => learn_from_post(state, id_from(p, "post_id")?).await,
        "crawl_site" => {
            let site = site_by_id(state, id_from(p, "site_id")?).await?;
            crate::ingest::crawl_site(state, &site).await
        }
        "process_upload" => {
            let upload_id = id_from(p, "upload_id")?;
            let site_id: Uuid = sqlx::query_scalar("select site_id from uploads where id = $1").bind(upload_id).fetch_one(&state.db).await?;
            let site = site_by_id(state, site_id).await?;
            crate::ingest::process_upload(state, &site, upload_id).await
        }
        "weekly_digest" => weekly_digest(state, id_from(p, "site_id")?).await,
        "check_domain" => check_domain(state, id_from(p, "domain_id")?).await,
        other => bail!("unknown job kind {other}"),
    }
}

/// Called when a job has used up its retries.
pub async fn on_job_dead(state: &AppState, job: &Job) -> Result<()> {
    if job.kind == "draft_piece" {
        let piece_id = id_from(&job.payload, "piece_id")?;
        sqlx::query("update pieces set status = 'failed', hold_reason = $2 where id = $1")
            .bind(piece_id)
            .bind(job.last_error.clone().unwrap_or_else(|| "drafting failed".into()))
            .execute(&state.db)
            .await?;
    }
    Ok(())
}

pub async fn notify(state: &AppState, org_id: Uuid, site_id: Option<Uuid>, kind: &str, message: &str) -> Result<()> {
    sqlx::query("insert into notifications (id, org_id, site_id, kind, message, created_at) values ($1, $2, $3, $4, $5, $6)")
        .bind(Uuid::new_v4())
        .bind(org_id)
        .bind(site_id)
        .bind(kind)
        .bind(message)
        .bind(state.clock.now())
        .execute(&state.db)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------- interview

pub const CORE_QUESTIONS: &[(&str, &str)] = &[
    ("product", "What does {site} do, and what makes it different from how people solved this problem before?"),
    ("audience", "Who is it for? Describe your best customer and the moment they realise they need you."),
    ("pain_points", "What problems do customers bring to you, in their own words?"),
    ("positioning", "How do you want to be described? What is the one thing buyers should remember about you?"),
    ("opinions", "What do you believe about your industry that most people in it would disagree with?"),
    ("competitors", "Who do buyers compare you with, and where are you better or worse than them?"),
    ("stories", "Tell a short story about a customer whose results changed after using you."),
    ("avoid", "What words, claims or tones should your content never use?"),
];

pub async fn start_interview(state: &AppState, site: &Site) -> Result<usize> {
    let existing: i64 = sqlx::query_scalar("select count(*) from questions where site_id = $1 and kind = 'core'")
        .bind(site.id)
        .fetch_one(&state.db)
        .await?;
    if existing > 0 {
        return Ok(0);
    }
    for (i, (topic, prompt)) in CORE_QUESTIONS.iter().enumerate() {
        sqlx::query("insert into questions (id, org_id, site_id, kind, topic, prompt, position, created_at) values ($1, $2, $3, 'core', $4, $5, $6, $7)")
            .bind(Uuid::new_v4())
            .bind(site.org_id)
            .bind(site.id)
            .bind(topic)
            .bind(prompt.replace("{site}", &site.name))
            .bind(i as i32)
            .bind(state.clock.now())
            .execute(&state.db)
            .await?;
    }
    Ok(CORE_QUESTIONS.len())
}

#[derive(Deserialize)]
struct ProposedEntries {
    entries: Vec<ProposedEntry>,
}

#[derive(Deserialize)]
struct ProposedEntry {
    kind: String,
    body: String,
    #[serde(default)]
    passage_index: Option<i64>,
    #[serde(default)]
    quote: String,
}

/// Runs entry extraction over a source's passages and proposes the results.
async fn extract_from_passages(state: &AppState, site: &Site, source_id: Uuid, source_kind: &str, topic_hint: &str) -> Result<usize> {
    let passages: Vec<(Uuid, String)> = sqlx::query_as("select id, text from passages where source_id = $1 order by ordinal")
        .bind(source_id)
        .fetch_all(&state.db)
        .await?;
    let mut created = 0;
    for chunk in passages.chunks(8) {
        let ctx = json!({
            "source_kind": source_kind,
            "topic": topic_hint,
            "passages": chunk.iter().enumerate().map(|(i, (_, t))| json!({ "index": i, "text": t })).collect::<Vec<_>>(),
        });
        let out: ProposedEntries = ai::run(state, scope(site), "extract_entries", Tier::Mid, ctx, None).await?;
        for e in out.entries {
            let passage = e.passage_index.and_then(|i| chunk.get(i as usize));
            // Evidence only counts if the quote really is in the passage.
            let quote = match passage {
                Some((_, text)) if !e.quote.trim().is_empty() && text.contains(e.quote.trim()) => e.quote.trim().to_string(),
                _ => String::new(),
            };
            let made = library::propose_entry(
                state,
                site,
                NewEntry {
                    kind: e.kind,
                    body: e.body,
                    source_id: Some(source_id),
                    source_kind: source_kind.to_string(),
                    passage_id: passage.map(|(id, _)| *id),
                    quote,
                    replaces: None,
                },
            )
            .await?;
            if made.is_some() {
                created += 1;
            }
        }
    }
    Ok(created)
}

async fn interview_answered(state: &AppState, question_id: Uuid) -> Result<Value> {
    let q = sqlx::query_as::<_, Question>("select * from questions where id = $1").bind(question_id).fetch_one(&state.db).await?;
    let site = site_by_id(state, q.site_id).await?;
    let answer = q.answer.clone().unwrap_or_default();
    let (source_id, _) = create_source(state, &site, "interview", &q.id.to_string(), &q.prompt, &[answer.clone()]).await?;
    let created = extract_from_passages(state, &site, source_id, "interview", &q.topic).await?;

    if q.kind == "core" {
        let has_followup: bool = sqlx::query_scalar("select exists(select 1 from questions where site_id = $1 and kind = 'followup' and topic = $2)")
            .bind(site.id)
            .bind(&q.topic)
            .fetch_one(&state.db)
            .await?;
        if !has_followup {
            #[derive(Deserialize)]
            struct Followups {
                questions: Vec<FollowupQ>,
            }
            #[derive(Deserialize)]
            struct FollowupQ {
                #[serde(default)]
                topic: String,
                prompt: String,
            }
            let out: Followups = ai::run(
                state,
                scope(&site),
                "interview_followups",
                Tier::Cheap,
                json!({ "answers": [{ "topic": q.topic, "question": q.prompt, "answer": answer }] }),
                None,
            )
            .await?;
            for f in out.questions.into_iter().take(1) {
                sqlx::query("insert into questions (id, org_id, site_id, kind, topic, prompt, position, created_at) values ($1, $2, $3, 'followup', $4, $5, $6, $7)")
                    .bind(Uuid::new_v4())
                    .bind(site.org_id)
                    .bind(site.id)
                    .bind(if f.topic.is_empty() { q.topic.clone() } else { f.topic })
                    .bind(f.prompt)
                    .bind(q.position * 10 + 1)
                    .bind(state.clock.now())
                    .execute(&state.db)
                    .await?;
            }
        }
    }
    if q.kind == "coverage" {
        if let Some(piece_id) = q.piece_id {
            let open: i64 = sqlx::query_scalar("select count(*) from questions where piece_id = $1 and answer is null")
                .bind(piece_id)
                .fetch_one(&state.db)
                .await?;
            if open == 0 {
                sqlx::query("update pieces set hold_reason = 'Waiting for you to approve the entries from your answers.' where id = $1 and status = 'held'")
                    .bind(piece_id)
                    .execute(&state.db)
                    .await?;
            }
        }
    }
    Ok(json!({ "proposed_entries": created }))
}

async fn extract_source(state: &AppState, source_id: Uuid) -> Result<Value> {
    let (site_id, kind): (Uuid, String) = sqlx::query_as("select site_id, kind from sources where id = $1").bind(source_id).fetch_one(&state.db).await?;
    let site = site_by_id(state, site_id).await?;
    let created = extract_from_passages(state, &site, source_id, &kind, "").await?;
    Ok(json!({ "proposed_entries": created }))
}

// ---------------------------------------------------------------- gates and conflicts

async fn similar_entries(state: &AppState, entry: &Entry, limit: i64) -> Result<Vec<Entry>> {
    let words: Vec<String> = crate::text::significant_words(&entry.body).into_iter().filter(|w| w.chars().all(|c| c.is_alphanumeric())).collect();
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let query = words.join(" | ");
    Ok(sqlx::query_as::<_, Entry>(
        "select * from entries where site_id = $1 and id <> $2 and status = 'approved' and search @@ to_tsquery('english', $3)
         order by ts_rank(search, to_tsquery('english', $3)) desc limit $4",
    )
    .bind(entry.site_id)
    .bind(entry.id)
    .bind(&query)
    .bind(limit)
    .fetch_all(&state.db)
    .await?)
}

async fn detect_conflicts(state: &AppState, entry_id: Uuid) -> Result<Value> {
    let entry = sqlx::query_as::<_, Entry>("select * from entries where id = $1").bind(entry_id).fetch_one(&state.db).await?;
    if entry.status != "proposed" {
        return Ok(json!({ "skipped": "no longer proposed" }));
    }
    let candidates = similar_entries(state, &entry, 5).await?;
    if candidates.is_empty() {
        return Ok(json!({ "links": 0 }));
    }
    let site = site_by_id(state, entry.site_id).await?;
    #[derive(Deserialize)]
    struct Links {
        links: Vec<LinkOut>,
    }
    #[derive(Deserialize)]
    struct LinkOut {
        to_num: i64,
        kind: String,
        #[serde(default)]
        note: String,
    }
    let ctx = json!({
        "entry": { "num": entry.num, "kind": entry.kind, "body": entry.body },
        "candidates": candidates.iter().map(|c| json!({ "num": c.num, "kind": c.kind, "body": c.body })).collect::<Vec<_>>(),
    });
    let out: Links = ai::run(state, scope(&site), "detect_conflicts", Tier::Mid, ctx, None).await?;
    let mut made = 0;
    let mut contradicts = Vec::new();
    for l in out.links {
        let Some(target) = candidates.iter().find(|c| c.num as i64 == l.to_num) else { continue };
        if !["refines", "contradicts", "replaces"].contains(&l.kind.as_str()) {
            continue;
        }
        sqlx::query(
            "insert into entry_links (id, org_id, site_id, from_entry, to_entry, kind, note, created_at) values ($1, $2, $3, $4, $5, $6, $7, $8)
             on conflict do nothing",
        )
        .bind(Uuid::new_v4())
        .bind(entry.org_id)
        .bind(entry.site_id)
        .bind(entry.id)
        .bind(target.id)
        .bind(&l.kind)
        .bind(&l.note)
        .bind(state.clock.now())
        .execute(&state.db)
        .await?;
        made += 1;
        if l.kind == "contradicts" || l.kind == "replaces" {
            contradicts.push(target.num);
        }
    }
    if !contradicts.is_empty() {
        let reason = format!(
            "it conflicts with approved {}",
            contradicts.iter().map(|n| format!("E{n}")).collect::<Vec<_>>().join(", ")
        );
        sqlx::query("update entries set gate = 'person', gate_reason = $2 where id = $1 and status = 'proposed'")
            .bind(entry.id)
            .bind(reason)
            .execute(&state.db)
            .await?;
    }
    Ok(json!({ "links": made }))
}

async fn verify_entry(state: &AppState, entry_id: Uuid) -> Result<Value> {
    // Conflict detection runs first so a contradiction can move the entry to a person.
    let pending_conflict_check: bool = sqlx::query_scalar(
        "select exists(select 1 from jobs where kind = 'detect_conflicts' and payload->>'entry_id' = $1 and status in ('queued', 'running', 'failed'))",
    )
    .bind(entry_id.to_string())
    .fetch_one(&state.db)
    .await?;
    if pending_conflict_check {
        bail!("waiting for the conflict check to finish");
    }
    let entry = sqlx::query_as::<_, Entry>("select * from entries where id = $1").bind(entry_id).fetch_one(&state.db).await?;
    if entry.status != "proposed" || entry.gate != "ai" {
        return Ok(json!({ "skipped": "not waiting on the AI gate" }));
    }
    let quote: Option<String> = sqlx::query_scalar("select quote from entry_sources where entry_id = $1 and quote <> '' limit 1")
        .bind(entry.id)
        .fetch_optional(&state.db)
        .await?;
    let site = site_by_id(state, entry.site_id).await?;
    let Some(quote) = quote else {
        sqlx::query("update entries set gate = 'person', gate_reason = 'there is no quoted passage to check it against' where id = $1")
            .bind(entry.id)
            .execute(&state.db)
            .await?;
        return Ok(json!({ "supported": false }));
    };
    #[derive(Deserialize)]
    struct Verdict {
        supported: bool,
        #[serde(default)]
        reason: String,
    }
    let v: Verdict = ai::run(state, scope(&site), "verify_claim", Tier::Mid, json!({ "claim": entry.body, "quote": quote }), None).await?;
    if v.supported {
        library::approve(state, &entry, Gate::Ai, "ai-verifier").await.map_err(|e| anyhow!(e.message))?;
    } else {
        sqlx::query("update entries set gate = 'person', gate_reason = $2 where id = $1")
            .bind(entry.id)
            .bind(format!("the verifier could not confirm it from the quoted passage: {}", v.reason))
            .execute(&state.db)
            .await?;
    }
    Ok(json!({ "supported": v.supported, "reason": v.reason }))
}

// ---------------------------------------------------------------- topics, tagging and the graph

#[derive(Deserialize)]
struct Tags {
    tags: Vec<TagOut>,
}

#[derive(Deserialize)]
struct TagOut {
    entry_num: i64,
    topic_id: String,
}

async fn tag_entries(state: &AppState, site: &Site, entries: &[Entry]) -> Result<usize> {
    let topics = sqlx::query_as::<_, Topic>("select * from topics where site_id = $1 and status = 'approved'").bind(site.id).fetch_all(&state.db).await?;
    if topics.is_empty() || entries.is_empty() {
        return Ok(0);
    }
    let mut made = 0;
    for chunk in entries.chunks(25) {
        let ctx = json!({
            "entries": chunk.iter().map(|e| json!({ "num": e.num, "body": e.body })).collect::<Vec<_>>(),
            "topics": topics.iter().map(|t| json!({ "id": t.id, "title": t.title, "description": t.description })).collect::<Vec<_>>(),
        });
        let out: Tags = ai::run(state, scope(site), "tag_topics", Tier::Cheap, ctx, None).await?;
        for t in out.tags {
            let Some(entry) = chunk.iter().find(|e| e.num as i64 == t.entry_num) else { continue };
            let Some(topic) = topics.iter().find(|x| x.id.to_string() == t.topic_id) else { continue };
            sqlx::query("insert into entry_topics (entry_id, topic_id) values ($1, $2) on conflict do nothing")
                .bind(entry.id)
                .bind(topic.id)
                .execute(&state.db)
                .await?;
            made += 1;
        }
    }
    Ok(made)
}

#[derive(Deserialize, Clone)]
struct EntityRef {
    kind: String,
    name: String,
}

#[derive(Deserialize)]
struct GraphOut {
    #[serde(default)]
    entities: Vec<EntityRef>,
    #[serde(default)]
    edges: Vec<EdgeOut>,
    #[serde(default)]
    about: Vec<AboutOut>,
}

#[derive(Deserialize)]
struct EdgeOut {
    from: EntityRef,
    to: EntityRef,
    kind: String,
}

#[derive(Deserialize)]
struct AboutOut {
    entry_num: i64,
    entity: EntityRef,
}

const ENTITY_KINDS: &[&str] = &["product", "feature", "persona", "use_case", "competitor", "integration", "topic"];
const EDGE_KINDS: &[&str] = &["has_feature", "serves", "competes_with", "alternative_to", "integrates_with", "part_of_topic"];

/// Finds an entity by kind and name, creating it if needed. A new entity is approved by the
/// AI gate when its name appears in the approved entry it came from; otherwise a person decides.
async fn upsert_entity(state: &AppState, site: &Site, r: &EntityRef, evidence: &str) -> Result<Option<Entity>> {
    let name = r.name.trim();
    if name.is_empty() || name.len() > 120 || !ENTITY_KINDS.contains(&r.kind.as_str()) {
        return Ok(None);
    }
    if let Some(e) = sqlx::query_as::<_, Entity>("select * from entities where site_id = $1 and kind = $2 and lower(name) = lower($3) and status <> 'rejected'")
        .bind(site.id)
        .bind(&r.kind)
        .bind(name)
        .fetch_optional(&state.db)
        .await?
    {
        return Ok(Some(e));
    }
    if r.kind == "topic" {
        // Topic entities only come from the approved topic map.
        return Ok(None);
    }
    let verified = r.kind == "product" && name.eq_ignore_ascii_case(&site.name) || evidence.to_lowercase().contains(&name.to_lowercase());
    let (status, by) = if verified && !site.strict_gates { ("approved", Some("ai")) } else { ("proposed", None) };
    let e = sqlx::query_as::<_, Entity>(
        "insert into entities (id, org_id, site_id, kind, name, status, approved_by, created_at) values ($1, $2, $3, $4, $5, $6, $7, $8) returning *",
    )
    .bind(Uuid::new_v4())
    .bind(site.org_id)
    .bind(site.id)
    .bind(&r.kind)
    .bind(name)
    .bind(status)
    .bind(by)
    .bind(state.clock.now())
    .fetch_one(&state.db)
    .await?;
    Ok(Some(e))
}

async fn extract_graph(state: &AppState, site: &Site, entries: &[Entry]) -> Result<Value> {
    let known: Vec<(String, String)> = sqlx::query_as("select kind, name from entities where site_id = $1 and status = 'approved'")
        .bind(site.id)
        .fetch_all(&state.db)
        .await?;
    let topics: Vec<String> = sqlx::query_scalar("select title from topics where site_id = $1 and status = 'approved'").bind(site.id).fetch_all(&state.db).await?;
    let ctx = json!({
        "site_name": site.name,
        "entries": entries.iter().map(|e| json!({ "num": e.num, "kind": e.kind, "body": e.body })).collect::<Vec<_>>(),
        "entities": known.iter().map(|(k, n)| json!({ "kind": k, "name": n })).collect::<Vec<_>>(),
        "topics": topics.iter().map(|t| json!({ "title": t })).collect::<Vec<_>>(),
    });
    let out: GraphOut = ai::run(state, scope(site), "extract_graph", Tier::Mid, ctx, None).await?;
    let evidence = format!("{} {}", site.name, entries.iter().map(|e| e.body.as_str()).collect::<Vec<_>>().join(" "));
    for r in &out.entities {
        upsert_entity(state, site, r, &evidence).await?;
    }
    let mut edges = 0;
    for e in &out.edges {
        if !EDGE_KINDS.contains(&e.kind.as_str()) {
            continue;
        }
        let (Some(from), Some(to)) = (upsert_entity(state, site, &e.from, &evidence).await?, upsert_entity(state, site, &e.to, &evidence).await?) else {
            continue;
        };
        if from.id == to.id {
            continue;
        }
        // Who competes with whom is positioning: always a person's call.
        let auto = e.kind != "competes_with" && from.status == "approved" && to.status == "approved" && !site.strict_gates;
        let (status, by) = if auto { ("approved", Some("ai")) } else { ("proposed", None) };
        let res = sqlx::query(
            "insert into entity_edges (id, org_id, site_id, from_entity, to_entity, kind, status, approved_by, created_at)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9) on conflict do nothing",
        )
        .bind(Uuid::new_v4())
        .bind(site.org_id)
        .bind(site.id)
        .bind(from.id)
        .bind(to.id)
        .bind(&e.kind)
        .bind(status)
        .bind(by)
        .bind(state.clock.now())
        .execute(&state.db)
        .await?;
        edges += res.rows_affected();
    }
    let mut about = 0;
    for a in &out.about {
        let Some(entry) = entries.iter().find(|e| e.num as i64 == a.entry_num) else { continue };
        let Some(entity) = upsert_entity(state, site, &a.entity, &evidence).await? else { continue };
        sqlx::query("insert into entry_entities (entry_id, entity_id) values ($1, $2) on conflict do nothing")
            .bind(entry.id)
            .bind(entity.id)
            .execute(&state.db)
            .await?;
        about += 1;
    }
    Ok(json!({ "entities": out.entities.len(), "edges": edges, "about": about }))
}

async fn entry_approved(state: &AppState, entry_id: Uuid) -> Result<Value> {
    let entry = sqlx::query_as::<_, Entry>("select * from entries where id = $1").bind(entry_id).fetch_one(&state.db).await?;
    if entry.status != "approved" {
        return Ok(json!({ "skipped": true }));
    }
    let site = site_by_id(state, entry.site_id).await?;
    let tagged = tag_entries(state, &site, std::slice::from_ref(&entry)).await?;
    let graph = extract_graph(state, &site, std::slice::from_ref(&entry)).await?;
    Ok(json!({ "tagged": tagged, "graph": graph }))
}

async fn retag_site(state: &AppState, site_id: Uuid) -> Result<Value> {
    let site = site_by_id(state, site_id).await?;
    let entries = sqlx::query_as::<_, Entry>("select * from entries where site_id = $1 and status = 'approved' order by num").bind(site_id).fetch_all(&state.db).await?;
    let tagged = tag_entries(state, &site, &entries).await?;
    Ok(json!({ "tagged": tagged }))
}

/// Makes a topic entity for an approved topic so the graph can start from it.
pub async fn ensure_topic_entity(state: &AppState, topic: &Topic) -> Result<()> {
    sqlx::query(
        "insert into entities (id, org_id, site_id, kind, name, topic_id, status, approved_by, created_at)
         values ($1, $2, $3, 'topic', $4, $5, 'approved', 'rule', $6) on conflict do nothing",
    )
    .bind(Uuid::new_v4())
    .bind(topic.org_id)
    .bind(topic.site_id)
    .bind(&topic.title)
    .bind(topic.id)
    .bind(state.clock.now())
    .execute(&state.db)
    .await?;
    Ok(())
}

#[derive(Deserialize)]
struct TopicsOut {
    topics: Vec<TopicOut>,
}

#[derive(Deserialize)]
struct TopicOut {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    children: Vec<TopicOut>,
}

pub async fn unique_topic_slug(state: &AppState, site_id: Uuid, title: &str) -> Result<String> {
    let base = slugify(title);
    let mut slug = base.clone();
    let mut n = 2;
    loop {
        let taken: bool = sqlx::query_scalar("select exists(select 1 from topics where site_id = $1 and slug = $2)")
            .bind(site_id)
            .bind(&slug)
            .fetch_one(&state.db)
            .await?;
        if !taken {
            return Ok(slug);
        }
        slug = format!("{base}-{n}");
        n += 1;
    }
}

async fn insert_topic(state: &AppState, site: &Site, parent: Option<Uuid>, title: &str, description: &str, position: i32) -> Result<Uuid> {
    let id = Uuid::new_v4();
    let slug = unique_topic_slug(state, site.id, title).await?;
    sqlx::query(
        "insert into topics (id, org_id, site_id, parent_id, title, slug, description, status, position, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, 'proposed', $8, $9)",
    )
    .bind(id)
    .bind(site.org_id)
    .bind(site.id)
    .bind(parent)
    .bind(title)
    .bind(slug)
    .bind(description)
    .bind(position)
    .bind(state.clock.now())
    .execute(&state.db)
    .await?;
    Ok(id)
}

async fn propose_topics(state: &AppState, site_id: Uuid) -> Result<Value> {
    let site = site_by_id(state, site_id).await?;
    let entries = sqlx::query_as::<_, Entry>("select * from entries where site_id = $1 and status in ('approved', 'proposed') order by num limit 200")
        .bind(site_id)
        .fetch_all(&state.db)
        .await?;
    if entries.is_empty() {
        bail!("the library is empty; answer the interview first");
    }
    let existing: Vec<String> = sqlx::query_scalar("select title from topics where site_id = $1 and status = 'approved'").bind(site_id).fetch_all(&state.db).await?;
    let profile = refresh_profile(state, site_id).await?;
    let ctx = json!({
        "site_name": site.name,
        "profile": profile,
        "existing_topics": existing,
        "entries": entries.iter().map(|e| json!({ "num": e.num, "kind": e.kind, "body": e.body })).collect::<Vec<_>>(),
    });
    let out: TopicsOut = ai::run(state, scope(&site), "propose_topics", Tier::Mid, ctx, None).await?;
    sqlx::query("delete from topics where site_id = $1 and status = 'proposed'").bind(site_id).execute(&state.db).await?;
    let mut count = 0;
    for (i, t) in out.topics.iter().enumerate() {
        if existing.iter().any(|e| e.eq_ignore_ascii_case(&t.title)) {
            continue;
        }
        let pid = insert_topic(state, &site, None, &t.title, &t.description, i as i32).await?;
        count += 1;
        for (j, c) in t.children.iter().enumerate() {
            insert_topic(state, &site, Some(pid), &c.title, &c.description, j as i32).await?;
            count += 1;
        }
    }
    Ok(json!({ "proposed_topics": count }))
}

// ---------------------------------------------------------------- calendar

/// Publishing weekdays for a cadence of posts per week.
pub fn weekdays_for(cadence: i32) -> Vec<Weekday> {
    use Weekday::*;
    match cadence {
        1 => vec![Tue],
        2 => vec![Tue, Thu],
        3 => vec![Mon, Wed, Fri],
        4 => vec![Mon, Tue, Thu, Fri],
        5 => vec![Mon, Tue, Wed, Thu, Fri],
        6 => vec![Mon, Tue, Wed, Thu, Fri, Sat],
        _ => vec![Mon, Tue, Wed, Thu, Fri, Sat, Sun],
    }
}

pub const CALENDAR_HORIZON_DAYS: i64 = 21;
/// Above this share of comparison and listicle pieces the content starts to fight the business model.
pub const COMPARISON_SHARE_LIMIT: f64 = 0.25;

pub fn calendar_dates(cadence: i32, from: NaiveDate, until: NaiveDate) -> Vec<NaiveDate> {
    let days = weekdays_for(cadence);
    let mut out = Vec::new();
    let mut d = from;
    while d <= until {
        if days.contains(&d.weekday()) {
            out.push(d);
        }
        d += Duration::days(1);
    }
    out
}

pub async fn comparison_share(state: &AppState, site_id: Uuid) -> Result<(i64, i64)> {
    let (comparison, total): (i64, i64) = sqlx::query_as(
        "select count(*) filter (where content_type = any($2)), count(*) from pieces where site_id = $1 and status <> 'rejected'",
    )
    .bind(site_id)
    .bind(COMPARISON_TYPES)
    .fetch_one(&state.db)
    .await?;
    Ok((comparison, total))
}

#[derive(Deserialize)]
struct CalendarOut {
    pieces: Vec<PieceOut>,
}

#[derive(Deserialize)]
struct PieceOut {
    topic_id: Option<String>,
    content_type: String,
    title: String,
    #[serde(default)]
    angle: String,
    #[serde(default)]
    target_keyword: Option<String>,
    #[serde(default)]
    timeliness: Option<String>,
}

async fn propose_calendar(state: &AppState, site_id: Uuid) -> Result<Value> {
    let site = site_by_id(state, site_id).await?;
    let topics = sqlx::query_as::<_, Topic>("select * from topics where site_id = $1 and status = 'approved' order by position").bind(site_id).fetch_all(&state.db).await?;
    if topics.is_empty() {
        bail!("approve the topic map before planning the calendar");
    }
    let today = state.clock.today();
    let last: Option<NaiveDate> = sqlx::query_scalar("select max(scheduled_for) from pieces where site_id = $1 and status <> 'rejected'")
        .bind(site_id)
        .fetch_one(&state.db)
        .await?;
    let from = match last {
        Some(d) if d >= today => d + Duration::days(1),
        _ => today + Duration::days(1),
    };
    let dates = calendar_dates(site.cadence_per_week, from, today + Duration::days(CALENDAR_HORIZON_DAYS));
    if dates.is_empty() {
        return Ok(json!({ "proposed": 0 }));
    }
    let coverage: Vec<(Uuid, i64)> = sqlx::query_as(
        "select t.id, (select count(*) from pieces p where p.topic_id = t.id and p.status <> 'rejected') from topics t where t.site_id = $1 and t.status = 'approved'",
    )
    .bind(site_id)
    .fetch_all(&state.db)
    .await?;
    let (comparison, total) = comparison_share(state, site_id).await?;
    let allow_comparison = total == 0 || (comparison as f64 + 1.0) / (total as f64 + 1.0) <= COMPARISON_SHARE_LIMIT;
    let existing: Vec<String> = sqlx::query_scalar("select title from pieces where site_id = $1").bind(site_id).fetch_all(&state.db).await?;
    let ctx = json!({
        "site_name": site.name,
        "dates": dates.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
        "allow_comparison": allow_comparison,
        "existing_titles": existing,
        "topics": topics.iter().map(|t| json!({
            "id": t.id,
            "title": t.title,
            "description": t.description,
            "parent_id": t.parent_id,
            "covered": coverage.iter().find(|(id, _)| *id == t.id).map(|(_, c)| *c).unwrap_or(0),
        })).collect::<Vec<_>>(),
    });
    let out: CalendarOut = ai::run(state, scope(&site), "propose_calendar", Tier::Mid, ctx, None).await?;
    let mut made = 0;
    for (date, p) in dates.iter().zip(out.pieces.iter()) {
        let topic_id = p.topic_id.as_deref().and_then(|s| Uuid::parse_str(s).ok()).filter(|id| topics.iter().any(|t| t.id == *id));
        let content_type = if CONTENT_TYPES.contains(&p.content_type.as_str()) { p.content_type.as_str() } else { "explainer" };
        let content_type = if !allow_comparison && COMPARISON_TYPES.contains(&content_type) { "how_to" } else { content_type };
        let timeliness = match p.timeliness.as_deref() {
            Some("timely") => "timely",
            _ => "timeless",
        };
        sqlx::query(
            "insert into pieces (id, org_id, site_id, topic_id, content_type, title, angle, target_keyword, timeliness, scheduled_for, status, created_at)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'proposed', $11)",
        )
        .bind(Uuid::new_v4())
        .bind(site.org_id)
        .bind(site.id)
        .bind(topic_id)
        .bind(content_type)
        .bind(&p.title)
        .bind(&p.angle)
        .bind(&p.target_keyword)
        .bind(timeliness)
        .bind(date)
        .bind(state.clock.now())
        .execute(&state.db)
        .await?;
        made += 1;
    }
    Ok(json!({ "proposed": made }))
}

// ---------------------------------------------------------------- drafting

pub async fn month_spend(state: &AppState, site_id: Uuid) -> Result<f64> {
    let now = state.clock.now();
    let start = Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0).single().unwrap_or(now);
    let spent: Option<f64> = sqlx::query_scalar("select sum(cost_usd) from ai_calls where site_id = $1 and created_at >= $2")
        .bind(site_id)
        .bind(start)
        .fetch_one(&state.db)
        .await?;
    Ok(spent.unwrap_or(0.0))
}

/// Pauses drafting when the month's budget is spent. Returns true if drafting may go ahead.
async fn within_budget(state: &AppState, site: &Site) -> Result<bool> {
    if site.drafting_paused {
        return Ok(false);
    }
    let spent = month_spend(state, site.id).await?;
    if spent >= site.monthly_budget_usd {
        sqlx::query("update sites set drafting_paused = true where id = $1").bind(site.id).execute(&state.db).await?;
        notify(
            state,
            site.org_id,
            Some(site.id),
            "budget_paused",
            &format!("Drafting for {} is paused: ${:.2} of the ${:.2} monthly AI budget is spent. Raise the budget to resume.", site.name, spent, site.monthly_budget_usd),
        )
        .await?;
        return Ok(false);
    }
    Ok(true)
}

#[derive(Deserialize, serde::Serialize)]
struct Brief {
    title: String,
    #[serde(default)]
    angle: String,
    #[serde(default)]
    outline: Vec<String>,
    #[serde(default)]
    entry_nums: Vec<i64>,
    #[serde(default)]
    meta_description: String,
}

#[derive(Deserialize)]
struct DraftOut {
    title: String,
    #[serde(default)]
    meta_description: String,
    body_md: String,
}

pub async fn unique_post_slug(state: &AppState, site_id: Uuid, title: &str, except: Option<Uuid>) -> Result<String> {
    let base = slugify(title);
    let mut slug = base.clone();
    let mut n = 2;
    loop {
        let taken: bool = sqlx::query_scalar("select exists(select 1 from posts where site_id = $1 and slug = $2 and id is distinct from $3)
                                             or exists(select 1 from fixed_pages where site_id = $1 and slug = $2)")
            .bind(site_id)
            .bind(&slug)
            .bind(except)
            .fetch_one(&state.db)
            .await?;
        if !taken && !RESERVED_SLUGS.contains(&slug.as_str()) {
            return Ok(slug);
        }
        slug = format!("{base}-{n}");
        n += 1;
    }
}

/// Paths the blog service uses for its own pages.
pub const RESERVED_SLUGS: &[&str] = &["topic", "author", "sitemap", "rss", "feed", "robots", "llms", "page", "assets"];

async fn draft_piece(state: &AppState, piece_id: Uuid) -> Result<Value> {
    let piece = sqlx::query_as::<_, Piece>("select * from pieces where id = $1").bind(piece_id).fetch_one(&state.db).await?;
    if !matches!(piece.status.as_str(), "approved" | "drafting") {
        return Ok(json!({ "skipped": format!("piece is {}", piece.status) }));
    }
    let site = site_by_id(state, piece.site_id).await?;
    if !within_budget(state, &site).await? {
        sqlx::query("update pieces set status = 'approved' where id = $1").bind(piece.id).execute(&state.db).await?;
        return Ok(json!({ "paused": true }));
    }
    sqlx::query("update pieces set status = 'drafting' where id = $1").bind(piece.id).execute(&state.db).await?;

    // Coverage check: only write when the library has enough to say, including a position.
    let relevant = retrieval::entries_for_piece(state, &piece).await?;
    let gaps = retrieval::coverage_gaps(&relevant);
    if !gaps.is_empty() {
        let open: i64 = sqlx::query_scalar("select count(*) from questions where piece_id = $1 and answer is null").bind(piece.id).fetch_one(&state.db).await?;
        if open == 0 {
            #[derive(Deserialize)]
            struct Qs {
                questions: Vec<String>,
            }
            let ctx = json!({
                "piece": { "title": piece.title, "angle": piece.angle, "content_type": piece.content_type },
                "missing": gaps,
                "entries": relevant.iter().map(|e| json!({ "num": e.num, "kind": e.kind, "body": e.body })).collect::<Vec<_>>(),
            });
            let qs: Qs = ai::run(state, scope(&site), "coverage_questions", Tier::Cheap, ctx, None).await?;
            for (i, q) in qs.questions.iter().take(3).enumerate() {
                sqlx::query(
                    "insert into questions (id, org_id, site_id, piece_id, kind, topic, prompt, position, created_at) values ($1, $2, $3, $4, 'coverage', $5, $6, $7, $8)",
                )
                .bind(Uuid::new_v4())
                .bind(site.org_id)
                .bind(site.id)
                .bind(piece.id)
                .bind(if gaps.contains(&"opinion") && i == 0 { "opinions" } else { "" })
                .bind(q)
                .bind(i as i32)
                .bind(state.clock.now())
                .execute(&state.db)
                .await?;
            }
        }
        let reason = format!(
            "The library doesn't have enough to say yet ({}). Answer the questions in your inbox to release it.",
            gaps.iter().map(|g| if *g == "opinion" { "no opinion or position on this topic" } else { "too few relevant entries" }).collect::<Vec<_>>().join(", ")
        );
        sqlx::query("update pieces set status = 'held', hold_reason = $2 where id = $1").bind(piece.id).bind(reason).execute(&state.db).await?;
        return Ok(json!({ "held": gaps }));
    }

    let profile = refresh_profile(state, site.id).await?;
    let entry_json = |list: &[Entry]| list.iter().map(|e| json!({ "num": e.num, "kind": e.kind, "body": e.body })).collect::<Vec<_>>();
    let piece_json = json!({
        "title": piece.title, "angle": piece.angle, "content_type": piece.content_type,
        "target_keyword": piece.target_keyword, "timeliness": piece.timeliness,
    });
    let brief: Brief = ai::run(state, scope(&site), "write_brief", Tier::Mid, json!({ "piece": piece_json, "entries": entry_json(&relevant) }), Some(profile.clone())).await?;
    let mut selected: Vec<Entry> = relevant.iter().filter(|e| brief.entry_nums.contains(&(e.num as i64))).cloned().collect();
    if selected.len() < retrieval::MIN_RELEVANT {
        selected = relevant.clone();
    }
    let ctx = json!({
        "site_name": site.name,
        "content_type": piece.content_type,
        "brief": brief,
        "entries": entry_json(&selected),
    });
    let draft: DraftOut = ai::run(state, scope(&site), "draft", Tier::Draft, ctx, Some(profile.clone())).await?;

    // Citations may only point at approved entries that were supplied, directly or in the profile.
    let mut allowed: Vec<i32> = selected.iter().map(|e| e.num).collect();
    allowed.extend(cited_nums(&profile));
    let cited = cited_nums(&draft.body_md);
    if cited.is_empty() {
        bail!("the draft cited no library entries");
    }
    let approved: Vec<i32> = sqlx::query_scalar("select num from entries where site_id = $1 and status = 'approved' and num = any($2)")
        .bind(site.id)
        .bind(&cited)
        .fetch_all(&state.db)
        .await?;
    let invalid: Vec<String> = cited.iter().filter(|n| !allowed.contains(n) || !approved.contains(n)).map(|n| format!("E{n}")).collect();
    if !invalid.is_empty() {
        bail!("the draft cited entries that are not approved or were not supplied: {}", invalid.join(", "));
    }

    let title = if draft.title.trim().is_empty() { piece.title.clone() } else { draft.title.trim().to_string() };
    let slug = unique_post_slug(state, site.id, &title, None).await?;
    let now = state.clock.now();
    let post_id = Uuid::new_v4();
    sqlx::query(
        "insert into posts (id, org_id, site_id, piece_id, topic_id, content_type, title, slug, meta_description, body_md, ai_draft_md, status, timeliness, author_name, last_updated_at, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10, 'in_review', $11, $12, $13, $13)",
    )
    .bind(post_id)
    .bind(site.org_id)
    .bind(site.id)
    .bind(piece.id)
    .bind(piece.topic_id)
    .bind(&piece.content_type)
    .bind(&title)
    .bind(&slug)
    .bind(if draft.meta_description.is_empty() { &brief.meta_description } else { &draft.meta_description })
    .bind(&draft.body_md)
    .bind(&piece.timeliness)
    .bind(&site.author_name)
    .bind(now)
    .execute(&state.db)
    .await?;
    sqlx::query("insert into post_revisions (id, post_id, title, body_md, author, created_at) values ($1, $2, $3, $4, 'ai', $5)")
        .bind(Uuid::new_v4())
        .bind(post_id)
        .bind(&title)
        .bind(&draft.body_md)
        .bind(now)
        .execute(&state.db)
        .await?;
    sqlx::query("update pieces set status = 'drafted', hold_reason = '', brief = $2 where id = $1")
        .bind(piece.id)
        .bind(serde_json::to_value(&brief)?)
        .execute(&state.db)
        .await?;
    if site.review_mode == "auto" {
        enqueue(state, Some(site.org_id), Some(site.id), "auto_review", json!({ "post_id": post_id })).await?;
    }
    Ok(json!({ "post_id": post_id, "cited": cited.len() }))
}

async fn recheck_held(state: &AppState, site_id: Uuid) -> Result<Value> {
    let held = sqlx::query_as::<_, Piece>("select * from pieces where site_id = $1 and status = 'held'").bind(site_id).fetch_all(&state.db).await?;
    let today = state.clock.today();
    let mut released = 0;
    for piece in held {
        let open: i64 = sqlx::query_scalar("select count(*) from questions where piece_id = $1 and answer is null").bind(piece.id).fetch_one(&state.db).await?;
        if open > 0 {
            continue;
        }
        let relevant = retrieval::entries_for_piece(state, &piece).await?;
        if retrieval::coverage_gaps(&relevant).is_empty() {
            sqlx::query("update pieces set status = 'approved', hold_reason = '' where id = $1").bind(piece.id).execute(&state.db).await?;
            if piece.scheduled_for <= today {
                enqueue_once(state, Some(piece.org_id), Some(piece.site_id), "draft_piece", json!({ "piece_id": piece.id })).await?;
            }
            released += 1;
        }
    }
    Ok(json!({ "released": released }))
}

// ---------------------------------------------------------------- review and publishing

pub fn publish_time(date: NaiveDate) -> chrono::DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(9, 0, 0).unwrap())
}

async fn auto_review(state: &AppState, post_id: Uuid) -> Result<Value> {
    let post = sqlx::query_as::<_, Post>("select * from posts where id = $1").bind(post_id).fetch_one(&state.db).await?;
    if post.status != "in_review" {
        return Ok(json!({ "skipped": true }));
    }
    let site = site_by_id(state, post.site_id).await?;
    let mut issues: Vec<String> = crate::structure::check(&post.body_md, &post.content_type).into_iter().map(|f| f.message).collect();
    let coverage = crate::structure::citation_coverage(&post.body_md);
    if coverage < 1.0 {
        issues.push(format!("Only {:.0}% of paragraphs cite a library entry.", coverage * 100.0));
    }
    if issues.is_empty() {
        let nums = cited_nums(&post.body_md);
        let entries = sqlx::query_as::<_, Entry>("select * from entries where site_id = $1 and num = any($2)").bind(site.id).bind(&nums).fetch_all(&state.db).await?;
        #[derive(Deserialize)]
        struct Review {
            pass: bool,
            #[serde(default)]
            issues: Vec<String>,
        }
        let r: Review = ai::run(
            state,
            scope(&site),
            "review_draft",
            Tier::Mid,
            json!({ "body_md": post.body_md, "entries": entries.iter().map(|e| json!({ "num": e.num, "kind": e.kind, "body": e.body })).collect::<Vec<_>>() }),
            None,
        )
        .await?;
        if !r.pass {
            issues.extend(r.issues);
            if issues.is_empty() {
                issues.push("The AI reviewer did not pass it.".into());
            }
        }
    }
    if issues.is_empty() {
        let date: Option<NaiveDate> = match post.piece_id {
            Some(pid) => sqlx::query_scalar("select scheduled_for from pieces where id = $1").bind(pid).fetch_optional(&state.db).await?,
            None => None,
        };
        let at = date.map(publish_time).unwrap_or_else(|| state.clock.now()).max(state.clock.now());
        sqlx::query("update posts set status = 'scheduled', publish_at = $2, review_notes = 'Passed the automatic checks.' where id = $1")
            .bind(post.id)
            .bind(at)
            .execute(&state.db)
            .await?;
        Ok(json!({ "scheduled": at }))
    } else {
        sqlx::query("update posts set review_notes = $2 where id = $1").bind(post.id).bind(format!("Held for review: {}", issues.join(" "))).execute(&state.db).await?;
        Ok(json!({ "issues": issues }))
    }
}

/// Every public URL that shows a post, for cache purging.
pub async fn post_urls(state: &AppState, site: &Site, post: &Post) -> Result<Vec<String>> {
    let mut hosts = vec![format!("{}.{}", site.slug, state.config.blog_base_domain)];
    let custom: Vec<String> = sqlx::query_scalar("select hostname from domains where site_id = $1 and status = 'active'").bind(site.id).fetch_all(&state.db).await?;
    hosts.extend(custom);
    let mut paths = vec!["/".to_string(), format!("/{}", post.slug), format!("/{}.md", post.slug), "/sitemap.xml".into(), "/rss.xml".into(), "/llms.txt".into()];
    if let Some(topic_id) = post.topic_id {
        if let Some(slug) = sqlx::query_scalar::<_, String>("select slug from topics where id = $1").bind(topic_id).fetch_optional(&state.db).await? {
            paths.push(format!("/topic/{slug}"));
        }
    }
    Ok(hosts.iter().flat_map(|h| paths.iter().map(move |p| format!("https://{h}{p}"))).collect())
}

pub async fn purge_post(state: &AppState, post: &Post) -> Result<()> {
    let site = site_by_id(state, post.site_id).await?;
    let urls = post_urls(state, &site, post).await?;
    state.cdn.purge(&urls).await
}

async fn publish_post(state: &AppState, post_id: Uuid) -> Result<Value> {
    let now = state.clock.now();
    let post = sqlx::query_as::<_, Post>(
        "update posts set status = 'published', published_at = coalesce(published_at, $2), last_updated_at = $2
         where id = $1 and status = 'scheduled' and publish_at <= $2 returning *",
    )
    .bind(post_id)
    .bind(now)
    .fetch_optional(&state.db)
    .await?;
    let Some(post) = post else { return Ok(json!({ "skipped": true })) };
    purge_post(state, &post).await?;
    let site = site_by_id(state, post.site_id).await?;
    if site.github_repo.is_some() && site.github_token.is_some() {
        enqueue(state, Some(site.org_id), Some(site.id), "github_commit", json!({ "post_id": post.id })).await?;
    }
    enqueue(state, Some(site.org_id), Some(site.id), "learn_from_post", json!({ "post_id": post.id })).await?;
    Ok(json!({ "published": post.slug }))
}

pub fn front_matter_markdown(post: &Post, site: &Site) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let date = post.published_at.or(post.publish_at).unwrap_or(post.created_at);
    format!(
        "---\ntitle: \"{}\"\ndescription: \"{}\"\ndate: {}\nupdated: {}\nslug: {}\nauthor: \"{}\"\n---\n\n{}",
        esc(&post.title),
        esc(&post.meta_description),
        date.to_rfc3339(),
        post.last_updated_at.to_rfc3339(),
        post.slug,
        esc(if post.author_name.is_empty() { &site.author_name } else { &post.author_name }),
        strip_citations(&post.body_md)
    )
}

async fn github_commit(state: &AppState, post_id: Uuid) -> Result<Value> {
    let post = sqlx::query_as::<_, Post>("select * from posts where id = $1").bind(post_id).fetch_one(&state.db).await?;
    let site = site_by_id(state, post.site_id).await?;
    let (Some(repo), Some(token)) = (site.github_repo.clone(), site.github_token.clone()) else {
        return Ok(json!({ "skipped": "GitHub is not configured" }));
    };
    let path = format!("{}/{}.md", site.github_folder.trim_matches('/'), post.slug);
    state
        .github
        .put_file(&token, &repo, &site.github_branch, &path, &front_matter_markdown(&post, &site), &format!("Publish \"{}\"", post.title))
        .await
        .context("committing to GitHub")?;
    sqlx::query("update posts set github_committed_at = $2 where id = $1").bind(post.id).bind(state.clock.now()).execute(&state.db).await?;
    Ok(json!({ "path": path }))
}

// ---------------------------------------------------------------- learning

async fn propose_learned(state: &AppState, site: &Site, kind: &str, title: &str, before: &str, after: &str) -> Result<usize> {
    let out: ProposedEntries = ai::run(state, scope(site), "learn_from_edit", Tier::Mid, json!({ "before": before, "after": after }), None).await?;
    if out.entries.is_empty() {
        return Ok(0);
    }
    let passages: Vec<String> = out.entries.iter().map(|e| if e.quote.is_empty() { e.body.clone() } else { e.quote.clone() }).collect();
    let (source_id, passage_ids) = create_source(state, site, kind, title, title, &passages).await?;
    let mut made = 0;
    for (e, pid) in out.entries.into_iter().zip(passage_ids) {
        let quote = if e.quote.is_empty() { e.body.clone() } else { e.quote.clone() };
        if library::propose_entry(state, site, NewEntry { kind: e.kind, body: e.body, source_id: Some(source_id), source_kind: kind.into(), passage_id: Some(pid), quote, replaces: None })
            .await?
            .is_some()
        {
            made += 1;
        }
    }
    Ok(made)
}

async fn learn_from_edit(state: &AppState, post_id: Uuid) -> Result<Value> {
    let post = sqlx::query_as::<_, Post>("select * from posts where id = $1").bind(post_id).fetch_one(&state.db).await?;
    if strip_citations(&post.ai_draft_md).trim() == strip_citations(&post.body_md).trim() {
        return Ok(json!({ "proposed": 0 }));
    }
    let site = site_by_id(state, post.site_id).await?;
    let made = propose_learned(state, &site, "draft_edit", &format!("Your edits to \"{}\"", post.title), &post.ai_draft_md, &post.body_md).await?;
    Ok(json!({ "proposed": made }))
}

async fn learn_from_post(state: &AppState, post_id: Uuid) -> Result<Value> {
    let post = sqlx::query_as::<_, Post>("select * from posts where id = $1").bind(post_id).fetch_one(&state.db).await?;
    // Only paragraphs without citations can hold something the library doesn't know yet.
    let uncited: Vec<&str> = post
        .body_md
        .split("\n\n")
        .filter(|p| !p.trim().starts_with('#') && !p.trim().starts_with('|') && !crate::text::CITATION.is_match(p) && crate::text::word_count(p) >= 8)
        .collect();
    if uncited.is_empty() {
        return Ok(json!({ "proposed": 0 }));
    }
    let site = site_by_id(state, post.site_id).await?;
    let made = propose_learned(state, &site, "published_post", &format!("Published: {}", post.title), "", &uncited.join("\n\n")).await?;
    Ok(json!({ "proposed": made }))
}

// ---------------------------------------------------------------- digest and domains

async fn weekly_digest(state: &AppState, site_id: Uuid) -> Result<Value> {
    let site = site_by_id(state, site_id).await?;
    let end = state.clock.now();
    // Covers everything since the previous digest, or since the site was created.
    let last: Option<chrono::DateTime<Utc>> = sqlx::query_scalar("select max(period_end) from digests where site_id = $1")
        .bind(site_id)
        .fetch_one(&state.db)
        .await?;
    let start = last.unwrap_or(site.created_at - Duration::seconds(1));
    let entries = sqlx::query_as::<_, Entry>("select * from entries where site_id = $1 and approved_by = 'ai' and approved_at > $2 and approved_at <= $3 order by num")
        .bind(site_id)
        .bind(start)
        .bind(end)
        .fetch_all(&state.db)
        .await?;
    let entities: Vec<(Uuid, String, String)> = sqlx::query_as(
        "select id, kind, name from entities where site_id = $1 and approved_by = 'ai' and created_at > $2 and created_at <= $3 order by name",
    )
    .bind(site_id)
    .bind(start)
    .bind(end)
    .fetch_all(&state.db)
    .await?;
    let items = json!({
        "entries": entries.iter().map(|e| json!({ "id": e.id, "num": e.num, "kind": e.kind, "body": e.body, "version": e.version })).collect::<Vec<_>>(),
        "entities": entities.iter().map(|(id, k, n)| json!({ "id": id, "kind": k, "name": n })).collect::<Vec<_>>(),
    });
    sqlx::query("insert into digests (id, org_id, site_id, period_start, period_end, items, created_at) values ($1, $2, $3, $4, $5, $6, $5)")
        .bind(Uuid::new_v4())
        .bind(site.org_id)
        .bind(site.id)
        .bind(start)
        .bind(end)
        .bind(&items)
        .execute(&state.db)
        .await?;
    if entries.is_empty() && entities.is_empty() {
        return Ok(json!({ "changes": 0 }));
    }
    let mut body = format!("This week the AI approved {} library entries and {} entities for {}.\n\n", entries.len(), entities.len(), site.name);
    for e in &entries {
        body.push_str(&format!("- E{} ({}): {}\n  Review or revert: {}/sites/{}/library/{}\n", e.num, e.kind, e.body, state.config.app_url, site.id, e.id));
    }
    for (_, k, n) in &entities {
        body.push_str(&format!("- New {k}: {n}\n"));
    }
    let owners: Vec<String> = sqlx::query_scalar(
        "select u.email from memberships m join users u on u.id = m.user_id where m.org_id = $1 and m.role = 'owner' and u.email is not null",
    )
    .bind(site.org_id)
    .fetch_all(&state.db)
    .await?;
    let subject = format!("{}: what the AI approved this week", site.name);
    for to in owners {
        let id = Uuid::new_v4();
        sqlx::query("insert into email_outbox (id, org_id, to_address, subject, body, created_at) values ($1, $2, $3, $4, $5, $6)")
            .bind(id)
            .bind(site.org_id)
            .bind(&to)
            .bind(&subject)
            .bind(&body)
            .bind(end)
            .execute(&state.db)
            .await?;
        let result = state.mailer.send(&to, &subject, &body).await;
        sqlx::query("update email_outbox set sent_at = case when $2::text is null then $3 else null end, error = $2 where id = $1")
            .bind(id)
            .bind(result.err().map(|e| e.to_string()))
            .bind(end)
            .execute(&state.db)
            .await?;
    }
    Ok(json!({ "changes": entries.len() + entities.len() }))
}

async fn check_domain(state: &AppState, domain_id: Uuid) -> Result<Value> {
    let d = sqlx::query_as::<_, Domain>("select * from domains where id = $1").bind(domain_id).fetch_one(&state.db).await?;
    let Some(pid) = d.provider_id.clone() else { return Ok(json!({ "skipped": true })) };
    let s = state.cdn.hostname_status(&pid).await?;
    let status = if s.active { "active" } else if s.failed { "failed" } else { "pending" };
    sqlx::query("update domains set status = $2, verification = $3 where id = $1").bind(d.id).bind(status).bind(&s.verification).execute(&state.db).await?;
    Ok(json!({ "status": status }))
}

// ---------------------------------------------------------------- the scheduled tick

/// Creates all work that has come due. Called by Cloud Scheduler, or every minute locally.
pub async fn tick(state: &AppState) -> Result<Value> {
    let now = state.clock.now();
    let today = state.clock.today();
    let mut created = 0;

    let due: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "select p.id, p.org_id, p.site_id from pieces p join sites s on s.id = p.site_id
         where p.status = 'approved' and p.scheduled_for <= $1 and not s.drafting_paused",
    )
    .bind(today)
    .fetch_all(&state.db)
    .await?;
    for (id, org, site) in due {
        created += enqueue_once(state, Some(org), Some(site), "draft_piece", json!({ "piece_id": id })).await?.is_some() as usize;
    }

    let publish: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as("select id, org_id, site_id from posts where status = 'scheduled' and publish_at <= $1")
        .bind(now)
        .fetch_all(&state.db)
        .await?;
    for (id, org, site) in publish {
        created += enqueue_once(state, Some(org), Some(site), "publish_post", json!({ "post_id": id })).await?.is_some() as usize;
    }

    // Keep the rolling calendar topped up, once the previous proposal has been reviewed.
    let horizon = today + Duration::days(14);
    let short: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "select s.id, s.org_id from sites s
         where exists(select 1 from topics t where t.site_id = s.id and t.status = 'approved')
           and not exists(select 1 from pieces p where p.site_id = s.id and p.status = 'proposed')
           and coalesce((select max(scheduled_for) from pieces p where p.site_id = s.id and p.status <> 'rejected'), $1 - 1) < $2",
    )
    .bind(today)
    .bind(horizon)
    .fetch_all(&state.db)
    .await?;
    for (site, org) in short {
        created += enqueue_once(state, Some(org), Some(site), "propose_calendar", json!({ "site_id": site })).await?.is_some() as usize;
    }

    let recrawl: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "select id, org_id from sites where website_url is not null and last_crawled_at is not null and last_crawled_at < $1",
    )
    .bind(now - Duration::days(10))
    .fetch_all(&state.db)
    .await?;
    for (site, org) in recrawl {
        created += enqueue_once(state, Some(org), Some(site), "crawl_site", json!({ "site_id": site })).await?.is_some() as usize;
    }

    let digest: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "select s.id, s.org_id from sites s where s.created_at < $1
         and coalesce((select max(period_end) from digests d where d.site_id = s.id), s.created_at) <= $1",
    )
    .bind(now - Duration::days(7))
    .fetch_all(&state.db)
    .await?;
    for (site, org) in digest {
        created += enqueue_once(state, Some(org), Some(site), "weekly_digest", json!({ "site_id": site })).await?.is_some() as usize;
    }

    let pending: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as("select id, org_id, site_id from domains where status = 'pending'").fetch_all(&state.db).await?;
    for (id, org, site) in pending {
        created += enqueue_once(state, Some(org), Some(site), "check_domain", json!({ "domain_id": id })).await?.is_some() as usize;
    }
    Ok(json!({ "jobs_created": created }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cadence_dates() {
        let mon = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let dates = calendar_dates(3, mon, mon + Duration::days(6));
        let days: Vec<Weekday> = dates.iter().map(|d| d.weekday()).collect();
        assert_eq!(days, vec![Weekday::Mon, Weekday::Wed, Weekday::Fri]);
        assert_eq!(calendar_dates(7, mon, mon + Duration::days(6)).len(), 7);
    }
}
