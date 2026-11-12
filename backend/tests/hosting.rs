//! Slices 2 and 3: hosted blogs, SEO and AI-agent outputs, custom domains, plus Clerk
//! token verification against keys the tests sign with.

mod common;

use axum::http::StatusCode;
use chrono::{Duration, TimeZone, Utc};
use common::*;
use kwil::clock::Clock;
use serde_json::{json, Value};
use std::sync::Arc;

/// A site with one published post. Returns (site, post).
async fn published(app: &TestApp) -> (Value, Value) {
    let site_id = site_with_topics(app).await;
    let (piece, post) = first_draft(app, &site_id).await;
    app.post(&format!("/api/posts/{}/approve", post["id"].as_str().unwrap()), json!({})).await;
    let date = chrono::NaiveDate::parse_from_str(piece["scheduled_for"].as_str().unwrap(), "%Y-%m-%d").unwrap();
    app.advance_to(Utc.from_utc_datetime(&date.and_hms_opt(10, 0, 0).unwrap())).await;
    let site = app.get(&format!("/api/sites/{site_id}")).await;
    let post = app.get(&format!("/api/posts/{}", post["id"].as_str().unwrap())).await["post"].clone();
    assert_eq!(post["status"], "published");
    (site, post)
}

#[tokio::test]
async fn every_site_gets_a_subdomain_blog_with_seo_and_agent_outputs() {
    let app = TestApp::new().await;
    let (site, post) = published(&app).await;
    let host = "ledgerly.blog.test";
    let slug = post["slug"].as_str().unwrap();
    assert_eq!(site["blog_url"], "https://ledgerly.blog.test");

    let (status, home) = app.blog_get(host, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(home.contains(post["title"].as_str().unwrap()));
    assert!(home.contains("rel=\"canonical\" href=\"https://ledgerly.blog.test/\""));

    let (_, page) = app.blog_get(host, &format!("/{slug}")).await;
    assert!(page.contains(&format!("<link rel=\"canonical\" href=\"https://ledgerly.blog.test/{slug}\">")));
    assert!(page.contains("og:title") && page.contains("twitter:card"));
    assert!(page.contains("\"@type\":\"Article\"") && page.contains("\"@type\":\"BreadcrumbList\""));
    assert!(page.contains(&format!("/{slug}.md")), "links to the Markdown version");

    let (status, md) = app.blog_get(host, &format!("/{slug}.md")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(md.starts_with(&format!("# {}", post["title"].as_str().unwrap())));
    assert!(!md.contains("[[E"));

    let (_, sitemap) = app.blog_get(host, "/sitemap.xml").await;
    assert!(sitemap.contains(&format!("<loc>https://ledgerly.blog.test/{slug}</loc>")));
    let (_, rss) = app.blog_get(host, "/rss.xml").await;
    assert!(rss.contains("<rss version=\"2.0\"") && rss.contains(&format!("https://ledgerly.blog.test/{slug}")));
    let (_, robots) = app.blog_get(host, "/robots.txt").await;
    assert!(robots.contains("Sitemap: https://ledgerly.blog.test/sitemap.xml"));
    let (_, llms) = app.blog_get(host, "/llms.txt").await;
    assert!(llms.starts_with("# Ledgerly") && llms.contains(&format!("https://ledgerly.blog.test/{slug}.md")));

    let topic_id = post["topic_id"].as_str();
    if let Some(topic_id) = topic_id {
        let topics = app.get(&format!("/api/sites/{}/topics", site["id"].as_str().unwrap())).await;
        let t = topics["topics"].as_array().unwrap().iter().find(|t| t["id"] == json!(topic_id)).unwrap();
        let (status, tp) = app.blog_get(host, &format!("/topic/{}", t["slug"].as_str().unwrap())).await;
        assert_eq!(status, StatusCode::OK);
        assert!(tp.contains(post["title"].as_str().unwrap()));
    }
    let (status, author) = app.blog_get(host, "/author/the-ledgerly-team").await;
    assert_eq!(status, StatusCode::OK);
    assert!(author.contains(post["title"].as_str().unwrap()));
}

#[tokio::test]
async fn unpublished_and_other_tenants_content_never_appears() {
    let app = TestApp::new().await;
    let (site, post) = published(&app).await;
    let site_id = site["id"].as_str().unwrap();

    // A draft in review on the same site is invisible.
    let cal = app.get(&format!("/api/sites/{site_id}/calendar")).await;
    let next = cal["pieces"].as_array().unwrap().iter().find(|p| p["status"] == "approved").unwrap();
    app.post(&format!("/api/pieces/{}/draft-now", next["id"].as_str().unwrap()), json!({})).await;
    app.run_jobs().await;
    let drafts = app.get(&format!("/api/sites/{site_id}/posts?status=in_review")).await;
    let draft = &drafts["posts"][0];
    let (status, _) = app.blog_get("ledgerly.blog.test", &format!("/{}", draft["slug"].as_str().unwrap())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, sitemap) = app.blog_get("ledgerly.blog.test", "/sitemap.xml").await;
    assert!(!sitemap.contains(draft["slug"].as_str().unwrap()));

    // Another tenant's site cannot show this tenant's post.
    let (status, other) = app.request_as(&OTHER_TENANT, "POST", "/api/sites", Some(json!({ "name": "Other Co" }))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app.blog_get(&format!("{}.blog.test", other["slug"].as_str().unwrap()), &format!("/{}", post["slug"].as_str().unwrap())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app.blog_get("unknown.blog.test", "/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Unpublishing removes it and clears the cache.
    app.post(&format!("/api/posts/{}/unpublish", post["id"].as_str().unwrap()), json!({})).await;
    let (status, _) = app.blog_get("ledgerly.blog.test", &format!("/{}", post["slug"].as_str().unwrap())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn publishing_and_editing_clear_the_posts_cache() {
    let app = TestApp::new().await;
    let (_, post) = published(&app).await;
    let slug = post["slug"].as_str().unwrap().to_string();
    let purged = app.cdn.purged.lock().unwrap().clone();
    assert!(purged.contains(&format!("https://ledgerly.blog.test/{slug}")));
    assert!(purged.contains(&"https://ledgerly.blog.test/sitemap.xml".to_string()));
    app.cdn.purged.lock().unwrap().clear();

    let body = format!("{}\nAn added clarification about milestone billing for agencies.\n", post["body_md"].as_str().unwrap());
    app.patch(&format!("/api/posts/{}", post["id"].as_str().unwrap()), json!({ "body_md": body })).await;
    assert!(app.cdn.purged.lock().unwrap().contains(&format!("https://ledgerly.blog.test/{slug}")));
    let (_, page) = app.blog_get("ledgerly.blog.test", &format!("/{slug}")).await;
    assert!(page.contains("An added clarification"));
}

#[tokio::test]
async fn custom_domains_go_through_the_cdn_and_become_canonical() {
    let app = TestApp::new().await;
    let (site, post) = published(&app).await;
    let site_id = site["id"].as_str().unwrap();
    let added = app.post(&format!("/api/sites/{site_id}/domains"), json!({ "hostname": "Blog.Ledgerly.com" })).await;
    assert_eq!(added["domain"]["hostname"], "blog.ledgerly.com");
    assert_eq!(added["domain"]["status"], "pending");
    assert!(added["instructions"].as_str().unwrap().contains("CNAME"));
    assert_eq!(app.cdn.hostnames.lock().unwrap().clone(), vec!["blog.ledgerly.com".to_string()]);

    // Pending domains are not served yet.
    let (status, _) = app.blog_get("blog.ledgerly.com", "/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The scheduled tick checks status; once active the domain serves and is canonical.
    app.tick().await;
    let domains = app.get(&format!("/api/sites/{site_id}/domains")).await;
    assert_eq!(domains["domains"][0]["status"], "active");
    let slug = post["slug"].as_str().unwrap();
    let (status, page) = app.blog_get("blog.ledgerly.com", &format!("/{slug}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(&format!("href=\"https://blog.ledgerly.com/{slug}\"")));
    let (_, sub) = app.blog_get("ledgerly.blog.test", &format!("/{slug}")).await;
    assert!(sub.contains(&format!("rel=\"canonical\" href=\"https://blog.ledgerly.com/{slug}\"")), "the subdomain points at the custom domain");

    let (status, _) = app.request_as(&OWNER, "POST", &format!("/api/sites/{site_id}/domains"), Some(json!({ "hostname": "blog.ledgerly.com" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = app.request_as(&OWNER, "POST", &format!("/api/sites/{site_id}/domains"), Some(json!({ "hostname": "not a domain" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = app.request_as(&OTHER_TENANT, "DELETE", &format!("/api/domains/{}", domains["domains"][0]["id"].as_str().unwrap()), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn branding_and_fixed_pages_shape_the_blog() {
    let app = TestApp::new().await;
    let (site, _) = published(&app).await;
    let site_id = site["id"].as_str().unwrap();
    app.patch(
        &format!("/api/sites/{site_id}"),
        json!({ "branding": { "primary_color": "#ff5500", "font": "serif", "logo_url": "https://cdn.test/logo.png",
                              "nav": [{ "label": "Product", "url": "https://ledgerly.com" }], "footer": "Made by Ledgerly" } }),
    )
    .await;
    app.post(&format!("/api/sites/{site_id}/pages"), json!({ "title": "About", "body_md": "We build invoicing for agencies." })).await;
    let (status, _) = app.request_as(&OWNER, "POST", &format!("/api/sites/{site_id}/pages"), Some(json!({ "title": "Topic", "slug": "topic" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "reserved paths are refused");

    let (_, home) = app.blog_get("ledgerly.blog.test", "/").await;
    assert!(home.contains("--p:#ff5500") && home.contains("Georgia"));
    assert!(home.contains("https://cdn.test/logo.png") && home.contains("Made by Ledgerly"));
    assert!(home.contains("href=\"/about\"") && home.contains("href=\"https://ledgerly.com\""));
    let (status, about) = app.blog_get("ledgerly.blog.test", "/about").await;
    assert_eq!(status, StatusCode::OK);
    assert!(about.contains("We build invoicing for agencies."));

    // Unsafe colours are ignored.
    app.patch(&format!("/api/sites/{site_id}"), json!({ "branding": { "primary_color": "red;}</style><script>" } })).await;
    let (_, home) = app.blog_get("ledgerly.blog.test", "/").await;
    assert!(!home.contains("<script>") && home.contains("--p:#2563eb"));
}

#[tokio::test]
async fn posts_publish_only_once_their_time_comes() {
    let app = TestApp::new().await;
    let site_id = site_with_topics(&app).await;
    let (_, post) = first_draft(&app, &site_id).await;
    let later = app.clock.now() + Duration::days(3);
    app.post(&format!("/api/posts/{}/approve", post["id"].as_str().unwrap()), json!({ "publish_at": later.to_rfc3339() })).await;
    app.tick().await;
    let (status, _) = app.blog_get("ledgerly.blog.test", &format!("/{}", post["slug"].as_str().unwrap())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    app.advance_to(later + Duration::minutes(1)).await;
    let (status, _) = app.blog_get("ledgerly.blog.test", &format!("/{}", post["slug"].as_str().unwrap())).await;
    assert_eq!(status, StatusCode::OK);
}

// ---------------------------------------------------------------- Clerk

fn clerk_token(claims: Value, kid: &str) -> String {
    let pem = std::fs::read("tests/fixtures/test_rsa.pem").unwrap();
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(&pem).unwrap();
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(kid.into());
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
}

async fn clerk_app() -> TestApp {
    let jwks: jsonwebtoken::jwk::JwkSet = serde_json::from_str(&std::fs::read_to_string("tests/fixtures/test_jwks.json").unwrap()).unwrap();
    let auth = kwil::auth::ClerkAuth::new(kwil::auth::KeySource::Static(jwks), Some("https://clerk.test".into()));
    TestApp::with_auth(Arc::new(auth)).await
}

async fn call(app: &TestApp, token: Option<&str>, path: &str) -> (StatusCode, String) {
    let mut req = axum::http::Request::builder().uri(path);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    app.raw(req.body(axum::body::Body::empty()).unwrap()).await
}

#[tokio::test]
async fn clerk_session_tokens_are_verified_and_scope_the_organization() {
    let app = clerk_app().await;
    let exp = (Utc::now() + Duration::hours(1)).timestamp();
    let good = clerk_token(json!({ "sub": "user_1", "iss": "https://clerk.test", "exp": exp, "o": { "id": "org_A", "rol": "admin", "slg": "acme" } }), "test-key");
    let (status, me) = call(&app, Some(&good), "/api/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let me: Value = serde_json::from_str(&me).unwrap();
    assert_eq!(me["org_external_id"], "org_A");
    assert_eq!(me["role"], "owner");

    // Version 1 claims, member role.
    let v1 = clerk_token(json!({ "sub": "user_2", "iss": "https://clerk.test", "exp": exp, "org_id": "org_A", "org_role": "org:member" }), "test-key");
    let (_, me) = call(&app, Some(&v1), "/api/me").await;
    assert_eq!(serde_json::from_str::<Value>(&me).unwrap()["role"], "member");

    let expired = clerk_token(json!({ "sub": "user_1", "iss": "https://clerk.test", "exp": exp - 7200, "o": { "id": "org_A" } }), "test-key");
    assert_eq!(call(&app, Some(&expired), "/api/me").await.0, StatusCode::UNAUTHORIZED);
    let wrong_issuer = clerk_token(json!({ "sub": "user_1", "iss": "https://evil.test", "exp": exp, "o": { "id": "org_A" } }), "test-key");
    assert_eq!(call(&app, Some(&wrong_issuer), "/api/me").await.0, StatusCode::UNAUTHORIZED);
    let unknown_key = clerk_token(json!({ "sub": "user_1", "iss": "https://clerk.test", "exp": exp, "o": { "id": "org_A" } }), "other-key");
    assert_eq!(call(&app, Some(&unknown_key), "/api/me").await.0, StatusCode::UNAUTHORIZED);
    let no_org = clerk_token(json!({ "sub": "user_1", "iss": "https://clerk.test", "exp": exp }), "test-key");
    assert_eq!(call(&app, Some(&no_org), "/api/me").await.0, StatusCode::FORBIDDEN);
    assert_eq!(call(&app, None, "/api/me").await.0, StatusCode::UNAUTHORIZED);
    // Development headers mean nothing when Clerk is on.
    let req = axum::http::Request::builder().uri("/api/me").header("x-dev-user", "dev-user").body(axum::body::Body::empty()).unwrap();
    assert_eq!(app.raw(req).await.0, StatusCode::UNAUTHORIZED);
}
