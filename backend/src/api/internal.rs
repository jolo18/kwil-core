//! Endpoints called by Google Cloud (Scheduler, Tasks) and by Clerk webhooks.

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use uuid::Uuid;

fn check_secret(state: &AppState, headers: &HeaderMap) -> AppResult<()> {
    let given = headers.get("x-internal-secret").and_then(|v| v.to_str().ok()).unwrap_or("");
    if given.is_empty() || given != state.config.internal_secret {
        return Err(AppError::unauthorized("bad internal secret"));
    }
    Ok(())
}

pub async fn cron_tick(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    check_secret(&state, &headers)?;
    Ok(Json(crate::workflows::tick(&state).await?))
}

pub async fn run_job(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    check_secret(&state, &headers)?;
    let ran = crate::jobs::run_one(&state, id).await?;
    Ok(Json(json!({ "ran": ran })))
}

/// Verifies a Svix signature as used by Clerk webhooks.
pub fn verify_svix(secret: &str, id: &str, timestamp: &str, body: &[u8], signatures: &str) -> bool {
    use base64::Engine;
    let key = secret.strip_prefix("whsec_").unwrap_or(secret);
    let Ok(key) = base64::engine::general_purpose::STANDARD.decode(key) else { return false };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(&key) else { return false };
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    let expected = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    signatures.split_whitespace().filter_map(|s| s.strip_prefix("v1,")).any(|s| s == expected)
}

pub async fn clerk_webhook(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> AppResult<Json<Value>> {
    let secret = state.config.clerk_webhook_secret.clone().ok_or_else(|| AppError::unauthorized("webhooks are not configured"))?;
    let h = |n: &str| headers.get(n).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let (id, ts, sig) = (h("svix-id"), h("svix-timestamp"), h("svix-signature"));
    let fresh = ts.parse::<i64>().map(|t| (state.clock.now().timestamp() - t).abs() < 300).unwrap_or(false);
    if !fresh || !verify_svix(&secret, &id, &ts, &body, &sig) {
        return Err(AppError::unauthorized("invalid webhook signature"));
    }
    let event: Value = serde_json::from_slice(&body).map_err(|e| AppError::bad_request(e.to_string()))?;
    let data = &event["data"];
    let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
    match event["type"].as_str().unwrap_or_default() {
        "organization.created" | "organization.updated" => {
            sqlx::query(
                "insert into organizations (id, external_id, name) values ($1, $2, $3) on conflict (external_id) do update set name = excluded.name",
            )
            .bind(Uuid::new_v4())
            .bind(s(&data["id"]))
            .bind(s(&data["name"]))
            .execute(&state.db)
            .await?;
        }
        "organization.deleted" => {
            sqlx::query("delete from organizations where external_id = $1").bind(s(&data["id"])).execute(&state.db).await?;
        }
        "user.created" | "user.updated" => {
            let email = data["email_addresses"].as_array().and_then(|a| a.first()).map(|e| s(&e["email_address"]));
            let name = format!("{} {}", s(&data["first_name"]), s(&data["last_name"])).trim().to_string();
            sqlx::query(
                "insert into users (id, external_id, email, name) values ($1, $2, $3, $4)
                 on conflict (external_id) do update set email = excluded.email, name = excluded.name",
            )
            .bind(Uuid::new_v4())
            .bind(s(&data["id"]))
            .bind(email)
            .bind(name)
            .execute(&state.db)
            .await?;
        }
        "organizationMembership.created" | "organizationMembership.updated" => {
            let org = s(&data["organization"]["id"]);
            let user = s(&data["public_user_data"]["user_id"]);
            let role = if s(&data["role"]) == "org:admin" { "owner" } else { "member" };
            sqlx::query("insert into organizations (id, external_id, name) values ($1, $2, $3) on conflict (external_id) do nothing")
                .bind(Uuid::new_v4())
                .bind(&org)
                .bind(s(&data["organization"]["name"]))
                .execute(&state.db)
                .await?;
            sqlx::query("insert into users (id, external_id, email) values ($1, $2, $3) on conflict (external_id) do nothing")
                .bind(Uuid::new_v4())
                .bind(&user)
                .bind(data["public_user_data"]["identifier"].as_str())
                .execute(&state.db)
                .await?;
            sqlx::query(
                "insert into memberships (org_id, user_id, role)
                 select o.id, u.id, $3 from organizations o, users u where o.external_id = $1 and u.external_id = $2
                 on conflict (org_id, user_id) do update set role = excluded.role",
            )
            .bind(&org)
            .bind(&user)
            .bind(role)
            .execute(&state.db)
            .await?;
        }
        "organizationMembership.deleted" => {
            sqlx::query(
                "delete from memberships where org_id = (select id from organizations where external_id = $1)
                 and user_id = (select id from users where external_id = $2)",
            )
            .bind(s(&data["organization"]["id"]))
            .bind(s(&data["public_user_data"]["user_id"]))
            .execute(&state.db)
            .await?;
        }
        _ => {}
    }
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svix_signatures() {
        use base64::Engine;
        let secret = format!("whsec_{}", base64::engine::general_purpose::STANDARD.encode(b"supersecretkey"));
        let mut mac = Hmac::<Sha256>::new_from_slice(b"supersecretkey").unwrap();
        mac.update(b"msg_1.1700000000.{\"a\":1}");
        let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        assert!(verify_svix(&secret, "msg_1", "1700000000", b"{\"a\":1}", &format!("v1,{sig}")));
        assert!(!verify_svix(&secret, "msg_1", "1700000000", b"{\"a\":2}", &format!("v1,{sig}")));
    }
}
