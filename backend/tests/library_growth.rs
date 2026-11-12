//! Slices 4 to 8: crawl, the AI gate, conflicts, the knowledge graph, uploads, learning from
//! edits, the weekly digest, automatic publishing, re-crawls and the GitHub publisher.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Duration;
use common::*;
use kwil::clock::Clock;
use serde_json::{json, Value};

fn site_pages(app: &TestApp, price: &str) {
    app.fetcher.set("https://ledgerly.test/robots.txt", "User-agent: *\nDisallow: /admin\nSitemap: https://ledgerly.test/sitemap.xml\n");
    app.fetcher.set(
        "https://ledgerly.test/sitemap.xml",
        "<urlset><url><loc>https://ledgerly.test/pricing</loc></url><url><loc>https://ledgerly.test/admin/secret</loc></url><url><loc>https://other.test/x</loc></url></urlset>",
    );
    app.fetcher.set(
        "https://ledgerly.test/pricing",
        &format!("<html><head><title>Pricing</title></head><body><nav><p>Home Pricing Blog Login Signup Contact</p></nav><main>\
<h1>Simple pricing for agencies</h1>\
<p>The Pro plan costs {price} per month and includes unlimited invoices for agencies.</p>\
<p>We believe agencies should never pay per invoice because billing volume is not value.</p>\
</main></body></html>"),
    );
}

async fn entries(app: &TestApp, site_id: &str, status: &str) -> Vec<Value> {
    app.get(&format!("/api/sites/{site_id}/entries?status={status}")).await["entries"].as_array().unwrap().clone()
}

#[tokio::test]
async fn crawled_facts_with_evidence_pass_the_ai_gate_but_opinions_wait_for_a_person() {
    let app = TestApp::new().await;
    let site = app.post("/api/sites", json!({ "name": "Ledgerly", "website_url": "ledgerly.test" })).await;
    let site_id = site["id"].as_str().unwrap();
    site_pages(&app, "$49");
    app.post(&format!("/api/sites/{site_id}/crawl"), json!({})).await;
    app.run_jobs().await;
    assert_eq!(app.job_errors().await, vec![]);

    let requested = app.fetcher.requested.lock().unwrap().clone();
    assert!(!requested.iter().any(|u| u.contains("/admin")), "robots.txt is respected");
    assert!(!requested.iter().any(|u| u.contains("other.test")), "only the site's own host is crawled");

    let approved = entries(&app, site_id, "approved").await;
    let price = approved.iter().find(|e| e["body"].as_str().unwrap().contains("$49")).expect("the pricing fact was approved");
    assert_eq!(price["approved_by"], "ai");
    assert_eq!(price["sources"][0]["kind"], "crawl");
    assert!(price["sources"][0]["quote"].as_str().unwrap().contains("$49"));

    let proposed = entries(&app, site_id, "proposed").await;
    let opinion = proposed.iter().find(|e| e["kind"] == "opinion").expect("the opinion waits");
    assert_eq!(opinion["gate"], "person");

    // Re-crawling unchanged pages adds nothing.
    let before = approved.len() + proposed.len();
    app.post(&format!("/api/sites/{site_id}/crawl"), json!({})).await;
    app.run_jobs().await;
    let after = entries(&app, site_id, "approved").await.len() + entries(&app, site_id, "proposed").await.len();
    assert_eq!(before, after);
}

#[tokio::test]
async fn strict_sites_send_everything_to_a_person() {
    let app = TestApp::new().await;
    let site = app.post("/api/sites", json!({ "name": "Ledgerly", "website_url": "https://ledgerly.test" })).await;
    let site_id = site["id"].as_str().unwrap();
    app.patch(&format!("/api/sites/{site_id}"), json!({ "strict_gates": true })).await;
    site_pages(&app, "$49");
    app.post(&format!("/api/sites/{site_id}/crawl"), json!({})).await;
    app.run_jobs().await;
    assert!(entries(&app, site_id, "approved").await.is_empty());
    assert!(entries(&app, site_id, "proposed").await.iter().all(|e| e["gate"] == "person"));
}

#[tokio::test]
async fn ai_cannot_override_a_person_and_conflicts_go_to_the_inbox() {
    let app = TestApp::new().await;
    let site = app.post("/api/sites", json!({ "name": "Ledgerly", "website_url": "https://ledgerly.test" })).await;
    let site_id = site["id"].as_str().unwrap();
    let mine = app.post(&format!("/api/sites/{site_id}/entries"), json!({ "kind": "fact", "body": "The Pro plan costs $39 per month and includes unlimited invoices for agencies." })).await;
    app.run_jobs().await;

    site_pages(&app, "$49");
    app.post(&format!("/api/sites/{site_id}/crawl"), json!({})).await;
    app.run_jobs().await;

    let proposed = entries(&app, site_id, "proposed").await;
    let crawled = proposed.iter().find(|e| e["body"].as_str().unwrap().contains("$49")).expect("the new price waits");
    assert_eq!(crawled["gate"], "person");
    assert!(crawled["gate_reason"].as_str().unwrap().contains(&format!("E{}", mine["num"])));

    let conflicts = app.get(&format!("/api/sites/{site_id}/conflicts")).await;
    let conflict = &conflicts["conflicts"][0];
    assert_eq!(conflict["kind"], "contradicts");
    assert_eq!(conflict["existing"]["num"], mine["num"]);
    let inbox = app.get("/api/inbox").await;
    assert_eq!(inbox["sites"][0]["conflicts"], 1);

    // The AI gate itself refuses a person-gated entry.
    let entry = sqlx::query_as::<_, kwil::models::Entry>("select * from entries where id = $1")
        .bind(uuid::Uuid::parse_str(crawled["id"].as_str().unwrap()).unwrap())
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert!(kwil::library::approve(&app.state, &entry, kwil::library::Gate::Ai, "ai").await.is_err());

    // The person keeps the new price; the old one is superseded.
    app.post(&format!("/api/links/{}/resolve", conflict["id"].as_str().unwrap()), json!({ "action": "keep_new" })).await;
    let old = app.get(&format!("/api/entries/{}", mine["id"].as_str().unwrap())).await;
    assert_eq!(old["status"], "superseded");
    let profile = app.get(&format!("/api/sites/{site_id}/profile")).await;
    assert!(!profile["profile_md"].as_str().unwrap().contains("$39"));
}

#[tokio::test]
async fn approved_entries_build_a_knowledge_graph_and_competitor_edges_need_a_person() {
    let app = TestApp::new().await;
    let site_id = site_with_topics(&app).await;
    let graph = app.get(&format!("/api/sites/{site_id}/graph")).await;
    let entities = graph["entities"].as_array().unwrap();
    let product = entities.iter().find(|e| e["kind"] == "product").expect("the product entity");
    assert_eq!(product["name"], "Ledgerly");
    assert!(entities.iter().any(|e| e["kind"] == "topic"), "approved topics become entities");
    let competitor = entities.iter().find(|e| e["kind"] == "competitor").expect("a competitor was found");
    let edges = graph["edges"].as_array().unwrap();
    let competes = edges.iter().find(|e| e["kind"] == "competes_with" && e["to_entity"] == competitor["id"]).expect("a competes_with edge");
    assert_eq!(competes["status"], "proposed", "who competes with whom is a person's call");
    assert!(edges.iter().any(|e| e["status"] == "approved"), "other edges are approved by the AI gate");

    app.post(&format!("/api/edges/{}/approve", competes["id"].as_str().unwrap()), json!({})).await;
    let graph = app.get(&format!("/api/sites/{site_id}/graph")).await;
    assert!(graph["edges"].as_array().unwrap().iter().any(|e| e["kind"] == "competes_with" && e["status"] == "approved"));
}

#[tokio::test]
async fn graph_retrieval_feeds_alternatives_pieces_from_the_named_competitor() {
    let app = TestApp::new().await;
    let site_id = site_with_topics(&app).await;
    let graph = app.get(&format!("/api/sites/{site_id}/graph")).await;
    let competitor = graph["entities"].as_array().unwrap().iter().find(|e| e["kind"] == "competitor").unwrap().clone();
    let name = competitor["name"].as_str().unwrap();
    let p = sqlx::query_as::<_, kwil::models::Piece>(
        "insert into pieces (id, org_id, site_id, content_type, title, angle, scheduled_for, status)
         select gen_random_uuid(), org_id, id, 'alternatives', $2, '', $3, 'approved' from sites where id = $1 returning *",
    )
    .bind(uuid::Uuid::parse_str(&site_id).unwrap())
    .bind(format!("The best {name} alternatives"))
    .bind(start_time().date_naive())
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    let found = kwil::retrieval::entries_for_piece(&app.state, &p).await.unwrap();
    assert!(found.iter().any(|e| e.kind == "competitor"), "entries about the competitor are retrieved");
}

#[tokio::test]
async fn uploads_and_transcripts_become_proposed_entries() {
    let app = TestApp::new().await;
    let site = app.post("/api/sites", json!({ "name": "Ledgerly" })).await;
    let site_id = site["id"].as_str().unwrap();
    let boundary = "XBOUNDARY";
    let body = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"pricing.md\"\r\nContent-Type: text/markdown\r\n\r\n\
The Team plan includes unlimited invoices and automatic payment reminders for every client.\r\n\
--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"call.vtt\"\r\nContent-Type: text/vtt\r\n\r\n\
WEBVTT\n\n1\n00:00:01.000 --> 00:00:05.000\nCustomer: We spent a whole day every month chasing late invoices before switching.\n\r\n--{b}--\r\n",
        b = boundary
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sites/{site_id}/uploads"))
        .header("x-dev-user", "dev-user")
        .header("x-dev-org", "dev-org")
        .header("content-type", format!("multipart/form-data; boundary={boundary}"))
        .body(Body::from(body))
        .unwrap();
    let (status, out) = app.raw(req).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    app.run_jobs().await;
    assert_eq!(app.job_errors().await, vec![]);

    let uploads = app.get(&format!("/api/sites/{site_id}/uploads")).await;
    assert!(uploads["uploads"].as_array().unwrap().iter().all(|u| u["status"] == "processed"));
    let all = [entries(&app, site_id, "approved").await, entries(&app, site_id, "proposed").await].concat();
    let doc = all.iter().find(|e| e["body"].as_str().unwrap().contains("Team plan")).expect("an entry from the document");
    assert_eq!(doc["sources"][0]["kind"], "upload");
    assert_eq!(doc["approved_by"], "ai", "a feature from your own document passes the AI gate");
    let quote = all.iter().find(|e| e["body"].as_str().unwrap().contains("chasing late invoices")).expect("an entry from the transcript");
    assert_eq!(quote["sources"][0]["kind"], "transcript");
    assert_eq!(quote["gate"], "person", "customer words need a person");

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sites/{site_id}/uploads"))
        .header("x-dev-user", "dev-user")
        .header("x-dev-org", "dev-org")
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from("--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"x.exe\"\r\n\r\nMZ\r\n--B--\r\n"))
        .unwrap();
    assert_eq!(app.raw(req).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn edits_to_drafts_teach_the_library_through_a_person() {
    let app = TestApp::new().await;
    let site_id = site_with_topics(&app).await;
    let (_, post) = first_draft(&app, &site_id).await;
    let post_id = post["id"].as_str().unwrap();
    let detail = app.get(&format!("/api/posts/{post_id}")).await;
    let body = detail["post"]["body_md"].as_str().unwrap();
    let edited = format!("{body}\nHonestly, we believe agencies that invoice weekly get paid faster than agencies that invoice monthly.\n");
    app.patch(&format!("/api/posts/{post_id}"), json!({ "body_md": edited })).await;
    app.post(&format!("/api/posts/{post_id}/approve"), json!({})).await;
    app.run_jobs().await;
    let proposed = entries(&app, &site_id, "proposed").await;
    let learned = proposed.iter().find(|e| e["body"].as_str().unwrap().contains("invoice weekly")).expect("the edit became a proposal");
    assert_eq!(learned["sources"][0]["kind"], "draft_edit");
    assert_eq!(learned["gate"], "person");
}

#[tokio::test]
async fn automatic_review_publishes_drafts_that_pass_every_check() {
    let app = TestApp::new().await;
    let site_id = site_with_topics(&app).await;
    app.patch(&format!("/api/sites/{site_id}"), json!({ "review_mode": "auto" })).await;
    let (piece, _) = first_draft(&app, &site_id).await;
    let posts = app.get(&format!("/api/sites/{site_id}/posts")).await;
    let post = posts["posts"].as_array().unwrap().iter().find(|p| p["piece_id"] == piece["id"]).unwrap().clone();
    assert_eq!(post["status"], "scheduled", "notes: {}", post["review_notes"]);
    assert!(!app.llm.calls_for("review_draft").is_empty(), "a separate AI reviewer ran");

    // A reviewer failure keeps the next one in the queue for a person.
    app.llm.push("review_draft", json!({ "pass": false, "issues": ["Sounds generic."] }));
    let cal = app.get(&format!("/api/sites/{site_id}/calendar")).await;
    let second = cal["pieces"][1]["id"].as_str().unwrap().to_string();
    app.post(&format!("/api/pieces/{second}/draft-now"), json!({})).await;
    app.run_jobs().await;
    let posts = app.get(&format!("/api/sites/{site_id}/posts?status=in_review")).await;
    let held = posts["posts"].as_array().unwrap().iter().find(|p| p["piece_id"] == json!(second)).expect("held for a person");
    assert!(held["review_notes"].as_str().unwrap().contains("generic"));
}

#[tokio::test]
async fn the_weekly_digest_lists_what_the_ai_approved_and_emails_owners() {
    let app = TestApp::new().await;
    let site = app.post("/api/sites", json!({ "name": "Ledgerly", "website_url": "https://ledgerly.test" })).await;
    let site_id = site["id"].as_str().unwrap();
    site_pages(&app, "$49");
    app.post(&format!("/api/sites/{site_id}/crawl"), json!({})).await;
    app.run_jobs().await;
    app.advance_to(start_time() + Duration::days(8)).await;
    let digests = app.get(&format!("/api/sites/{site_id}/digests")).await;
    let d = &digests["digests"][0];
    assert!(d["items"]["entries"].as_array().unwrap().iter().any(|e| e["body"].as_str().unwrap().contains("$49")));
    let emails: Vec<(String, String)> = sqlx::query_as("select to_address, body from email_outbox").fetch_all(&app.state.db).await.unwrap();
    assert_eq!(emails.len(), 1);
    assert_eq!(emails[0].0, "dev-user@example.com");
    assert!(emails[0].1.contains("Review or revert"));
}

#[tokio::test]
async fn sites_are_recrawled_on_schedule_after_the_first_crawl() {
    let app = TestApp::new().await;
    let site = app.post("/api/sites", json!({ "name": "Ledgerly", "website_url": "https://ledgerly.test" })).await;
    let site_id = site["id"].as_str().unwrap();
    site_pages(&app, "$49");
    app.post(&format!("/api/sites/{site_id}/crawl"), json!({})).await;
    app.run_jobs().await;
    site_pages(&app, "$59");
    app.advance_to(start_time() + Duration::days(11)).await;
    let all = [entries(&app, site_id, "approved").await, entries(&app, site_id, "proposed").await].concat();
    let new_price = all.iter().find(|e| e["body"].as_str().unwrap().contains("$59")).expect("the changed price was picked up");
    // It contradicts the AI-approved $49, so a person decides.
    assert_eq!(new_price["gate"], "person");
}

#[tokio::test]
async fn published_posts_are_committed_to_github_when_configured() {
    let app = TestApp::new().await;
    let site_id = site_with_topics(&app).await;
    app.patch(&format!("/api/sites/{site_id}"), json!({ "github_repo": "acme/blog", "github_token": "ghp_test", "github_folder": "src/content/blog" })).await;
    let site = app.get(&format!("/api/sites/{site_id}")).await;
    assert!(site.get("github_token").is_none(), "the token is never returned");
    assert_eq!(site["github_token_set"], true);
    let (_, post) = first_draft(&app, &site_id).await;
    app.post(&format!("/api/posts/{}/approve", post["id"].as_str().unwrap()), json!({ "publish_at": app.clock.now().to_rfc3339() })).await;
    app.run_jobs().await;
    let files = app.github.files.lock().unwrap().clone();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, "acme/blog");
    assert!(files[0].1.starts_with("src/content/blog/") && files[0].1.ends_with(".md"));
    assert!(files[0].2.starts_with("---\ntitle:") && !files[0].2.contains("[[E"));
}
