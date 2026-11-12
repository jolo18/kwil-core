//! Sites, the onboarding interview, library entries, conflicts, the graph and ingestion.

use crate::auth::Ctx;
use crate::error::{AppError, AppResult};
use crate::jobs::{enqueue, enqueue_once};
use crate::library::{self, load_entry, load_site, Gate};
use crate::models::*;
use crate::state::AppState;
use crate::text::slugify;
use axum::extract::{Multipart, Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn blog_url(state: &AppState, site: &Site) -> String {
    let base = &state.config.blog_base_domain;
    if base == "localhost" {
        format!("http://{}.localhost:{}", site.slug, state.config.blog_port)
    } else {
        format!("https://{}.{}", site.slug, base)
    }
}

fn site_json(state: &AppState, site: &Site) -> Value {
    let mut v = serde_json::to_value(site).unwrap_or_default();
    v["blog_url"] = json!(blog_url(state, site));
    v["github_token_set"] = json!(site.github_token.is_some());
    v
}

pub async fn list_sites(State(state): State<AppState>, ctx: Ctx) -> AppResult<Json<Value>> {
    let sites = sqlx::query_as::<_, Site>("select * from sites where org_id = $1 order by created_at").bind(ctx.org_id).fetch_all(&state.db).await?;
    Ok(Json(json!({ "sites": sites.iter().map(|s| site_json(&state, s)).collect::<Vec<_>>() })))
}

#[derive(Deserialize)]
pub struct NewSite {
    name: String,
    #[serde(default)]
    product_description: String,
    #[serde(default)]
    audience: String,
    #[serde(default)]
    website_url: Option<String>,
    #[serde(default)]
    slug: Option<String>,
}

pub async fn create_site(State(state): State<AppState>, ctx: Ctx, Json(body): Json<NewSite>) -> AppResult<Json<Value>> {
    ctx.require_owner()?;
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request("a site needs a name"));
    }
    let base = slugify(body.slug.as_deref().unwrap_or(name));
    let mut slug = base.clone();
    let mut n = 2;
    while sqlx::query_scalar::<_, bool>("select exists(select 1 from sites where slug = $1)").bind(&slug).fetch_one(&state.db).await? {
        slug = format!("{base}-{n}");
        n += 1;
    }
    let website = body.website_url.filter(|u| !u.trim().is_empty()).map(|u| if u.starts_with("http") { u } else { format!("https://{u}") });
    let site = sqlx::query_as::<_, Site>(
        "insert into sites (id, org_id, name, slug, product_description, audience, website_url, author_name, branding, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) returning *",
    )
    .bind(Uuid::new_v4())
    .bind(ctx.org_id)
    .bind(name)
    .bind(&slug)
    .bind(body.product_description.trim())
    .bind(body.audience.trim())
    .bind(website)
    .bind(format!("The {name} team"))
    .bind(json!({ "primary_color": "#2563eb", "font": "system", "nav": [], "footer": format!("© {name}") }))
    .bind(state.clock.now())
    .fetch_one(&state.db)
    .await?;
    crate::workflows::start_interview(&state, &site).await?;
    library::refresh_profile(&state, site.id).await?;
    Ok(Json(site_json(&state, &site)))
}

pub async fn get_site(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let site = load_site(&state, ctx.org_id, id).await?;
    let counts: Vec<(String, i64)> = sqlx::query_as("select status, count(*) from entries where site_id = $1 group by status").bind(id).fetch_all(&state.db).await?;
    let posts: Vec<(String, i64)> = sqlx::query_as("select status, count(*) from posts where site_id = $1 group by status").bind(id).fetch_all(&state.db).await?;
    let spent = crate::workflows::month_spend(&state, id).await?;
    let mut v = site_json(&state, &site);
    v["entry_counts"] = json!(counts.into_iter().collect::<std::collections::HashMap<_, _>>());
    v["post_counts"] = json!(posts.into_iter().collect::<std::collections::HashMap<_, _>>());
    v["month_spend_usd"] = json!(spent);
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct SiteUpdate {
    name: Option<String>,
    product_description: Option<String>,
    audience: Option<String>,
    website_url: Option<String>,
    cadence_per_week: Option<i32>,
    monthly_budget_usd: Option<f64>,
    model_draft: Option<String>,
    model_mid: Option<String>,
    model_cheap: Option<String>,
    review_mode: Option<String>,
    strict_gates: Option<bool>,
    author_name: Option<String>,
    branding: Option<Value>,
    github_repo: Option<String>,
    github_branch: Option<String>,
    github_folder: Option<String>,
    github_token: Option<String>,
    resume_drafting: Option<bool>,
}

pub async fn update_site(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<SiteUpdate>) -> AppResult<Json<Value>> {
    ctx.require_owner()?;
    let site = load_site(&state, ctx.org_id, id).await?;
    if let Some(c) = b.cadence_per_week {
        if !(1..=7).contains(&c) {
            return Err(AppError::bad_request("cadence must be between 1 and 7 posts a week"));
        }
    }
    if let Some(m) = &b.review_mode {
        if m != "person" && m != "auto" {
            return Err(AppError::bad_request("review mode must be person or auto"));
        }
    }
    if let Some(budget) = b.monthly_budget_usd {
        if budget < 0.0 {
            return Err(AppError::bad_request("the budget cannot be negative"));
        }
    }
    let empty_to_none = |v: Option<String>| v.map(|s| s.trim().to_string()).map(|s| if s.is_empty() { None } else { Some(s) });
    let website = empty_to_none(b.website_url).map(|o| o.map(|u| if u.starts_with("http") { u } else { format!("https://{u}") }));
    let updated = sqlx::query_as::<_, Site>(
        "update sites set
            name = coalesce($2, name), product_description = coalesce($3, product_description), audience = coalesce($4, audience),
            website_url = case when $5 then $6 else website_url end,
            cadence_per_week = coalesce($7, cadence_per_week), monthly_budget_usd = coalesce($8, monthly_budget_usd),
            model_draft = coalesce($9, model_draft), model_mid = coalesce($10, model_mid), model_cheap = coalesce($11, model_cheap),
            review_mode = coalesce($12, review_mode), strict_gates = coalesce($13, strict_gates), author_name = coalesce($14, author_name),
            branding = coalesce($15, branding),
            github_repo = case when $16 then $17 else github_repo end, github_branch = coalesce($18, github_branch),
            github_folder = coalesce($19, github_folder), github_token = case when $20 then $21 else github_token end
         where id = $1 returning *",
    )
    .bind(site.id)
    .bind(b.name)
    .bind(b.product_description)
    .bind(b.audience)
    .bind(website.is_some())
    .bind(website.flatten())
    .bind(b.cadence_per_week)
    .bind(b.monthly_budget_usd)
    .bind(b.model_draft)
    .bind(b.model_mid)
    .bind(b.model_cheap)
    .bind(b.review_mode)
    .bind(b.strict_gates)
    .bind(b.author_name)
    .bind(b.branding)
    .bind(b.github_repo.is_some())
    .bind(empty_to_none(b.github_repo).flatten())
    .bind(b.github_branch)
    .bind(b.github_folder)
    .bind(b.github_token.is_some())
    .bind(empty_to_none(b.github_token).flatten())
    .fetch_one(&state.db)
    .await?;
    // Raising the budget or asking to resume lifts a budget pause if there is room.
    if updated.drafting_paused && (b.resume_drafting == Some(true) || b.monthly_budget_usd.is_some()) {
        let spent = crate::workflows::month_spend(&state, id).await?;
        if spent < updated.monthly_budget_usd {
            sqlx::query("update sites set drafting_paused = false where id = $1").bind(id).execute(&state.db).await?;
        }
    }
    library::refresh_profile(&state, id).await?;
    let site = load_site(&state, ctx.org_id, id).await?;
    Ok(Json(site_json(&state, &site)))
}

pub async fn get_profile(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let md = library::refresh_profile(&state, id).await?;
    Ok(Json(json!({ "profile_md": md })))
}

// ---------------------------------------------------------------- interview

#[derive(Deserialize)]
pub struct QuestionFilter {
    #[serde(default)]
    open: Option<bool>,
}

pub async fn list_questions(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Query(f): Query<QuestionFilter>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let questions = sqlx::query_as::<_, Question>(
        "select * from questions where site_id = $1 and ($2::bool is null or ($2 and answer is null) or (not $2 and answer is not null))
         order by (kind = 'coverage') desc, position, created_at",
    )
    .bind(id)
    .bind(f.open)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "questions": questions })))
}

#[derive(Deserialize)]
pub struct Answer {
    answer: String,
}

pub async fn answer_question(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<Answer>) -> AppResult<Json<Value>> {
    let answer = b.answer.trim();
    if answer.is_empty() {
        return Err(AppError::bad_request("the answer is empty"));
    }
    let q = sqlx::query_as::<_, Question>(
        "update questions set answer = $3, answered_at = $4 where id = $1 and org_id = $2 and answer is null returning *",
    )
    .bind(id)
    .bind(ctx.org_id)
    .bind(answer)
    .bind(state.clock.now())
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("open question"))?;
    let job = enqueue(&state, Some(q.org_id), Some(q.site_id), "interview_answered", json!({ "question_id": q.id })).await?;
    Ok(Json(json!({ "question": q, "job_id": job })))
}

// ---------------------------------------------------------------- entries

#[derive(Deserialize)]
pub struct EntryFilter {
    status: Option<String>,
    kind: Option<String>,
    q: Option<String>,
}

#[derive(sqlx::FromRow, serde::Serialize)]
struct EntryRow {
    #[sqlx(flatten)]
    #[serde(flatten)]
    entry: Entry,
    sources: Value,
    topics: Value,
}

const ENTRY_ROW_SQL: &str = "select e.*,
    coalesce((select json_agg(json_build_object('kind', s.kind, 'reference', s.reference, 'title', s.title, 'quote', es.quote))
              from entry_sources es join sources s on s.id = es.source_id where es.entry_id = e.id), '[]'::json)::jsonb as sources,
    coalesce((select json_agg(json_build_object('id', t.id, 'title', t.title))
              from entry_topics et join topics t on t.id = et.topic_id where et.entry_id = e.id), '[]'::json)::jsonb as topics
    from entries e";

pub async fn list_entries(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Query(f): Query<EntryFilter>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let rows = sqlx::query_as::<_, EntryRow>(sqlx::AssertSqlSafe(format!(
        "{ENTRY_ROW_SQL} where e.site_id = $1 and ($2::text is null or e.status = $2) and ($3::text is null or e.kind = $3)
         and ($4::text is null or e.body ilike '%' || $4 || '%') order by e.num desc limit 500"
    )))
    .bind(id)
    .bind(f.status.filter(|s| !s.is_empty()))
    .bind(f.kind.filter(|s| !s.is_empty()))
    .bind(f.q.filter(|s| !s.is_empty()))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "entries": rows })))
}

#[derive(Deserialize)]
pub struct NewEntryBody {
    kind: String,
    body: String,
}

pub async fn create_entry(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<NewEntryBody>) -> AppResult<Json<Entry>> {
    let site = load_site(&state, ctx.org_id, id).await?;
    Ok(Json(library::add_manual_entry(&state, &site, &b.kind, &b.body).await?))
}

pub async fn get_entry(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let entry = load_entry(&state, ctx.org_id, id).await?;
    let row = sqlx::query_as::<_, EntryRow>(sqlx::AssertSqlSafe(format!("{ENTRY_ROW_SQL} where e.id = $1"))).bind(id).fetch_one(&state.db).await?;
    let versions = sqlx::query_as::<_, EntryVersion>("select * from entry_versions where entry_id = $1 order by version desc").bind(id).fetch_all(&state.db).await?;
    let links: Vec<(Uuid, String, String, String, i32, String, String)> = sqlx::query_as(
        "select l.id, l.kind, l.status, l.note, o.num, o.body, case when l.from_entry = $1 then 'outgoing' else 'incoming' end
         from entry_links l join entries o on o.id = case when l.from_entry = $1 then l.to_entry else l.from_entry end
         where l.from_entry = $1 or l.to_entry = $1",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let entities: Vec<(Uuid, String, String)> = sqlx::query_as(
        "select n.id, n.kind, n.name from entry_entities x join entities n on n.id = x.entity_id where x.entry_id = $1",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let replaces: Option<(i32, String)> = match entry.replaces_entry_id {
        Some(r) => sqlx::query_as("select num, body from entries where id = $1").bind(r).fetch_optional(&state.db).await?,
        None => None,
    };
    let mut v = serde_json::to_value(row).unwrap_or_default();
    v["versions"] = json!(versions);
    v["links"] = json!(links.iter().map(|(id, k, s, n, num, body, dir)| json!({ "id": id, "kind": k, "status": s, "note": n, "other_num": num, "other_body": body, "direction": dir })).collect::<Vec<_>>());
    v["entities"] = json!(entities.iter().map(|(id, k, n)| json!({ "id": id, "kind": k, "name": n })).collect::<Vec<_>>());
    v["replaces"] = json!(replaces.map(|(num, body)| json!({ "num": num, "body": body })));
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct EntryEdit {
    kind: Option<String>,
    body: Option<String>,
}

pub async fn update_entry(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<EntryEdit>) -> AppResult<Json<Entry>> {
    let entry = load_entry(&state, ctx.org_id, id).await?;
    Ok(Json(library::edit(&state, &entry, b.kind.as_deref(), b.body.as_deref()).await?))
}

pub async fn approve_entry(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Entry>> {
    let entry = load_entry(&state, ctx.org_id, id).await?;
    Ok(Json(library::approve(&state, &entry, Gate::Person, "person").await?))
}

pub async fn reject_entry(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Entry>> {
    let entry = load_entry(&state, ctx.org_id, id).await?;
    Ok(Json(library::reject(&state, &entry, "person").await?))
}

#[derive(Deserialize)]
pub struct Revert {
    version: i32,
}

pub async fn revert_entry(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<Revert>) -> AppResult<Json<Entry>> {
    let entry = load_entry(&state, ctx.org_id, id).await?;
    Ok(Json(library::revert(&state, &entry, b.version).await?))
}

#[derive(Deserialize)]
pub struct Ids {
    ids: Vec<Uuid>,
}

pub async fn bulk_approve(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<Ids>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let mut approved = 0;
    for eid in b.ids {
        let entry = load_entry(&state, ctx.org_id, eid).await?;
        if entry.site_id == id && entry.status == "proposed" {
            library::approve(&state, &entry, Gate::Person, "person").await?;
            approved += 1;
        }
    }
    Ok(Json(json!({ "approved": approved })))
}

// ---------------------------------------------------------------- conflicts

pub async fn list_conflicts(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let rows: Vec<(Uuid, String, String, Uuid, i32, String, String, Uuid, i32, String, String)> = sqlx::query_as(
        "select l.id, l.kind, l.note, a.id, a.num, a.body, a.status, b.id, b.num, b.body, b.status
         from entry_links l join entries a on a.id = l.from_entry join entries b on b.id = l.to_entry
         where l.site_id = $1 and l.status = 'open' and l.kind in ('contradicts', 'replaces') order by l.created_at desc",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let conflicts: Vec<Value> = rows
        .into_iter()
        .map(|(lid, kind, note, aid, anum, abody, astatus, bid, bnum, bbody, bstatus)| {
            json!({
                "id": lid, "kind": kind, "note": note,
                "new": { "id": aid, "num": anum, "body": abody, "status": astatus },
                "existing": { "id": bid, "num": bnum, "body": bbody, "status": bstatus },
            })
        })
        .collect();
    Ok(Json(json!({ "conflicts": conflicts })))
}

#[derive(Deserialize)]
pub struct Resolve {
    /// keep_new, keep_existing or keep_both
    action: String,
}

pub async fn resolve_link(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<Resolve>) -> AppResult<Json<Value>> {
    let (from, to, status): (Uuid, Uuid, String) = sqlx::query_as("select from_entry, to_entry, status from entry_links where id = $1 and org_id = $2")
        .bind(id)
        .bind(ctx.org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("conflict"))?;
    if status != "open" {
        return Err(AppError::conflict("this conflict is already resolved"));
    }
    let new = load_entry(&state, ctx.org_id, from).await?;
    let existing = load_entry(&state, ctx.org_id, to).await?;
    match b.action.as_str() {
        "keep_new" => {
            if new.status == "proposed" {
                library::approve(&state, &new, Gate::Person, "person").await?;
            }
            if existing.status == "approved" {
                let now = state.clock.now();
                sqlx::query("update entries set status = 'superseded', version = version + 1, updated_at = $2 where id = $1").bind(existing.id).bind(now).execute(&state.db).await?;
                sqlx::query(
                    "insert into entry_versions (id, entry_id, version, kind, body, status, approved_by, changed_by, changed_at)
                     select $2, id, version, kind, body, status, approved_by, 'person', $3 from entries where id = $1",
                )
                .bind(existing.id)
                .bind(Uuid::new_v4())
                .bind(now)
                .execute(&state.db)
                .await?;
                library::refresh_profile(&state, existing.site_id).await?;
            }
        }
        "keep_existing" => {
            if new.status == "proposed" {
                library::reject(&state, &new, "person").await?;
            }
        }
        "keep_both" => {
            if new.status == "proposed" {
                library::approve(&state, &new, Gate::Person, "person").await?;
            }
        }
        _ => return Err(AppError::bad_request("action must be keep_new, keep_existing or keep_both")),
    }
    sqlx::query("update entry_links set status = 'resolved' where id = $1").bind(id).execute(&state.db).await?;
    Ok(Json(json!({ "resolved": true })))
}

// ---------------------------------------------------------------- graph

pub async fn get_graph(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let entities = sqlx::query_as::<_, Entity>("select id, site_id, kind, name, topic_id, status, approved_by from entities where site_id = $1 and status <> 'rejected' order by kind, name")
        .bind(id)
        .fetch_all(&state.db)
        .await?;
    let edges = sqlx::query_as::<_, Edge>("select id, from_entity, to_entity, kind, status, approved_by from entity_edges where site_id = $1 and status <> 'rejected'")
        .bind(id)
        .fetch_all(&state.db)
        .await?;
    let about: Vec<(Uuid, i64)> = sqlx::query_as(
        "select x.entity_id, count(*) from entry_entities x join entries e on e.id = x.entry_id where e.site_id = $1 and e.status = 'approved' group by x.entity_id",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let entities: Vec<Value> = entities
        .iter()
        .map(|e| {
            let mut v = serde_json::to_value(e).unwrap_or_default();
            v["entry_count"] = json!(about.iter().find(|(eid, _)| *eid == e.id).map(|(_, c)| *c).unwrap_or(0));
            v
        })
        .collect();
    Ok(Json(json!({ "entities": entities, "edges": edges })))
}

async fn entity_in_org(state: &AppState, ctx: &Ctx, id: Uuid) -> AppResult<Entity> {
    sqlx::query_as::<_, Entity>("select id, site_id, kind, name, topic_id, status, approved_by from entities where id = $1 and org_id = $2")
        .bind(id)
        .bind(ctx.org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("entity"))
}

pub async fn approve_entity(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    entity_in_org(&state, &ctx, id).await?;
    sqlx::query("update entities set status = 'approved', approved_by = 'person' where id = $1").bind(id).execute(&state.db).await?;
    Ok(Json(json!({ "approved": true })))
}

pub async fn reject_entity(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let e = entity_in_org(&state, &ctx, id).await?;
    if e.kind == "topic" {
        return Err(AppError::bad_request("topic entities follow the topic map; remove the topic instead"));
    }
    // Deleting an entity is a person's call; this endpoint is only reachable by a person.
    sqlx::query("update entities set status = 'rejected' where id = $1").bind(id).execute(&state.db).await?;
    sqlx::query("update entity_edges set status = 'rejected' where from_entity = $1 or to_entity = $1").bind(id).execute(&state.db).await?;
    sqlx::query("delete from entry_entities where entity_id = $1").bind(id).execute(&state.db).await?;
    Ok(Json(json!({ "rejected": true })))
}

#[derive(Deserialize)]
pub struct Merge {
    into: Uuid,
}

pub async fn merge_entity(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<Merge>) -> AppResult<Json<Value>> {
    let from = entity_in_org(&state, &ctx, id).await?;
    let into = entity_in_org(&state, &ctx, b.into).await?;
    if from.site_id != into.site_id || from.id == into.id {
        return Err(AppError::bad_request("can only merge two different entities of the same site"));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("insert into entry_entities (entry_id, entity_id) select entry_id, $2 from entry_entities where entity_id = $1 on conflict do nothing")
        .bind(from.id)
        .bind(into.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "insert into entity_edges (id, org_id, site_id, from_entity, to_entity, kind, status, approved_by, created_at)
         select gen_random_uuid(), org_id, site_id, case when from_entity = $1 then $2 else from_entity end,
                case when to_entity = $1 then $2 else to_entity end, kind, status, approved_by, created_at
         from entity_edges where (from_entity = $1 or to_entity = $1)
           and (case when from_entity = $1 then $2 else from_entity end) <> (case when to_entity = $1 then $2 else to_entity end)
         on conflict do nothing",
    )
    .bind(from.id)
    .bind(into.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("delete from entities where id = $1").bind(from.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!({ "merged_into": into.id })))
}

async fn edge_in_org(state: &AppState, ctx: &Ctx, id: Uuid) -> AppResult<()> {
    let ok: bool = sqlx::query_scalar("select exists(select 1 from entity_edges where id = $1 and org_id = $2)").bind(id).bind(ctx.org_id).fetch_one(&state.db).await?;
    if ok {
        Ok(())
    } else {
        Err(AppError::not_found("edge"))
    }
}

pub async fn approve_edge(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    edge_in_org(&state, &ctx, id).await?;
    sqlx::query("update entity_edges set status = 'approved', approved_by = 'person' where id = $1").bind(id).execute(&state.db).await?;
    Ok(Json(json!({ "approved": true })))
}

pub async fn reject_edge(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    edge_in_org(&state, &ctx, id).await?;
    sqlx::query("update entity_edges set status = 'rejected' where id = $1").bind(id).execute(&state.db).await?;
    Ok(Json(json!({ "rejected": true })))
}

// ---------------------------------------------------------------- crawl and uploads

pub async fn start_crawl(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let site = load_site(&state, ctx.org_id, id).await?;
    if site.website_url.is_none() {
        return Err(AppError::bad_request("add the site's website URL in settings first"));
    }
    let job = enqueue_once(&state, Some(site.org_id), Some(site.id), "crawl_site", json!({ "site_id": site.id })).await?;
    Ok(Json(json!({ "job_id": job })))
}

pub async fn list_uploads(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let uploads = sqlx::query_as::<_, Upload>("select id, site_id, filename, content_type, size_bytes, status, error, created_at from uploads where site_id = $1 order by created_at desc")
        .bind(id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(json!({ "uploads": uploads })))
}

pub const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;

pub async fn upload(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, mut form: Multipart) -> AppResult<Json<Value>> {
    let site = load_site(&state, ctx.org_id, id).await?;
    let mut made = Vec::new();
    while let Some(field) = form.next_field().await.map_err(|e| AppError::bad_request(e.to_string()))? {
        let filename = field.file_name().unwrap_or("upload.txt").to_string();
        let content_type = field.content_type().unwrap_or("").to_string();
        let bytes = field.bytes().await.map_err(|e| AppError::bad_request(e.to_string()))?;
        if bytes.len() > MAX_UPLOAD_BYTES {
            return Err(AppError::bad_request("files must be under 20 MB"));
        }
        let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
        if !["pdf", "docx", "md", "markdown", "txt", "text", "vtt", "srt"].contains(&ext.as_str()) {
            return Err(AppError::bad_request(format!("unsupported file type .{ext}; use PDF, DOCX, Markdown, text, VTT or SRT")));
        }
        let upload_id = Uuid::new_v4();
        let key = format!("{}/{}/{}-{}", site.org_id, site.id, upload_id, slugify(&filename));
        state.storage.put(&key, bytes.to_vec(), &content_type).await?;
        sqlx::query(
            "insert into uploads (id, org_id, site_id, filename, content_type, storage_key, size_bytes, created_at) values ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(upload_id)
        .bind(site.org_id)
        .bind(site.id)
        .bind(&filename)
        .bind(&content_type)
        .bind(&key)
        .bind(bytes.len() as i64)
        .bind(state.clock.now())
        .execute(&state.db)
        .await?;
        enqueue(&state, Some(site.org_id), Some(site.id), "process_upload", json!({ "upload_id": upload_id })).await?;
        made.push(upload_id);
    }
    if made.is_empty() {
        return Err(AppError::bad_request("no file was attached"));
    }
    Ok(Json(json!({ "uploads": made })))
}
