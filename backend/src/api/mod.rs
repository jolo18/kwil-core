pub mod content;
pub mod hosting;
pub mod internal;
pub mod library;

use crate::auth::Ctx;
use crate::error::AppResult;
use crate::state::AppState;
use axum::extract::State;
use axum::http::{HeaderValue, Method};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tower_http::cors::{AllowHeaders, CorsLayer};

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/me", get(me))
        // Sites, interview and library
        .route("/sites", get(library::list_sites).post(library::create_site))
        .route("/sites/{id}", get(library::get_site).patch(library::update_site))
        .route("/sites/{id}/profile", get(library::get_profile))
        .route("/sites/{id}/questions", get(library::list_questions))
        .route("/questions/{id}/answer", post(library::answer_question))
        .route("/sites/{id}/entries", get(library::list_entries).post(library::create_entry))
        .route("/sites/{id}/entries/approve", post(library::bulk_approve))
        .route("/entries/{id}", get(library::get_entry).patch(library::update_entry))
        .route("/entries/{id}/approve", post(library::approve_entry))
        .route("/entries/{id}/reject", post(library::reject_entry))
        .route("/entries/{id}/revert", post(library::revert_entry))
        .route("/sites/{id}/conflicts", get(library::list_conflicts))
        .route("/links/{id}/resolve", post(library::resolve_link))
        .route("/sites/{id}/graph", get(library::get_graph))
        .route("/entities/{id}/approve", post(library::approve_entity))
        .route("/entities/{id}/reject", post(library::reject_entity))
        .route("/entities/{id}/merge", post(library::merge_entity))
        .route("/edges/{id}/approve", post(library::approve_edge))
        .route("/edges/{id}/reject", post(library::reject_edge))
        .route("/sites/{id}/crawl", post(library::start_crawl))
        .route("/sites/{id}/uploads", get(library::list_uploads).post(library::upload))
        // Planning, drafting and review
        .route("/sites/{id}/topics", get(content::list_topics).post(content::create_topic))
        .route("/sites/{id}/topics/propose", post(content::propose_topics))
        .route("/sites/{id}/topics/approve", post(content::approve_all_topics))
        .route("/topics/{id}", patch(content::update_topic).delete(content::delete_topic))
        .route("/topics/{id}/approve", post(content::approve_topic))
        .route("/topics/{id}/reject", post(content::reject_topic))
        .route("/sites/{id}/calendar", get(content::get_calendar))
        .route("/sites/{id}/calendar/propose", post(content::propose_calendar))
        .route("/sites/{id}/calendar/approve", post(content::approve_calendar))
        .route("/pieces/{id}", patch(content::update_piece))
        .route("/pieces/{id}/approve", post(content::approve_piece))
        .route("/pieces/{id}/reject", post(content::reject_piece))
        .route("/pieces/{id}/draft-now", post(content::draft_now))
        .route("/sites/{id}/posts", get(content::list_posts))
        .route("/posts/{id}", get(content::get_post).patch(content::update_post))
        .route("/posts/{id}/approve", post(content::approve_post))
        .route("/posts/{id}/reject", post(content::reject_post))
        .route("/posts/{id}/redraft", post(content::redraft_post))
        .route("/posts/{id}/unpublish", post(content::unpublish_post))
        .route("/posts/{id}/markdown", get(content::post_markdown))
        .route("/posts/{id}/github", post(content::commit_to_github))
        .route("/check", post(content::check_structure))
        .route("/inbox", get(content::inbox))
        .route("/notifications/{id}/read", post(content::read_notification))
        .route("/sites/{id}/jobs", get(content::list_jobs))
        .route("/jobs/{id}", get(content::get_job))
        .route("/sites/{id}/costs", get(content::site_costs))
        .route("/sites/{id}/digests", get(content::list_digests))
        // Hosting
        .route("/sites/{id}/domains", get(hosting::list_domains).post(hosting::add_domain))
        .route("/domains/{id}", delete(hosting::remove_domain))
        .route("/domains/{id}/check", post(hosting::check_domain))
        .route("/sites/{id}/pages", get(hosting::list_pages).post(hosting::create_page))
        .route("/pages/{id}", patch(hosting::update_page).delete(hosting::delete_page));

    let origin = state.config.cors_origin.clone();
    let cors = CorsLayer::new()
        .allow_origin(origin.parse::<HeaderValue>().unwrap_or(HeaderValue::from_static("http://localhost:5173")))
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE, Method::OPTIONS])
        .allow_headers(AllowHeaders::mirror_request())
        .allow_credentials(true);

    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api", api)
        .route("/internal/cron/tick", post(internal::cron_tick))
        .route("/internal/jobs/{id}/run", post(internal::run_job))
        .route("/webhooks/clerk", post(internal::clerk_webhook))
        .layer(cors)
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

async fn me(State(state): State<AppState>, ctx: Ctx) -> AppResult<Json<Value>> {
    let (org_name, ext): (String, String) = sqlx::query_as("select name, external_id from organizations where id = $1")
        .bind(ctx.org_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(json!({
        "user_id": ctx.user_id,
        "email": ctx.email,
        "org_id": ctx.org_id,
        "org_name": org_name,
        "org_external_id": ext,
        "role": ctx.role,
        "auth_mode": state.config.auth_mode,
        "ai_provider": state.config.ai_provider,
    })))
}
