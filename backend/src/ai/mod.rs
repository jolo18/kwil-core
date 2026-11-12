//! Typed AI tasks. Each task builds a prompt from structured context, calls the provider for
//! the site's model at the right tier, records the cost, and parses a JSON answer.

pub mod demo;
pub mod llm;

use crate::state::AppState;
use anyhow::{Context, Result};
use llm::{LlmRequest, Tier};
use serde::de::DeserializeOwned;
use serde_json::Value;
use uuid::Uuid;

/// The model names a site has chosen for each tier.
#[derive(Clone, Debug)]
pub struct Models {
    pub draft: String,
    pub mid: String,
    pub cheap: String,
}

impl Models {
    pub fn for_tier(&self, tier: Tier) -> &str {
        match tier {
            Tier::Draft => &self.draft,
            Tier::Mid => &self.mid,
            Tier::Cheap => &self.cheap,
        }
    }
}

/// Who an AI call is billed to.
#[derive(Clone, Copy, Debug)]
pub struct Scope {
    pub org_id: Uuid,
    pub site_id: Uuid,
}

const SYSTEM: &str = "You are the research and writing engine of Kwil, a tool that writes SEO and AI-search content for B2B SaaS companies. \
The company's own thinking lives in a knowledge library of short entries, each with an ID like E12. \
Never invent facts, opinions, customers or numbers: use only what the supplied entries say. \
Respond with one JSON object exactly matching the schema in the instructions, and nothing else.";

fn instructions(task: &str) -> &'static str {
    match task {
        "interview_followups" => "Read the interview answers. For answers that are vague or short, ask a sharper follow-up question. At most 3. \
Schema: {\"questions\": [{\"topic\": string, \"prompt\": string}]}",
        "extract_entries" => "Turn the passages into atomic library entries: one claim each, under 80 words, readable on its own. \
kind is one of fact, feature, audience, positioning, opinion, story, quote, competitor, avoid. quote must be copied exactly from the passage. \
Schema: {\"entries\": [{\"kind\": string, \"body\": string, \"passage_index\": number, \"quote\": string}]}",
        "propose_topics" => "Propose a topic map: 3 to 6 pillar topics the company should own, each with 2 to 5 subtopics, covering breadth and depth. \
Include topics without search volume when they help buyers evaluate a choice. \
Schema: {\"topics\": [{\"title\": string, \"description\": string, \"children\": [{\"title\": string, \"description\": string}]}]}",
        "propose_calendar" => "Plan one piece per date. Prefer topics with little coverage. content_type is one of how_to, buyer_guide, alternatives, comparison, listicle, explainer, report, opinion, case_study. \
If allow_comparison is false, do not use alternatives, comparison or listicle. Titles must not repeat existing_titles. \
Schema: {\"pieces\": [{\"topic_id\": string, \"content_type\": string, \"title\": string, \"angle\": string, \"target_keyword\": string, \"timeliness\": \"timely\"|\"timeless\"}]}",
        "write_brief" => "Write a brief for the piece using only the supplied entries. Pick the entries the piece should use. \
Schema: {\"title\": string, \"angle\": string, \"outline\": [string], \"entry_nums\": [number], \"meta_description\": string}",
        "coverage_questions" => "The library has too little to say about this piece. Ask the owner 2 or 3 short, specific questions whose answers would let the piece be written in their own voice. \
Schema: {\"questions\": [string]}",
        "draft" => "Write the post in Markdown following the brief. Rules: answer the question in the first paragraph with no warm-up; \
use clear ## headings and short sections; every paragraph must make sense on its own (never start with 'As mentioned' or refer back); \
alternatives and comparison pieces need a comparison table; cite every claim with the entry ID in double brackets like [[E12]]; \
cite only the supplied entries. Do not include the title as a heading. \
Schema: {\"title\": string, \"meta_description\": string, \"body_md\": string}",
        "verify_claim" => "Decide strictly whether the quoted passage supports the claim. Schema: {\"supported\": boolean, \"reason\": string}",
        "review_draft" => "Review the draft against the cited entries as a strict editor. Fail it for unsupported claims, generic filler, or anything off-brand. \
Schema: {\"pass\": boolean, \"issues\": [string]}",
        "extract_graph" => "Identify the entities these entries are about and how they relate. Entity kinds: product, feature, persona, use_case, competitor, integration, topic. \
Edge kinds: has_feature, serves, competes_with, alternative_to, integrates_with, part_of_topic. Reuse existing entity names exactly. \
Schema: {\"entities\": [{\"kind\": string, \"name\": string}], \"edges\": [{\"from\": {\"kind\": string, \"name\": string}, \"to\": {\"kind\": string, \"name\": string}, \"kind\": string}], \"about\": [{\"entry_num\": number, \"entity\": {\"kind\": string, \"name\": string}}]}",
        "detect_conflicts" => "Compare the new entry with the candidates. Report only clear relationships: contradicts (they cannot both be true), refines (adds detail), replaces (makes it obsolete). \
Schema: {\"links\": [{\"to_num\": number, \"kind\": string, \"note\": string}]}",
        "learn_from_edit" => "The owner edited an AI draft. Find what the edits reveal about what they actually think or know that the library may lack. Each becomes a proposed entry. \
Schema: {\"entries\": [{\"kind\": string, \"body\": string, \"quote\": string}]}",
        "tag_topics" => "Tag each entry with the topics it is relevant to. Schema: {\"tags\": [{\"entry_num\": number, \"topic_id\": string}]}",
        _ => "Schema: {}",
    }
}

fn parse_json<T: DeserializeOwned>(text: &str) -> Result<T> {
    let trimmed = text.trim();
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim_end_matches("```").trim())
        .unwrap_or(trimmed);
    let start = body.find('{').unwrap_or(0);
    let end = body.rfind('}').map(|i| i + 1).unwrap_or(body.len());
    serde_json::from_str(&body[start..end]).with_context(|| format!("model returned invalid JSON: {text}"))
}

pub async fn load_models(db: &sqlx::PgPool, site_id: Uuid) -> Result<Models> {
    let (draft, mid, cheap): (String, String, String) =
        sqlx::query_as("select model_draft, model_mid, model_cheap from sites where id = $1")
            .bind(site_id)
            .fetch_one(db)
            .await?;
    Ok(Models { draft, mid, cheap })
}

/// Runs one AI task and records what it cost against the organization and site.
pub async fn run<T: DeserializeOwned>(
    state: &AppState,
    scope: Scope,
    task: &str,
    tier: Tier,
    context: Value,
    cached_prefix: Option<String>,
) -> Result<T> {
    let models = load_models(&state.db, scope.site_id).await?;
    let model = models.for_tier(tier).to_string();
    let user = format!(
        "{}\n\nInput:\n{}",
        instructions(task),
        serde_json::to_string_pretty(&context).unwrap_or_default()
    );
    let req = LlmRequest {
        task: task.to_string(),
        tier,
        model: model.clone(),
        system: SYSTEM.to_string(),
        cached_prefix,
        user,
        context,
    };
    let resp = state.llm.complete(&req).await.with_context(|| format!("AI task {task} failed"))?;
    sqlx::query(
        "insert into ai_calls (id, org_id, site_id, task, tier, model, input_tokens, output_tokens, cost_usd, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(Uuid::new_v4())
    .bind(scope.org_id)
    .bind(scope.site_id)
    .bind(task)
    .bind(tier.as_str())
    .bind(&model)
    .bind(resp.input_tokens)
    .bind(resp.output_tokens)
    .bind(resp.cost_usd)
    .bind(state.clock.now())
    .execute(&state.db)
    .await?;
    parse_json(&resp.text)
}
