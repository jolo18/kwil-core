//! Custom domains and fixed pages for hosted blogs.

use crate::auth::Ctx;
use crate::error::{AppError, AppResult};
use crate::library::load_site;
use crate::models::{Domain, FixedPage};
use crate::state::AppState;
use crate::text::slugify;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

fn valid_hostname(h: &str) -> bool {
    let labels: Vec<&str> = h.split('.').collect();
    h.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

pub async fn list_domains(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let domains = sqlx::query_as::<_, Domain>("select id, site_id, hostname, status, provider_id, verification, created_at from domains where site_id = $1 order by created_at")
        .bind(id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(json!({ "domains": domains, "cname_target": state.config.cloudflare_cname_target })))
}

#[derive(Deserialize)]
pub struct NewDomain {
    hostname: String,
}

pub async fn add_domain(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<NewDomain>) -> AppResult<Json<Value>> {
    ctx.require_owner()?;
    let site = load_site(&state, ctx.org_id, id).await?;
    let hostname = b.hostname.trim().trim_end_matches('.').to_lowercase();
    let hostname = hostname.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string();
    if !valid_hostname(&hostname) {
        return Err(AppError::bad_request("enter a domain like blog.example.com"));
    }
    if hostname.ends_with(&format!(".{}", state.config.blog_base_domain)) {
        return Err(AppError::bad_request("that is already one of our subdomains"));
    }
    let taken: bool = sqlx::query_scalar("select exists(select 1 from domains where hostname = $1)").bind(&hostname).fetch_one(&state.db).await?;
    if taken {
        return Err(AppError::conflict("that domain is already connected to a site"));
    }
    let status = state.cdn.add_hostname(&hostname).await.map_err(|e| AppError::bad_request(format!("could not register the domain: {e}")))?;
    let domain = sqlx::query_as::<_, Domain>(
        "insert into domains (id, org_id, site_id, hostname, status, provider_id, verification, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8) returning id, site_id, hostname, status, provider_id, verification, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(site.org_id)
    .bind(site.id)
    .bind(&hostname)
    .bind(if status.active { "active" } else { "pending" })
    .bind(&status.provider_id)
    .bind(&status.verification)
    .bind(state.clock.now())
    .fetch_one(&state.db)
    .await?;
    Ok(Json(json!({
        "domain": domain,
        "instructions": format!("Add a CNAME record for {hostname} pointing to {}. HTTPS is issued automatically once the record is live.", state.config.cloudflare_cname_target),
    })))
}

async fn load_domain(state: &AppState, ctx: &Ctx, id: Uuid) -> AppResult<Domain> {
    sqlx::query_as::<_, Domain>("select id, site_id, hostname, status, provider_id, verification, created_at from domains where id = $1 and org_id = $2")
        .bind(id)
        .bind(ctx.org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("domain"))
}

pub async fn remove_domain(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    ctx.require_owner()?;
    let d = load_domain(&state, &ctx, id).await?;
    if let Some(pid) = &d.provider_id {
        state.cdn.remove_hostname(pid).await.map_err(|e| AppError::bad_request(format!("could not remove the domain: {e}")))?;
    }
    sqlx::query("delete from domains where id = $1").bind(id).execute(&state.db).await?;
    Ok(Json(json!({ "removed": true })))
}

pub async fn check_domain(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Domain>> {
    let d = load_domain(&state, &ctx, id).await?;
    if let Some(pid) = &d.provider_id {
        let s = state.cdn.hostname_status(pid).await.map_err(|e| AppError::bad_request(e.to_string()))?;
        let status = if s.active { "active" } else if s.failed { "failed" } else { "pending" };
        sqlx::query("update domains set status = $2, verification = $3 where id = $1").bind(id).bind(status).bind(&s.verification).execute(&state.db).await?;
    }
    Ok(Json(load_domain(&state, &ctx, id).await?))
}

// ---------------------------------------------------------------- fixed pages

pub async fn list_pages(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    load_site(&state, ctx.org_id, id).await?;
    let pages = sqlx::query_as::<_, FixedPage>("select * from fixed_pages where site_id = $1 order by nav_position, title").bind(id).fetch_all(&state.db).await?;
    Ok(Json(json!({ "pages": pages })))
}

#[derive(Deserialize)]
pub struct PageBody {
    title: Option<String>,
    slug: Option<String>,
    body_md: Option<String>,
    nav_position: Option<i32>,
}

async fn check_page_slug(state: &AppState, site_id: Uuid, slug: &str, except: Option<Uuid>) -> AppResult<()> {
    if crate::workflows::RESERVED_SLUGS.contains(&slug) {
        return Err(AppError::bad_request(format!("/{slug} is used by the blog itself")));
    }
    let taken: bool = sqlx::query_scalar(
        "select exists(select 1 from fixed_pages where site_id = $1 and slug = $2 and id is distinct from $3) or exists(select 1 from posts where site_id = $1 and slug = $2)",
    )
    .bind(site_id)
    .bind(slug)
    .bind(except)
    .fetch_one(&state.db)
    .await?;
    if taken {
        return Err(AppError::conflict(format!("/{slug} is already used by another page or post")));
    }
    Ok(())
}

pub async fn create_page(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<PageBody>) -> AppResult<Json<FixedPage>> {
    let site = load_site(&state, ctx.org_id, id).await?;
    let title = b.title.unwrap_or_default();
    if title.trim().is_empty() {
        return Err(AppError::bad_request("a page needs a title"));
    }
    let slug = slugify(b.slug.as_deref().unwrap_or(&title));
    check_page_slug(&state, site.id, &slug, None).await?;
    Ok(Json(
        sqlx::query_as::<_, FixedPage>(
            "insert into fixed_pages (id, org_id, site_id, slug, title, body_md, nav_position, created_at) values ($1, $2, $3, $4, $5, $6, $7, $8) returning *",
        )
        .bind(Uuid::new_v4())
        .bind(site.org_id)
        .bind(site.id)
        .bind(&slug)
        .bind(title.trim())
        .bind(b.body_md.unwrap_or_default())
        .bind(b.nav_position.unwrap_or(0))
        .bind(state.clock.now())
        .fetch_one(&state.db)
        .await?,
    ))
}

pub async fn update_page(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<PageBody>) -> AppResult<Json<FixedPage>> {
    let page = sqlx::query_as::<_, FixedPage>("select * from fixed_pages where id = $1 and org_id = $2")
        .bind(id)
        .bind(ctx.org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("page"))?;
    let slug = b.slug.as_deref().map(slugify);
    if let Some(s) = &slug {
        check_page_slug(&state, page.site_id, s, Some(page.id)).await?;
    }
    let updated = sqlx::query_as::<_, FixedPage>(
        "update fixed_pages set title = coalesce($2, title), slug = coalesce($3, slug), body_md = coalesce($4, body_md), nav_position = coalesce($5, nav_position)
         where id = $1 returning *",
    )
    .bind(id)
    .bind(b.title)
    .bind(slug)
    .bind(b.body_md)
    .bind(b.nav_position)
    .fetch_one(&state.db)
    .await?;
    let site = load_site(&state, ctx.org_id, page.site_id).await?;
    let host = format!("https://{}.{}", site.slug, state.config.blog_base_domain);
    state.cdn.purge(&[format!("{host}/{}", page.slug), format!("{host}/{}", updated.slug), format!("{host}/")]).await.ok();
    Ok(Json(updated))
}

pub async fn delete_page(State(state): State<AppState>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let n = sqlx::query("delete from fixed_pages where id = $1 and org_id = $2").bind(id).bind(ctx.org_id).execute(&state.db).await?.rows_affected();
    if n == 0 {
        return Err(AppError::not_found("page"));
    }
    Ok(Json(json!({ "deleted": true })))
}
