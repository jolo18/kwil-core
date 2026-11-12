//! Authentication behind one interface: a development identity for local work and
//! Clerk session tokens for real sign-in. Selection is by configuration only.

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Member,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Member => "member",
        }
    }
}

/// Who is calling, as reported by the identity provider.
#[derive(Clone, Debug)]
pub struct Identity {
    pub user_external_id: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub org_external_id: String,
    pub org_name: Option<String>,
    pub role: Role,
}

#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(&self, headers: &HeaderMap) -> AppResult<Identity>;
}

/// Local development only: a fixed user and organization. Tests may pick other
/// identities with the `x-dev-user`, `x-dev-org` and `x-dev-role` headers.
pub struct DevAuth;

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(|s| s.to_string()).filter(|s| !s.is_empty())
}

#[async_trait]
impl Authenticator for DevAuth {
    async fn authenticate(&self, headers: &HeaderMap) -> AppResult<Identity> {
        let user = header(headers, "x-dev-user").unwrap_or_else(|| "dev-user".into());
        let org = header(headers, "x-dev-org").unwrap_or_else(|| "dev-org".into());
        let role = match header(headers, "x-dev-role").as_deref() {
            Some("member") => Role::Member,
            _ => Role::Owner,
        };
        Ok(Identity {
            email: Some(format!("{user}@example.com")),
            name: Some(if user == "dev-user" { "Dev User".into() } else { user.clone() }),
            org_name: Some(if org == "dev-org" { "Dev Organization".into() } else { org.clone() }),
            user_external_id: user,
            org_external_id: org,
            role,
        })
    }
}

/// Where Clerk's public keys come from.
pub enum KeySource {
    Url(String),
    Static(JwkSet),
}

/// Verifies Clerk session tokens (RS256) against Clerk's published JWKS.
pub struct ClerkAuth {
    source: KeySource,
    issuer: Option<String>,
    http: reqwest::Client,
    cache: RwLock<Option<(JwkSet, Instant)>>,
}

#[derive(Debug, Deserialize)]
struct ClerkOrgClaim {
    id: String,
    #[serde(default)]
    rol: Option<String>,
    #[serde(default)]
    slg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClerkClaims {
    sub: String,
    /// Session token v2 organization claim.
    #[serde(default)]
    o: Option<ClerkOrgClaim>,
    /// Session token v1 organization claims.
    #[serde(default)]
    org_id: Option<String>,
    #[serde(default)]
    org_role: Option<String>,
    #[serde(default)]
    org_slug: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

impl ClerkAuth {
    pub fn new(source: KeySource, issuer: Option<String>) -> Self {
        Self { source, issuer, http: reqwest::Client::new(), cache: RwLock::new(None) }
    }

    async fn keys(&self, refresh: bool) -> AppResult<JwkSet> {
        match &self.source {
            KeySource::Static(set) => Ok(set.clone()),
            KeySource::Url(url) => {
                if !refresh {
                    if let Some((set, at)) = self.cache.read().await.as_ref() {
                        if at.elapsed() < Duration::from_secs(3600) {
                            return Ok(set.clone());
                        }
                    }
                }
                let set: JwkSet = self
                    .http
                    .get(url)
                    .send()
                    .await
                    .and_then(|r| r.error_for_status())
                    .map_err(|e| AppError::unauthorized(format!("could not fetch signing keys: {e}")))?
                    .json()
                    .await
                    .map_err(|e| AppError::unauthorized(format!("invalid signing keys: {e}")))?;
                *self.cache.write().await = Some((set.clone(), Instant::now()));
                Ok(set)
            }
        }
    }

    pub async fn verify(&self, token: &str) -> AppResult<Identity> {
        let head = decode_header(token).map_err(|_| AppError::unauthorized("malformed token"))?;
        let kid = head.kid.ok_or_else(|| AppError::unauthorized("token has no key id"))?;
        let mut set = self.keys(false).await?;
        if set.find(&kid).is_none() {
            set = self.keys(true).await?;
        }
        let jwk = set.find(&kid).ok_or_else(|| AppError::unauthorized("unknown signing key"))?;
        let key = DecodingKey::from_jwk(jwk).map_err(|_| AppError::unauthorized("unusable signing key"))?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_aud = false;
        if let Some(iss) = &self.issuer {
            validation.set_issuer(&[iss]);
        }
        let data = decode::<ClerkClaims>(token, &key, &validation)
            .map_err(|e| AppError::unauthorized(format!("invalid token: {e}")))?;
        let c = data.claims;
        let (org_id, role, slug) = match (c.o, c.org_id) {
            (Some(o), _) => (o.id, o.rol, o.slg),
            (None, Some(id)) => (id, c.org_role, c.org_slug),
            (None, None) => return Err(AppError::forbidden("select an organization first")),
        };
        let role = match role.as_deref() {
            Some("org:admin") | Some("admin") => Role::Owner,
            _ => Role::Member,
        };
        Ok(Identity {
            user_external_id: c.sub,
            email: c.email,
            name: c.name,
            org_external_id: org_id,
            org_name: slug,
            role,
        })
    }
}

#[async_trait]
impl Authenticator for ClerkAuth {
    async fn authenticate(&self, headers: &HeaderMap) -> AppResult<Identity> {
        let token = header(headers, "authorization")
            .and_then(|h| h.strip_prefix("Bearer ").map(|s| s.to_string()))
            .ok_or_else(|| AppError::unauthorized("missing bearer token"))?;
        self.verify(&token).await
    }
}

/// The resolved tenant context for a request. Every query on tenant data is scoped by `org_id`.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub role: Role,
    pub email: Option<String>,
}

impl Ctx {
    pub fn require_owner(&self) -> AppResult<()> {
        if self.role == Role::Owner {
            Ok(())
        } else {
            Err(AppError::forbidden("only organization owners can do this"))
        }
    }
}

/// Makes sure the organization, user and membership exist locally, then returns their ids.
pub async fn sync_identity(db: &sqlx::PgPool, id: &Identity) -> AppResult<Ctx> {
    let org_id: Uuid = sqlx::query_scalar(
        "insert into organizations (id, external_id, name) values ($1, $2, $3)
         on conflict (external_id) do update set name = case when organizations.name = '' then excluded.name else organizations.name end
         returning id",
    )
    .bind(Uuid::new_v4())
    .bind(&id.org_external_id)
    .bind(id.org_name.clone().unwrap_or_default())
    .fetch_one(db)
    .await?;
    let user_id: Uuid = sqlx::query_scalar(
        "insert into users (id, external_id, email, name) values ($1, $2, $3, $4)
         on conflict (external_id) do update set email = coalesce(excluded.email, users.email), name = coalesce(excluded.name, users.name)
         returning id",
    )
    .bind(Uuid::new_v4())
    .bind(&id.user_external_id)
    .bind(&id.email)
    .bind(&id.name)
    .fetch_one(db)
    .await?;
    sqlx::query(
        "insert into memberships (org_id, user_id, role) values ($1, $2, $3)
         on conflict (org_id, user_id) do update set role = excluded.role",
    )
    .bind(org_id)
    .bind(user_id)
    .bind(id.role.as_str())
    .execute(db)
    .await?;
    Ok(Ctx { org_id, user_id, role: id.role, email: id.email.clone() })
}

impl FromRequestParts<AppState> for Ctx {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let identity = state.auth.authenticate(&parts.headers).await?;
        sync_identity(&state.db, &identity).await
    }
}

pub fn build_authenticator(config: &crate::config::Config) -> anyhow::Result<Arc<dyn Authenticator>> {
    Ok(match config.auth_mode.as_str() {
        "dev" => Arc::new(DevAuth),
        "clerk" => Arc::new(ClerkAuth::new(
            KeySource::Url(config.clerk_jwks_url.clone().unwrap_or_default()),
            config.clerk_issuer.clone(),
        )),
        other => anyhow::bail!("unknown AUTH_MODE {other}"),
    })
}
