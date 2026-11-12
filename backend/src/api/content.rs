//! Topic map, calendar, posts and review, the inbox, jobs and costs.

use crate::auth::Ctx;
use crate::error::{AppError, AppResult};
use crate::jobs::{enqueue, enqueue_once, Job};
use crate::library::load_site;
use crate::models::*;
use crate::state::AppState;
use crate::text::cited_nums;
use crate::workflows::{self, comparison_share, ensure_topic_entity, publish_time, unique_post_slug, unique_topic_slug, COMPARISON_SHARE_LIMIT};
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

// ---------------------------------------------------------------- topics

async fn load_topic(state: &AppState, ctx: &Ctx, id: Uuid) -> AppResult<Topic> {
    sqlx::query_as::<_, Topic>("select * from topics where id = $1 and org_id = $2")
        .bind(id)
        .bind(ctx.org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("topic"))
}

pub async fn list_topics(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let rows: Vec<(Uuid, Option<Uuid>, String, String, String, String, i32, i64, i64, i64)> = sqlx::query_as(
        "select t.id, t.parent_id, t.title, t.slug, t.description, t.status, t.position,
                (select count(*) from pieces p where p.topic_id = t.id and p.status <> 'rejected'),
                (select count(*) from posts p where p.topic_id = t.id and p.status = 'published'),
                (select count(*) from entry_topics et join entries e on e.id = et.entry_id where et.topic_id = t.id and e.status = 'approved')
         from topics t where t.site_id = $1 and t.status <> 'rejected' order by t.position, t.title",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let topics: Vec<Value> = rows
        .into_iter()
        .map(|(id, parent, title, slug, desc, status, pos, pieces, published, entries)| {
            json!({ "id": id, "parent_id": parent, "title": title, "slug": slug, "description": desc, "status": status,
                    "position": pos, "pieces": pieces, "published": published, "entries": entries, "gap": pieces == 0 && status == "approved" })
        })
        .collect();
    Ok(Json(json!({ "topics": topics })))
}

pub async fn propose_topics(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let site = load_site(&state, ctx.org_id, id).await?;
    let job = enqueue_once(&state, Some(site.org_id), Some(site.id), "propose_topics", json!({ "site_id": site.id })).await?;
    Ok(Json(json!({ "job_id": job })))
}

#[derive(Deserialize)]
pub struct NewTopic {
    title: String,
    #[serde(default)]
    description: String,
    parent_id: Option<Uuid>,
}

pub async fn create_topic(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<NewTopic>) -> AppResult<Json<Topic>> {
    let site = load_site(&state, ctx.org_id, id).await?;
    if b.title.trim().is_empty() {
        return Err(AppError::bad_request("a topic needs a title"));
    }
    if let Some(p) = b.parent_id {
        let parent = load_topic(&state, &ctx, p).await?;
        if parent.site_id != site.id {
            return Err(AppError::bad_request("parent topic belongs to another site"));
        }
    }
    let slug = unique_topic_slug(&state, site.id, &b.title).await?;
    let topic = sqlx::query_as::<_, Topic>(
        "insert into topics (id, org_id, site_id, parent_id, title, slug, description, status, position, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, 'approved', 100, $8) returning *",
    )
    .bind(Uuid::new_v4())
    .bind(site.org_id)
    .bind(site.id)
    .bind(b.parent_id)
    .bind(b.title.trim())
    .bind(slug)
    .bind(b.description.trim())
    .bind(state.clock.now())
    .fetch_one(&state.db)
    .await?;
    ensure_topic_entity(&state, &topic).await?;
    enqueue_once(&state, Some(site.org_id), Some(site.id), "retag_site", json!({ "site_id": site.id })).await?;
    Ok(Json(topic))
}

#[derive(Deserialize)]
pub struct TopicEdit {
    title: Option<String>,
    description: Option<String>,
    parent_id: Option<Option<Uuid>>,
}

pub async fn update_topic(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<TopicEdit>) -> AppResult<Json<Topic>> {
    let topic = load_topic(&state, &ctx, id).await?;
    if let Some(Some(p)) = b.parent_id {
        if p == topic.id {
            return Err(AppError::bad_request("a topic cannot be its own parent"));
        }
        load_topic(&state, &ctx, p).await?;
    }
    let updated = sqlx::query_as::<_, Topic>(
        "update topics set title = coalesce($2, title), description = coalesce($3, description),
         parent_id = case when $4 then $5 else parent_id end where id = $1 returning *",
    )
    .bind(id)
    .bind(b.title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()))
    .bind(b.description)
    .bind(b.parent_id.is_some())
    .bind(b.parent_id.flatten())
    .fetch_one(&state.db)
    .await?;
    sqlx::query("update entities set name = $2 where topic_id = $1").bind(id).bind(&updated.title).execute(&state.db).await?;
    Ok(Json(updated))
}

pub async fn delete_topic(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    load_topic(&state, &ctx, id).await?;
    sqlx::query("delete from topics where id = $1").bind(id).execute(&state.db).await?;
    Ok(Json(json!({ "deleted": true })))
}

async fn approve_topic_inner(state: &AppState, topic: &Topic) -> AppResult<()> {
    let t = sqlx::query_as::<_, Topic>("update topics set status = 'approved' where id = $1 returning *").bind(topic.id).fetch_one(&state.db).await?;
    ensure_topic_entity(state, &t).await?;
    Ok(())
}

pub async fn approve_topic(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let topic = load_topic(&state, &ctx, id).await?;
    approve_topic_inner(&state, &topic).await?;
    if let Some(p) = topic.parent_id {
        let parent = load_topic(&state, &ctx, p).await?;
        if parent.status == "proposed" {
            approve_topic_inner(&state, &parent).await?;
        }
    }
    enqueue_once(&state, Some(topic.org_id), Some(topic.site_id), "retag_site", json!({ "site_id": topic.site_id })).await?;
    Ok(Json(json!({ "approved": true })))
}

pub async fn reject_topic(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    load_topic(&state, &ctx, id).await?;
    sqlx::query("update topics set status = 'rejected' where id = $1 or parent_id = $1").bind(id).execute(&state.db).await?;
    sqlx::query("delete from entities where topic_id in (select id from topics where status = 'rejected' and (id = $1 or parent_id = $1))").bind(id).execute(&state.db).await?;
    Ok(Json(json!({ "rejected": true })))
}

pub async fn approve_all_topics(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let site = load_site(&state, ctx.org_id, id).await?;
    let proposed = sqlx::query_as::<_, Topic>("select * from topics where site_id = $1 and status = 'proposed'").bind(id).fetch_all(&state.db).await?;
    for t in &proposed {
        approve_topic_inner(&state, t).await?;
    }
    enqueue_once(&state, Some(site.org_id), Some(site.id), "retag_site", json!({ "site_id": site.id })).await?;
    Ok(Json(json!({ "approved": proposed.len() })))
}

// ---------------------------------------------------------------- calendar

pub async fn get_calendar(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let site = load_site(&state, ctx.org_id, id).await?;
    let rows: Vec<(Uuid, Option<Uuid>, Option<String>, String, String, String, Option<String>, String, NaiveDate, String, String, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "select p.id, p.topic_id, t.title, p.content_type, p.title, p.angle, p.target_keyword, p.timeliness, p.scheduled_for, p.status, p.hold_reason,
                (select id from posts where piece_id = p.id order by created_at desc limit 1),
                (select status from posts where piece_id = p.id order by created_at desc limit 1)
         from pieces p left join topics t on t.id = p.topic_id
         where p.site_id = $1 and p.status <> 'rejected' order by p.scheduled_for, p.created_at",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let pieces: Vec<Value> = rows
        .into_iter()
        .map(|(id, tid, ttitle, ct, title, angle, kw, tl, date, status, hold, post_id, post_status)| {
            json!({ "id": id, "topic_id": tid, "topic_title": ttitle, "content_type": ct, "title": title, "angle": angle,
                    "target_keyword": kw, "timeliness": tl, "scheduled_for": date, "status": status, "hold_reason": hold,
                    "post_id": post_id, "post_status": post_status })
        })
        .collect();
    let (comparison, total) = comparison_share(&state, id).await?;
    let share = if total == 0 { 0.0 } else { comparison as f64 / total as f64 };
    let gaps: Vec<(Uuid, String)> = sqlx::query_as(
        "select t.id, t.title from topics t where t.site_id = $1 and t.status = 'approved'
         and not exists(select 1 from pieces p where p.topic_id = t.id and p.status <> 'rejected') order by t.position",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "pieces": pieces,
        "cadence_per_week": site.cadence_per_week,
        "drafting_paused": site.drafting_paused,
        "mix": {
            "comparison": comparison,
            "total": total,
            "share": share,
            "limit": COMPARISON_SHARE_LIMIT,
            "warning": if share > COMPARISON_SHARE_LIMIT {
                Some(format!("{:.0}% of planned pieces are comparisons, alternatives or listicles. Above {:.0}% a product company starts to look like a review site.", share * 100.0, COMPARISON_SHARE_LIMIT * 100.0))
            } else { None },
        },
        "gaps": gaps.iter().map(|(id, t)| json!({ "id": id, "title": t })).collect::<Vec<_>>(),
    })))
}

pub async fn propose_calendar(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let site = load_site(&state, ctx.org_id, id).await?;
    let approved: i64 = sqlx::query_scalar("select count(*) from topics where site_id = $1 and status = 'approved'").bind(id).fetch_one(&state.db).await?;
    if approved == 0 {
        return Err(AppError::bad_request("approve the topic map before planning the calendar"));
    }
    let job = enqueue_once(&state, Some(site.org_id), Some(site.id), "propose_calendar", json!({ "site_id": site.id })).await?;
    Ok(Json(json!({ "job_id": job })))
}

pub async fn approve_calendar(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let n = sqlx::query("update pieces set status = 'approved' where site_id = $1 and status = 'proposed'").bind(id).execute(&state.db).await?.rows_affected();
    Ok(Json(json!({ "approved": n })))
}

async fn load_piece(state: &AppState, ctx: &Ctx, id: Uuid) -> AppResult<Piece> {
    sqlx::query_as::<_, Piece>("select * from pieces where id = $1 and org_id = $2")
        .bind(id)
        .bind(ctx.org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("calendar item"))
}

#[derive(Deserialize)]
pub struct PieceEdit {
    title: Option<String>,
    angle: Option<String>,
    content_type: Option<String>,
    scheduled_for: Option<NaiveDate>,
    topic_id: Option<Uuid>,
    target_keyword: Option<String>,
    timeliness: Option<String>,
}

pub async fn update_piece(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<PieceEdit>) -> AppResult<Json<Piece>> {
    let piece = load_piece(&state, &ctx, id).await?;
    if let Some(ct) = &b.content_type {
        if !CONTENT_TYPES.contains(&ct.as_str()) {
            return Err(AppError::bad_request(format!("unknown content type {ct}")));
        }
    }
    if let Some(t) = &b.timeliness {
        if t != "timely" && t != "timeless" {
            return Err(AppError::bad_request("timeliness must be timely or timeless"));
        }
    }
    if let Some(t) = b.topic_id {
        let topic = load_topic(&state, &ctx, t).await?;
        if topic.site_id != piece.site_id {
            return Err(AppError::bad_request("topic belongs to another site"));
        }
    }
    let p = sqlx::query_as::<_, Piece>(
        "update pieces set title = coalesce($2, title), angle = coalesce($3, angle), content_type = coalesce($4, content_type),
         scheduled_for = coalesce($5, scheduled_for), topic_id = coalesce($6, topic_id), target_keyword = coalesce($7, target_keyword),
         timeliness = coalesce($8, timeliness) where id = $1 returning *",
    )
    .bind(id)
    .bind(b.title)
    .bind(b.angle)
    .bind(b.content_type)
    .bind(b.scheduled_for)
    .bind(b.topic_id)
    .bind(b.target_keyword)
    .bind(b.timeliness)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(p))
}

pub async fn approve_piece(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Piece>> {
    let piece = load_piece(&state, &ctx, id).await?;
    if !matches!(piece.status.as_str(), "proposed" | "failed") {
        return Err(AppError::conflict(format!("this item is already {}", piece.status)));
    }
    Ok(Json(sqlx::query_as::<_, Piece>("update pieces set status = 'approved', hold_reason = '' where id = $1 returning *").bind(id).fetch_one(&state.db).await?))
}

pub async fn reject_piece(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Piece>> {
    let piece = load_piece(&state, &ctx, id).await?;
    if matches!(piece.status.as_str(), "drafting" | "drafted") {
        return Err(AppError::conflict("this item already has a draft; reject the draft instead"));
    }
    Ok(Json(sqlx::query_as::<_, Piece>("update pieces set status = 'rejected' where id = $1 returning *").bind(id).fetch_one(&state.db).await?))
}

/// Moves an approved item to today and drafts it now.
pub async fn draft_now(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let piece = load_piece(&state, &ctx, id).await?;
    if !matches!(piece.status.as_str(), "approved" | "proposed" | "failed") {
        return Err(AppError::conflict(format!("this item is {}", piece.status)));
    }
    sqlx::query("update pieces set status = 'approved', hold_reason = '', scheduled_for = least(scheduled_for, $2) where id = $1")
        .bind(id)
        .bind(state.clock.today())
        .execute(&state.db)
        .await?;
    let job = enqueue_once(&state, Some(piece.org_id), Some(piece.site_id), "draft_piece", json!({ "piece_id": piece.id })).await?;
    Ok(Json(json!({ "job_id": job })))
}

// ---------------------------------------------------------------- posts

#[derive(Deserialize)]
pub struct PostFilter {
    status: Option<String>,
}

pub async fn list_posts(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Query(f): Query<PostFilter>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let posts = sqlx::query_as::<_, Post>("select * from posts where site_id = $1 and ($2::text is null or status = $2) order by coalesce(publish_at, created_at) desc")
        .bind(id)
        .bind(f.status.filter(|s| !s.is_empty()))
        .fetch_all(&state.db)
        .await?;
    Ok(Json(json!({ "posts": posts })))
}

async fn load_post(state: &AppState, ctx: &Ctx, id: Uuid) -> AppResult<Post> {
    sqlx::query_as::<_, Post>("select * from posts where id = $1 and org_id = $2")
        .bind(id)
        .bind(ctx.org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("post"))
}

pub async fn get_post(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let post = load_post(&state, &ctx, id).await?;
    let revisions = sqlx::query_as::<_, PostRevision>("select * from post_revisions where post_id = $1 order by created_at desc").bind(id).fetch_all(&state.db).await?;
    let nums = cited_nums(&post.body_md);
    let cited = sqlx::query_as::<_, Entry>("select * from entries where site_id = $1 and num = any($2) order by num").bind(post.site_id).bind(&nums).fetch_all(&state.db).await?;
    let findings = crate::structure::check(&post.body_md, &post.content_type);
    let piece = match post.piece_id {
        Some(p) => sqlx::query_as::<_, Piece>("select * from pieces where id = $1").bind(p).fetch_optional(&state.db).await?,
        None => None,
    };
    let site = load_site(&state, ctx.org_id, post.site_id).await?;
    Ok(Json(json!({
        "post": post,
        "revisions": revisions,
        "cited_entries": cited,
        "findings": findings,
        "citation_coverage": crate::structure::citation_coverage(&post.body_md),
        "piece": piece,
        "public_url": format!("{}/{}", crate::api::library::blog_url(&state, &site), post.slug),
    })))
}

#[derive(Deserialize)]
pub struct PostEdit {
    title: Option<String>,
    body_md: Option<String>,
    meta_description: Option<String>,
    timeliness: Option<String>,
    slug: Option<String>,
    author_name: Option<String>,
}

pub async fn update_post(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<PostEdit>) -> AppResult<Json<Value>> {
    let post = load_post(&state, &ctx, id).await?;
    if let Some(t) = &b.timeliness {
        if t != "timely" && t != "timeless" {
            return Err(AppError::bad_request("timeliness must be timely or timeless"));
        }
    }
    // Citations a person adds must also point at approved entries.
    if let Some(body) = &b.body_md {
        let nums = cited_nums(body);
        let approved: Vec<i32> = sqlx::query_scalar("select num from entries where site_id = $1 and status = 'approved' and num = any($2)")
            .bind(post.site_id)
            .bind(&nums)
            .fetch_all(&state.db)
            .await?;
        let bad: Vec<String> = nums.iter().filter(|n| !approved.contains(n)).map(|n| format!("E{n}")).collect();
        if !bad.is_empty() {
            return Err(AppError::bad_request(format!("these citations don't point at approved entries: {}", bad.join(", "))));
        }
    }
    let slug = match &b.slug {
        Some(s) if crate::text::slugify(s) != post.slug => Some(unique_post_slug(&state, post.site_id, s, Some(post.id)).await?),
        _ => None,
    };
    let now = state.clock.now();
    let updated = sqlx::query_as::<_, Post>(
        "update posts set title = coalesce($2, title), body_md = coalesce($3, body_md), meta_description = coalesce($4, meta_description),
         timeliness = coalesce($5, timeliness), slug = coalesce($6, slug), author_name = coalesce($7, author_name), last_updated_at = $8
         where id = $1 returning *",
    )
    .bind(id)
    .bind(b.title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()))
    .bind(&b.body_md)
    .bind(b.meta_description)
    .bind(b.timeliness)
    .bind(slug)
    .bind(b.author_name)
    .bind(now)
    .fetch_one(&state.db)
    .await?;
    if b.body_md.is_some() && updated.body_md != post.body_md || updated.title != post.title {
        sqlx::query("insert into post_revisions (id, post_id, title, body_md, author, created_at) values ($1, $2, $3, $4, 'person', $5)")
            .bind(Uuid::new_v4())
            .bind(id)
            .bind(&updated.title)
            .bind(&updated.body_md)
            .bind(now)
            .execute(&state.db)
            .await?;
    }
    if updated.status == "published" {
        workflows::purge_post(&state, &updated).await?;
        if post.slug != updated.slug {
            workflows::purge_post(&state, &post).await?;
        }
    }
    Ok(Json(json!({
        "post": updated,
        "findings": crate::structure::check(&updated.body_md, &updated.content_type),
    })))
}

#[derive(Deserialize, Default)]
pub struct ApprovePost {
    publish_at: Option<DateTime<Utc>>,
}

pub async fn approve_post(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, body: Option<Json<ApprovePost>>) -> AppResult<Json<Post>> {
    let post = load_post(&state, &ctx, id).await?;
    if post.status != "in_review" {
        return Err(AppError::conflict(format!("this post is {}", post.status)));
    }
    let now = state.clock.now();
    let requested = body.and_then(|b| b.0.publish_at);
    let planned = match post.piece_id {
        Some(pid) => sqlx::query_scalar::<_, NaiveDate>("select scheduled_for from pieces where id = $1").bind(pid).fetch_optional(&state.db).await?.map(publish_time),
        None => None,
    };
    let at = requested.or(planned).unwrap_or(now).max(now);
    let updated = sqlx::query_as::<_, Post>("update posts set status = 'scheduled', publish_at = $2, review_notes = '' where id = $1 returning *")
        .bind(id)
        .bind(at)
        .fetch_one(&state.db)
        .await?;
    if at <= now {
        enqueue(&state, Some(post.org_id), Some(post.site_id), "publish_post", json!({ "post_id": post.id })).await?;
    }
    enqueue(&state, Some(post.org_id), Some(post.site_id), "learn_from_edit", json!({ "post_id": post.id })).await?;
    Ok(Json(updated))
}

#[derive(Deserialize, Default)]
pub struct RejectPost {
    #[serde(default)]
    notes: String,
}

pub async fn reject_post(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, body: Option<Json<RejectPost>>) -> AppResult<Json<Post>> {
    let post = load_post(&state, &ctx, id).await?;
    if post.status == "published" {
        return Err(AppError::conflict("unpublish the post first"));
    }
    let notes = body.map(|b| b.0.notes).unwrap_or_default();
    Ok(Json(sqlx::query_as::<_, Post>("update posts set status = 'rejected', review_notes = $2 where id = $1 returning *").bind(id).bind(notes).fetch_one(&state.db).await?))
}

pub async fn redraft_post(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let post = load_post(&state, &ctx, id).await?;
    let Some(piece_id) = post.piece_id else { return Err(AppError::bad_request("this post has no calendar item to redraft from")) };
    if post.status == "published" {
        return Err(AppError::conflict("unpublish the post first"));
    }
    sqlx::query("update posts set status = 'rejected', review_notes = 'Replaced by a new draft.' where id = $1").bind(id).execute(&state.db).await?;
    sqlx::query("update pieces set status = 'approved' where id = $1").bind(piece_id).execute(&state.db).await?;
    let job = enqueue_once(&state, Some(post.org_id), Some(post.site_id), "draft_piece", json!({ "piece_id": piece_id })).await?;
    Ok(Json(json!({ "job_id": job })))
}

pub async fn unpublish_post(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Post>> {
    let post = load_post(&state, &ctx, id).await?;
    if !matches!(post.status.as_str(), "published" | "scheduled") {
        return Err(AppError::conflict("only published or scheduled posts can be unpublished"));
    }
    let updated = sqlx::query_as::<_, Post>("update posts set status = 'in_review' where id = $1 returning *").bind(id).fetch_one(&state.db).await?;
    workflows::purge_post(&state, &updated).await?;
    Ok(Json(updated))
}

pub async fn post_markdown(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<impl IntoResponse> {
    let post = load_post(&state, &ctx, id).await?;
    let site = load_site(&state, ctx.org_id, post.site_id).await?;
    let md = workflows::front_matter_markdown(&post, &site);
    Ok((
        [
            (header::CONTENT_TYPE, "text/markdown; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}.md\"", post.slug)),
        ],
        md,
    ))
}

pub async fn commit_to_github(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let post = load_post(&state, &ctx, id).await?;
    let site = load_site(&state, ctx.org_id, post.site_id).await?;
    if site.github_repo.is_none() || site.github_token.is_none() {
        return Err(AppError::bad_request("set the GitHub repository and token in the site's settings first"));
    }
    let job = enqueue(&state, Some(post.org_id), Some(post.site_id), "github_commit", json!({ "post_id": post.id })).await?;
    Ok(Json(json!({ "job_id": job })))
}

#[derive(Deserialize)]
pub struct CheckBody {
    body_md: String,
    #[serde(default)]
    content_type: String,
}

pub async fn check_structure(_ctx: Ctx, Json(b): Json<CheckBody>) -> AppResult<Json<Value>> {
    Ok(Json(json!({
        "findings": crate::structure::check(&b.body_md, &b.content_type),
        "citation_coverage": crate::structure::citation_coverage(&b.body_md),
    })))
}

// ---------------------------------------------------------------- inbox

pub async fn inbox(State(state): State<AppState>, ctx: Ctx) -> AppResult<Json<Value>> {
    let sites = sqlx::query_as::<_, Site>("select * from sites where org_id = $1 order by created_at").bind(ctx.org_id).fetch_all(&state.db).await?;
    let mut out = Vec::new();
    let mut total = 0;
    for site in &sites {
        let questions = sqlx::query_as::<_, Question>("select * from questions where site_id = $1 and answer is null order by (kind = 'coverage') desc, position, created_at")
            .bind(site.id)
            .fetch_all(&state.db)
            .await?;
        let entries = sqlx::query_as::<_, Entry>("select * from entries where site_id = $1 and status = 'proposed' and gate = 'person' order by num")
            .bind(site.id)
            .fetch_all(&state.db)
            .await?;
        let waiting_on_ai: i64 = sqlx::query_scalar("select count(*) from entries where site_id = $1 and status = 'proposed' and gate <> 'person'")
            .bind(site.id)
            .fetch_one(&state.db)
            .await?;
        let drafts = sqlx::query_as::<_, Post>("select * from posts where site_id = $1 and status = 'in_review' order by created_at").bind(site.id).fetch_all(&state.db).await?;
        let proposed_pieces: i64 = sqlx::query_scalar("select count(*) from pieces where site_id = $1 and status = 'proposed'").bind(site.id).fetch_one(&state.db).await?;
        let proposed_topics: i64 = sqlx::query_scalar("select count(*) from topics where site_id = $1 and status = 'proposed'").bind(site.id).fetch_one(&state.db).await?;
        let conflicts: i64 = sqlx::query_scalar("select count(*) from entry_links where site_id = $1 and status = 'open' and kind in ('contradicts', 'replaces')")
            .bind(site.id)
            .fetch_one(&state.db)
            .await?;
        let proposed_graph: i64 = sqlx::query_scalar(
            "select (select count(*) from entities where site_id = $1 and status = 'proposed') + (select count(*) from entity_edges where site_id = $1 and status = 'proposed')",
        )
        .bind(site.id)
        .fetch_one(&state.db)
        .await?;
        let held = sqlx::query_as::<_, Piece>("select * from pieces where site_id = $1 and status in ('held', 'failed') order by scheduled_for").bind(site.id).fetch_all(&state.db).await?;
        let notifications = sqlx::query_as::<_, Notification>("select id, site_id, kind, message, read_at, created_at from notifications where site_id = $1 and read_at is null order by created_at desc")
            .bind(site.id)
            .fetch_all(&state.db)
            .await?;
        let count = questions.len() + entries.len() + drafts.len() + (proposed_pieces > 0) as usize + (proposed_topics > 0) as usize
            + conflicts as usize + notifications.len() + (proposed_graph > 0) as usize;
        total += count;
        out.push(json!({
            "site": { "id": site.id, "name": site.name, "slug": site.slug },
            "count": count,
            "questions": questions,
            "entries": entries,
            "entries_waiting_on_ai": waiting_on_ai,
            "drafts": drafts,
            "proposed_pieces": proposed_pieces,
            "proposed_topics": proposed_topics,
            "conflicts": conflicts,
            "proposed_graph": proposed_graph,
            "held": held,
            "notifications": notifications,
        }));
    }
    let org_notes = sqlx::query_as::<_, Notification>("select id, site_id, kind, message, read_at, created_at from notifications where org_id = $1 and site_id is null and read_at is null")
        .bind(ctx.org_id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(json!({ "total": total + org_notes.len(), "sites": out, "notifications": org_notes })))
}

pub async fn read_notification(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    sqlx::query("update notifications set read_at = $3 where id = $1 and org_id = $2").bind(id).bind(ctx.org_id).bind(state.clock.now()).execute(&state.db).await?;
    Ok(Json(json!({ "read": true })))
}

// ---------------------------------------------------------------- jobs, costs, digests

pub async fn list_jobs(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let jobs = sqlx::query_as::<_, Job>("select * from jobs where site_id = $1 order by created_at desc limit 100").bind(id).fetch_all(&state.db).await?;
    let active: i64 = sqlx::query_scalar("select count(*) from jobs where site_id = $1 and status in ('queued', 'running', 'failed')").bind(id).fetch_one(&state.db).await?;
    Ok(Json(json!({ "jobs": jobs, "active": active })))
}

pub async fn get_job(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Job>> {
    Ok(Json(
        sqlx::query_as::<_, Job>("select * from jobs where id = $1 and org_id = $2")
            .bind(id)
            .bind(ctx.org_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| AppError::not_found("job"))?,
    ))
}

pub async fn site_costs(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let site = load_site(&state, ctx.org_id, id).await?;
    let spent = workflows::month_spend(&state, id).await?;
    let by_task: Vec<(String, String, i64, f64)> = sqlx::query_as(
        "select task, model, count(*), coalesce(sum(cost_usd), 0) from ai_calls where site_id = $1 and created_at >= date_trunc('month', $2::timestamptz)
         group by task, model order by 4 desc",
    )
    .bind(id)
    .bind(state.clock.now())
    .fetch_all(&state.db)
    .await?;
    let org_total: Option<f64> = sqlx::query_scalar("select sum(cost_usd) from ai_calls where org_id = $1 and created_at >= date_trunc('month', $2::timestamptz)")
        .bind(ctx.org_id)
        .bind(state.clock.now())
        .fetch_one(&state.db)
        .await?;
    Ok(Json(json!({
        "month_spend_usd": spent,
        "monthly_budget_usd": site.monthly_budget_usd,
        "drafting_paused": site.drafting_paused,
        "org_month_spend_usd": org_total.unwrap_or(0.0),
        "by_task": by_task.iter().map(|(t, m, c, s)| json!({ "task": t, "model": m, "calls": c, "cost_usd": s })).collect::<Vec<_>>(),
    })))
}

pub async fn list_digests(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let digests = sqlx::query_as::<_, Digest>("select id, site_id, period_start, period_end, items, created_at from digests where site_id = $1 order by period_end desc limit 20")
        .bind(id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(json!({ "digests": digests })))
}
