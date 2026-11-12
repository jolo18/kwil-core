//! Slice 1: interview to approved entries, topic map, calendar, scheduled drafting with
//! citations, coverage holds, budget, review, structure checks and the inbox.

mod common;

use axum::http::StatusCode;
use chrono::{Duration, TimeZone, Utc};
use common::*;
use serde_json::{json, Value};

#[tokio::test]
async fn interview_answers_become_proposed_entries_that_need_a_person() {
    let app = TestApp::new().await;
    let site = app.post("/api/sites", json!({ "name": "Ledgerly" })).await;
    let site_id = site["id"].as_str().unwrap();
    let questions = app.get(&format!("/api/sites/{site_id}/questions?open=true")).await;
    assert_eq!(questions["questions"].as_array().unwrap().len(), 8, "the interview starts with eight core questions");

    let q = questions["questions"].as_array().unwrap().iter().find(|q| q["topic"] == "opinions").unwrap();
    app.post(&format!("/api/questions/{}/answer", q["id"].as_str().unwrap()), json!({ "answer": answers()[4].1 })).await;
    app.run_jobs().await;

    let entries = app.get(&format!("/api/sites/{site_id}/entries?status=proposed")).await;
    let entries = entries["entries"].as_array().unwrap();
    assert!(!entries.is_empty());
    for e in entries {
        assert_eq!(e["kind"], "opinion");
        assert_eq!(e["gate"], "person", "opinions always need a person");
        assert_eq!(e["sources"][0]["kind"], "interview");
    }

    let inbox = app.get("/api/inbox").await;
    assert_eq!(inbox["sites"][0]["entries"].as_array().unwrap().len(), entries.len());

    // Approving puts the entry into the site profile that every prompt carries.
    let id = entries[0]["id"].as_str().unwrap();
    let approved = app.post(&format!("/api/entries/{id}/approve"), json!({})).await;
    assert_eq!(approved["approved_by"], "person");
    let profile = app.get(&format!("/api/sites/{site_id}/profile")).await;
    assert!(profile["profile_md"].as_str().unwrap().contains(&format!("[E{}]", approved["num"])));
}

#[tokio::test]
async fn short_answers_get_a_follow_up_question() {
    let app = TestApp::new().await;
    let site = app.post("/api/sites", json!({ "name": "Ledgerly" })).await;
    let site_id = site["id"].as_str().unwrap();
    let questions = app.get(&format!("/api/sites/{site_id}/questions?open=true")).await;
    let q = &questions["questions"][0];
    app.post(&format!("/api/questions/{}/answer", q["id"].as_str().unwrap()), json!({ "answer": "Invoicing for agencies." })).await;
    app.run_jobs().await;
    let open = app.get(&format!("/api/sites/{site_id}/questions?open=true")).await;
    assert!(open["questions"].as_array().unwrap().iter().any(|q| q["kind"] == "followup"));
}

#[tokio::test]
async fn tenants_cannot_see_each_others_data() {
    let app = TestApp::new().await;
    let site_id = onboarded_site(&app).await;
    let (status, _) = app.request_as(&OTHER_TENANT, "GET", &format!("/api/sites/{site_id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app.request_as(&OTHER_TENANT, "GET", &format!("/api/sites/{site_id}/entries"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let entries = app.get(&format!("/api/sites/{site_id}/entries")).await;
    let entry_id = entries["entries"][0]["id"].as_str().unwrap();
    let (status, _) = app.request_as(&OTHER_TENANT, "POST", &format!("/api/entries/{entry_id}/approve"), Some(json!({}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, sites) = app.request_as(&OTHER_TENANT, "GET", "/api/sites", None).await;
    assert_eq!(sites["sites"].as_array().unwrap().len(), 0);
    let (_, inbox) = app.request_as(&OTHER_TENANT, "GET", "/api/inbox", None).await;
    assert_eq!(inbox["sites"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn members_cannot_create_sites() {
    let app = TestApp::new().await;
    let member = Who { user: "sam", org: "dev-org", role: "member" };
    let (status, _) = app.request_as(&member, "POST", "/api/sites", Some(json!({ "name": "X" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_core_loop_drafts_with_valid_citations_and_publishes_on_schedule() {
    let app = TestApp::new().await;
    let site_id = site_with_topics(&app).await;

    let topics = app.get(&format!("/api/sites/{site_id}/topics")).await;
    assert!(topics["topics"].as_array().unwrap().len() >= 3);
    assert!(topics["topics"].as_array().unwrap().iter().all(|t| t["status"] == "approved"));

    let (piece, post) = first_draft(&app, &site_id).await;
    assert_eq!(app.job_errors().await, vec![], "no job failed");
    assert!(!post.is_null(), "a draft is waiting for review");
    assert_eq!(post["piece_id"], piece["id"]);

    // Every citation points at an approved entry of this site.
    let detail = app.get(&format!("/api/posts/{}", post["id"].as_str().unwrap())).await;
    let body = detail["post"]["body_md"].as_str().unwrap();
    let cited = kwil::text::cited_nums(body);
    assert!(!cited.is_empty());
    let entries = app.get(&format!("/api/sites/{site_id}/entries?status=approved")).await;
    let approved: Vec<i64> = entries["entries"].as_array().unwrap().iter().map(|e| e["num"].as_i64().unwrap()).collect();
    for n in &cited {
        assert!(approved.contains(&(*n as i64)), "E{n} is approved");
    }
    assert_eq!(detail["cited_entries"].as_array().unwrap().len(), cited.len());

    // The draft prompt carried the site profile as the cached prefix.
    let call = app.llm.calls_for("draft").pop().unwrap();
    assert!(call.cached_prefix.unwrap().contains("Ledgerly"));
    assert_eq!(call.model, "anthropic/claude-opus-5.5");

    // The editor shows citations and structure findings; edits are revisions.
    let edited = format!("{body}\nA new closing thought from the owner about milestone billing for agencies.\n");
    app.patch(&format!("/api/posts/{}", post["id"].as_str().unwrap()), json!({ "body_md": edited })).await;
    let detail = app.get(&format!("/api/posts/{}", post["id"].as_str().unwrap())).await;
    assert_eq!(detail["revisions"].as_array().unwrap().len(), 2);

    // Approving schedules it for the planned date; it publishes when the time comes.
    let approved_post = app.post(&format!("/api/posts/{}/approve", post["id"].as_str().unwrap()), json!({})).await;
    assert_eq!(approved_post["status"], "scheduled");
    let date = chrono::NaiveDate::parse_from_str(piece["scheduled_for"].as_str().unwrap(), "%Y-%m-%d").unwrap();
    app.advance_to(Utc.from_utc_datetime(&date.and_hms_opt(9, 0, 1).unwrap())).await;
    let published = app.get(&format!("/api/posts/{}", post["id"].as_str().unwrap())).await;
    assert_eq!(published["post"]["status"], "published");

    // The hosted blog shows it, without citation markers.
    let slug = published["post"]["slug"].as_str().unwrap();
    let (status, html) = app.blog_get("ledgerly.blog.test", &format!("/{slug}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(published["post"]["title"].as_str().unwrap()));
    assert!(!html.contains("[[E"));
    assert!(html.contains("application/ld+json"));

    // Markdown download strips citations and adds front matter.
    let req = axum::http::Request::builder()
        .uri(format!("/api/posts/{}/markdown", post["id"].as_str().unwrap()))
        .header("x-dev-user", "dev-user")
        .header("x-dev-org", "dev-org")
        .body(axum::body::Body::empty())
        .unwrap();
    let (status, md) = app.raw(req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(md.starts_with("---\ntitle:"));
    assert!(!md.contains("[[E"));
}

#[tokio::test]
async fn calendar_follows_cadence_and_warns_about_comparison_heavy_mixes() {
    let app = TestApp::new().await;
    let site_id = site_with_topics(&app).await;
    app.patch(&format!("/api/sites/{site_id}"), json!({ "cadence_per_week": 5 })).await;
    app.post(&format!("/api/sites/{site_id}/calendar/propose"), json!({})).await;
    app.run_jobs().await;
    let cal = app.get(&format!("/api/sites/{site_id}/calendar")).await;
    let pieces = cal["pieces"].as_array().unwrap();
    // Weekdays from tomorrow through three weeks out.
    assert_eq!(pieces.len(), 15);
    assert!(pieces.iter().all(|p| p["status"] == "proposed"));
    assert!(cal["mix"]["share"].as_f64().unwrap() <= 0.25, "the planner keeps comparisons in proportion: {}", cal["mix"]);

    // Changing several items into comparisons triggers the warning.
    for p in pieces.iter().take(6) {
        app.patch(&format!("/api/pieces/{}", p["id"].as_str().unwrap()), json!({ "content_type": "alternatives" })).await;
    }
    let cal = app.get(&format!("/api/sites/{site_id}/calendar")).await;
    assert!(cal["mix"]["warning"].is_string());

    // Nothing is drafted until the calendar is approved.
    app.advance_to(common::start_time() + Duration::days(2)).await;
    let posts = app.get(&format!("/api/sites/{site_id}/posts")).await;
    assert_eq!(posts["posts"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_thin_library_holds_the_piece_and_asks_questions_instead_of_writing_filler() {
    let app = TestApp::new().await;
    let site = app.post("/api/sites", json!({ "name": "Ledgerly" })).await;
    let site_id = site["id"].as_str().unwrap();
    // Only facts, no opinion or position yet.
    app.post(&format!("/api/sites/{site_id}/entries"), json!({ "kind": "fact", "body": "Ledgerly syncs invoices to accounting software within a minute." })).await;
    app.post(&format!("/api/sites/{site_id}/entries"), json!({ "kind": "feature", "body": "Invoices go out automatically when a milestone is marked done." })).await;
    let topic = app.post(&format!("/api/sites/{site_id}/topics"), json!({ "title": "Milestone invoicing", "description": "Billing agencies by milestone" })).await;
    app.run_jobs().await;

    let piece = sqlx::query_scalar::<_, uuid::Uuid>(
        "insert into pieces (id, org_id, site_id, topic_id, content_type, title, angle, scheduled_for, status)
         select gen_random_uuid(), org_id, id, $2, 'how_to', 'How to start milestone invoicing', '', $3, 'approved' from sites where id = $1 returning id",
    )
    .bind(uuid::Uuid::parse_str(site_id).unwrap())
    .bind(uuid::Uuid::parse_str(topic["id"].as_str().unwrap()).unwrap())
    .bind(common::start_time().date_naive())
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    app.tick().await;

    let cal = app.get(&format!("/api/sites/{site_id}/calendar")).await;
    let p = cal["pieces"].as_array().unwrap().iter().find(|p| p["id"] == json!(piece)).unwrap().clone();
    assert_eq!(p["status"], "held");
    assert!(app.llm.calls_for("draft").is_empty(), "nothing was drafted");
    let inbox = app.get("/api/inbox").await;
    let questions: Vec<Value> = inbox["sites"][0]["questions"].as_array().unwrap().iter().filter(|q| q["kind"] == "coverage").cloned().collect();
    assert!(!questions.is_empty() && questions.len() <= 3);

    // Answering, then approving what came out of the answers, releases the piece.
    for q in &questions {
        app.post(
            &format!("/api/questions/{}/answer", q["id"].as_str().unwrap()),
            json!({ "answer": "We think milestone invoicing is better than hourly billing because agencies should be paid for outcomes. Milestone invoicing keeps cash flow predictable for small agencies." }),
        )
        .await;
    }
    app.run_jobs().await;
    approve_all_entries(&app, site_id).await;
    app.tick().await;
    let cal = app.get(&format!("/api/sites/{site_id}/calendar")).await;
    let p = cal["pieces"].as_array().unwrap().iter().find(|p| p["id"] == json!(piece)).unwrap().clone();
    assert_eq!(p["status"], "drafted", "{p}");
    assert_eq!(app.job_errors().await, vec![]);
}

#[tokio::test]
async fn drafts_citing_unsupplied_entries_are_rejected_and_the_piece_fails_after_retries() {
    let app = TestApp::new().await;
    let site_id = site_with_topics(&app).await;
    for _ in 0..3 {
        app.llm.push("draft", json!({ "title": "Bad", "meta_description": "x", "body_md": "Invented claim [[E999]].\n" }));
    }
    app.post(&format!("/api/sites/{site_id}/calendar/propose"), json!({})).await;
    app.run_jobs().await;
    app.post(&format!("/api/sites/{site_id}/calendar/approve"), json!({})).await;
    let cal = app.get(&format!("/api/sites/{site_id}/calendar")).await;
    let piece_id = cal["pieces"][0]["id"].as_str().unwrap().to_string();
    app.post(&format!("/api/pieces/{piece_id}/draft-now"), json!({})).await;
    // Run the first attempt, then let the retries come due.
    for _ in 0..3 {
        app.run_jobs().await;
        app.clock.advance_days(1);
    }
    app.run_jobs().await;
    let errors = app.job_errors().await;
    assert!(errors.iter().any(|(k, e)| k == "draft_piece" && e.contains("E999")), "{errors:?}");
    let cal = app.get(&format!("/api/sites/{site_id}/calendar")).await;
    let p = cal["pieces"].as_array().unwrap().iter().find(|p| p["id"] == json!(piece_id)).unwrap().clone();
    assert_eq!(p["status"], "failed");
    let posts = app.get(&format!("/api/sites/{site_id}/posts")).await;
    assert_eq!(posts["posts"].as_array().unwrap().len(), 0, "no draft with invalid citations was saved");
    let inbox = app.get("/api/inbox").await;
    assert!(inbox["sites"][0]["notifications"].as_array().unwrap().iter().any(|n| n["kind"] == "job_failed"));
}

#[tokio::test]
async fn spending_the_budget_pauses_drafting_until_it_is_raised() {
    let app = TestApp::new().await;
    let site_id = site_with_topics(&app).await;
    let costs = app.get(&format!("/api/sites/{site_id}/costs")).await;
    let spent = costs["month_spend_usd"].as_f64().unwrap();
    assert!(spent > 0.0, "every AI call is costed");
    assert!(!costs["by_task"].as_array().unwrap().is_empty());
    app.patch(&format!("/api/sites/{site_id}"), json!({ "monthly_budget_usd": spent })).await;

    let (_, post) = first_draft(&app, &site_id).await;
    assert!(post.is_null(), "nothing is drafted over budget");
    let site = app.get(&format!("/api/sites/{site_id}")).await;
    assert_eq!(site["drafting_paused"], true);
    let inbox = app.get("/api/inbox").await;
    assert!(inbox["sites"][0]["notifications"].as_array().unwrap().iter().any(|n| n["kind"] == "budget_paused"));

    app.patch(&format!("/api/sites/{site_id}"), json!({ "monthly_budget_usd": spent + 10.0 })).await;
    app.tick().await;
    let posts = app.get(&format!("/api/sites/{site_id}/posts?status=in_review")).await;
    assert_eq!(posts["posts"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn structure_checks_flag_problems_in_the_editor() {
    let app = TestApp::new().await;
    let v = app.post("/api/check", json!({ "body_md": "In today's world, invoicing matters.\n\nAs mentioned above, it is hard.\n", "content_type": "alternatives" })).await;
    let rules: Vec<&str> = v["findings"].as_array().unwrap().iter().map(|f| f["rule"].as_str().unwrap()).collect();
    assert!(rules.contains(&"intro_answer_first"));
    assert!(rules.contains(&"paragraph_stands_alone"));
    assert!(rules.contains(&"comparison_table"));
}

#[tokio::test]
async fn people_cannot_cite_unapproved_entries_when_editing() {
    let app = TestApp::new().await;
    let site_id = site_with_topics(&app).await;
    let (_, post) = first_draft(&app, &site_id).await;
    let (status, v) = app.request_as(&OWNER, "PATCH", &format!("/api/posts/{}", post["id"].as_str().unwrap()), Some(json!({ "body_md": "Claim [[E4040]].\n" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
}

#[tokio::test]
async fn entry_edits_are_versioned_and_revertible() {
    let app = TestApp::new().await;
    let site = app.post("/api/sites", json!({ "name": "Ledgerly" })).await;
    let site_id = site["id"].as_str().unwrap();
    let e = app.post(&format!("/api/sites/{site_id}/entries"), json!({ "kind": "fact", "body": "The Pro plan costs $49 a month." })).await;
    let id = e["id"].as_str().unwrap();
    app.patch(&format!("/api/entries/{id}"), json!({ "body": "The Pro plan costs $59 a month." })).await;
    let reverted = app.post(&format!("/api/entries/{id}/revert"), json!({ "version": 1 })).await;
    assert_eq!(reverted["body"], "The Pro plan costs $49 a month.");
    let detail = app.get(&format!("/api/entries/{id}")).await;
    assert_eq!(detail["versions"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn the_development_login_is_refused_in_production() {
    let mut config = kwil::config::Config::for_tests("postgres://x/y");
    config.env = kwil::config::Env::Production;
    config.internal_secret = "real".into();
    assert!(config.validate().is_err());
    config.auth_mode = "clerk".into();
    config.clerk_jwks_url = Some("https://example.clerk.accounts.dev/.well-known/jwks.json".into());
    assert!(config.validate().is_ok());
}

#[tokio::test]
async fn internal_endpoints_need_the_shared_secret() {
    let app = TestApp::new().await;
    let req = axum::http::Request::builder().method("POST").uri("/internal/cron/tick").body(axum::body::Body::empty()).unwrap();
    let (status, _) = app.raw(req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
