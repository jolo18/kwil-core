//! A deterministic stand-in for the model, used when no OpenRouter key is configured and as
//! the default behaviour of the test fake. It works only from the structured `context` of a
//! request, so its output is predictable and always well-formed.

use super::llm::{LlmProvider, LlmRequest, LlmResponse};
use crate::text::{overlap, sentences, significant_words, stems, title_case, truncate_words};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub struct Demo;

#[async_trait]
impl LlmProvider for Demo {
    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse> {
        let ctx = &req.context;
        let out = match req.task.as_str() {
            "interview_followups" => followups(ctx),
            "extract_entries" => extract_entries(ctx),
            "propose_topics" => propose_topics(ctx),
            "propose_calendar" => propose_calendar(ctx),
            "write_brief" => write_brief(ctx),
            "coverage_questions" => coverage_questions(ctx),
            "draft" => draft(ctx),
            "verify_claim" => verify_claim(ctx),
            "review_draft" => review_draft(ctx),
            "extract_graph" => extract_graph(ctx),
            "detect_conflicts" => detect_conflicts(ctx),
            "learn_from_edit" => learn_from_edit(ctx),
            "tag_topics" => tag_topics(ctx),
            other => return Err(anyhow!("demo provider has no behaviour for task {other}")),
        };
        Ok(LlmResponse { text: out.to_string(), ..Default::default() })
    }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn arr(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

fn followups(ctx: &Value) -> Value {
    let mut questions = Vec::new();
    for a in arr(&ctx["answers"]) {
        let answer = s(&a["answer"]);
        let topic = s(&a["topic"]);
        if answer.split_whitespace().count() < 12 && questions.len() < 3 {
            questions.push(json!({
                "topic": topic,
                "prompt": format!(
                    "You answered \"{}\" about {}. Can you give a concrete example a customer would recognise?",
                    truncate_words(&answer, 12),
                    topic.replace('_', " ")
                )
            }));
        }
    }
    json!({ "questions": questions })
}

const OPINION_MARKERS: &[&str] = &[
    "believe", "think", "should", "never", "always", "overrated", "wrong", "mistake", "better",
    "worse", "hate", "love", "opinion", "honestly", "disagree", "myth",
];

fn classify(sentence: &str, hint: &str, source_kind: &str) -> &'static str {
    let lower = sentence.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    match hint {
        "audience" | "pain_points" => return "audience",
        "positioning" => return "positioning",
        "opinions" => return "opinion",
        "competitors" => return "competitor",
        "stories" => return "story",
        "avoid" => return "avoid",
        _ => {}
    }
    if sentence.contains('"') || sentence.contains('“') {
        return "quote";
    }
    if has(&[" vs ", "unlike", "compared to", "alternative", "competitor"]) {
        return "competitor";
    }
    if has(OPINION_MARKERS) {
        return "opinion";
    }
    if has(&["lets you", "you can", "feature", "integrat", "automat", "dashboard", "export", "import"]) {
        return "feature";
    }
    if source_kind == "transcript" {
        return "quote";
    }
    "fact"
}

fn extract_entries(ctx: &Value) -> Value {
    let hint = s(&ctx["topic"]);
    let source_kind = s(&ctx["source_kind"]);
    let mut entries = Vec::new();
    for p in arr(&ctx["passages"]) {
        let index = p["index"].as_i64().unwrap_or(0);
        let text = s(&p["text"]);
        let mut taken = 0;
        for sentence in sentences(&text) {
            if sentence.split_whitespace().count() < 5 || taken >= 8 {
                continue;
            }
            let body = truncate_words(&sentence, 80);
            entries.push(json!({
                "kind": classify(&sentence, &hint, &source_kind),
                "body": body,
                "passage_index": index,
                "quote": sentence,
            }));
            taken += 1;
        }
    }
    json!({ "entries": entries })
}

fn top_keywords(texts: &[String], n: usize) -> Vec<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for t in texts {
        for w in significant_words(t) {
            if w.chars().count() >= 5 && w.chars().all(|c| c.is_alphabetic()) {
                *counts.entry(w).or_default() += 1;
            }
        }
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.into_iter().take(n).map(|(w, _)| w).collect()
}

fn propose_topics(ctx: &Value) -> Value {
    let texts: Vec<String> = arr(&ctx["entries"]).iter().map(|e| s(&e["body"])).collect();
    let mut keywords = top_keywords(&texts, 3);
    if keywords.is_empty() {
        keywords = vec!["getting started".into()];
    }
    let topics: Vec<Value> = keywords
        .iter()
        .map(|k| {
            let t = title_case(k);
            json!({
                "title": t,
                "description": format!("Everything a buyer needs to understand about {k}."),
                "children": [
                    { "title": format!("Getting started with {k}"), "description": format!("First steps with {k}.") },
                    { "title": format!("Common {k} mistakes"), "description": format!("What teams get wrong about {k}.") },
                    { "title": format!("Choosing a {k} tool"), "description": format!("How to evaluate {k} options.") }
                ]
            })
        })
        .collect();
    json!({ "topics": topics })
}

fn piece_title(content_type: &str, topic: &str) -> String {
    let lower = topic.to_lowercase();
    match content_type {
        "how_to" => format!("How to get real results from {lower}"),
        "explainer" => format!("{topic}, explained in plain terms"),
        "buyer_guide" => format!("A buyer's guide to {lower}"),
        "opinion" => format!("What most teams get wrong about {lower}"),
        "alternatives" => format!("Alternatives for {lower}: an honest comparison"),
        "comparison" => format!("{topic}: comparing your options"),
        "listicle" => format!("7 practical tips for {lower}"),
        "case_study" => format!("How one team improved {lower}"),
        _ => format!("The state of {lower}"),
    }
}

fn propose_calendar(ctx: &Value) -> Value {
    let mut topics = arr(&ctx["topics"]);
    topics.sort_by(|a, b| {
        a["covered"].as_i64().unwrap_or(0).cmp(&b["covered"].as_i64().unwrap_or(0)).then(s(&a["title"]).cmp(&s(&b["title"])))
    });
    let allow_comparison = ctx["allow_comparison"].as_bool().unwrap_or(true);
    let mut types = vec!["how_to", "explainer", "buyer_guide", "opinion"];
    if allow_comparison {
        types.push("alternatives");
    }
    let existing: Vec<String> = arr(&ctx["existing_titles"]).iter().map(s).collect();
    let dates = arr(&ctx["dates"]);
    let mut pieces = Vec::new();
    if topics.is_empty() {
        return json!({ "pieces": pieces });
    }
    let offset = existing.len();
    for (i, _date) in dates.iter().enumerate() {
        let topic = &topics[i % topics.len()];
        let content_type = types[(i + offset) % types.len()];
        let mut title = piece_title(content_type, &s(&topic["title"]));
        let mut n = 2;
        while existing.contains(&title) || pieces.iter().any(|p: &Value| s(&p["title"]) == title) {
            title = format!("{} (part {n})", piece_title(content_type, &s(&topic["title"])));
            n += 1;
        }
        pieces.push(json!({
            "topic_id": topic["id"],
            "content_type": content_type,
            "title": title,
            "angle": format!("Answer the question directly, then back it with our own experience of {}.", s(&topic["title"]).to_lowercase()),
            "target_keyword": s(&topic["title"]).to_lowercase(),
            "timeliness": "timeless"
        }));
    }
    json!({ "pieces": pieces })
}

fn write_brief(ctx: &Value) -> Value {
    let piece = &ctx["piece"];
    let entries = arr(&ctx["entries"]);
    let nums: Vec<i64> = entries.iter().filter_map(|e| e["num"].as_i64()).take(12).collect();
    let content_type = s(&piece["content_type"]);
    let mut outline = vec!["The short answer".to_string(), "Why it matters".to_string(), "How to do it".to_string(), "Our take".to_string()];
    if content_type == "alternatives" || content_type == "comparison" {
        outline.insert(1, "Options compared".into());
    }
    let title = s(&piece["title"]);
    json!({
        "title": title,
        "angle": s(&piece["angle"]),
        "outline": outline,
        "entry_nums": nums,
        "meta_description": truncate_words(&format!("{title}. A direct answer with practical detail from teams who do this every day."), 25),
    })
}

fn coverage_questions(ctx: &Value) -> Value {
    let title = s(&ctx["piece"]["title"]);
    let missing: Vec<String> = arr(&ctx["missing"]).iter().map(s).collect();
    let mut questions = Vec::new();
    if missing.iter().any(|m| m == "opinion") {
        questions.push(format!("What's your own position on \"{title}\"? What do most people get wrong about it?"));
    }
    if missing.iter().any(|m| m == "depth") {
        questions.push(format!("What facts, numbers or customer examples should a post called \"{title}\" include?"));
    }
    questions.push(format!("Which customer problem does \"{title}\" help with, in the customer's own words?"));
    json!({ "questions": questions })
}

fn sentence_case(text: &str) -> String {
    let t = text.trim().trim_end_matches(['.', '!', '?']);
    let mut c = t.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str() + ".",
        None => String::new(),
    }
}

fn draft(ctx: &Value) -> Value {
    let brief = &ctx["brief"];
    let content_type = s(&ctx["content_type"]);
    let entries = arr(&ctx["entries"]);
    let site = s(&ctx["site_name"]);
    let by_kind = |kinds: &[&str]| -> Vec<&Value> {
        entries.iter().filter(|e| kinds.contains(&s(&e["kind"]).as_str())).collect()
    };
    let cite = |e: &Value| format!("{} [[E{}]]", sentence_case(&s(&e["body"])), e["num"].as_i64().unwrap_or(0));

    let opinions = by_kind(&["opinion", "positioning"]);
    let audience = by_kind(&["audience", "story", "quote"]);
    let facts = by_kind(&["fact", "feature"]);
    let competitors = by_kind(&["competitor"]);

    let mut md = String::new();
    let lead: Option<&Value> = opinions.first().copied().or(facts.first().copied()).or(entries.first());
    if let Some(lead) = lead {
        md.push_str(&cite(lead));
        md.push_str("\n\n");
    }
    if content_type == "alternatives" || content_type == "comparison" {
        md.push_str("## Options compared\n\n| Option | Best for |\n| --- | --- |\n");
        md.push_str(&format!("| {site} | Teams that want the approach described here |\n"));
        for c in &competitors {
            let name = truncate_words(&s(&c["body"]), 4);
            md.push_str(&format!("| {name} | See the notes below |\n"));
        }
        md.push('\n');
        for c in &competitors {
            md.push_str(&cite(c));
            md.push_str("\n\n");
        }
    }
    if !audience.is_empty() {
        md.push_str("## Why it matters\n\n");
        for e in audience.iter().take(3) {
            md.push_str(&cite(e));
            md.push_str("\n\n");
        }
    }
    if !facts.is_empty() {
        md.push_str("## How to do it\n\n");
        for e in facts.iter().take(5) {
            if Some(*e) != lead {
                md.push_str(&cite(e));
                md.push_str("\n\n");
            }
        }
    }
    let rest: Vec<&&Value> = opinions.iter().skip(1).collect();
    if !rest.is_empty() {
        md.push_str("## Our take\n\n");
        for e in rest.iter().take(3) {
            md.push_str(&cite(e));
            md.push_str("\n\n");
        }
    }
    json!({
        "title": s(&brief["title"]),
        "meta_description": s(&brief["meta_description"]),
        "body_md": md.trim_end().to_string() + "\n",
    })
}

fn verify_claim(ctx: &Value) -> Value {
    let claim = s(&ctx["claim"]);
    let quote = s(&ctx["quote"]);
    let score = overlap(&claim, &quote);
    json!({ "supported": score >= 0.6, "reason": format!("{:.0}% of the claim's key words appear in the quoted passage", score * 100.0) })
}

fn review_draft(ctx: &Value) -> Value {
    let body = s(&ctx["body_md"]);
    let mut issues = Vec::new();
    if body.len() < 200 {
        issues.push("The draft is too short to be useful.".to_string());
    }
    if !body.contains("[[E") {
        issues.push("The draft cites no library entries.".to_string());
    }
    json!({ "pass": issues.is_empty(), "issues": issues })
}

/// First run of capitalised words that isn't the start of the sentence, such as a product name.
fn proper_noun(text: &str, exclude: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut i = 0;
    while i < words.len() {
        let w = words[i].trim_matches(|c: char| !c.is_alphanumeric());
        let capital = w.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
        if capital && (i > 0 || words.len() == 1) && !w.eq_ignore_ascii_case(exclude) && !["I", "We", "Our"].contains(&w) {
            let mut name = vec![w.to_string()];
            let mut j = i + 1;
            while j < words.len() {
                let n = words[j].trim_matches(|c: char| !c.is_alphanumeric());
                if n.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    name.push(n.to_string());
                    j += 1;
                } else {
                    break;
                }
            }
            return Some(name.join(" "));
        }
        i += 1;
    }
    None
}

fn short_name(text: &str) -> String {
    let skip = ["we", "our", "it", "you", "the", "a", "an", "with", "your"];
    let words: Vec<&str> = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .skip_while(|w| skip.contains(&w.to_lowercase().as_str()))
        .take(4)
        .collect();
    words.join(" ").to_lowercase()
}

fn extract_graph(ctx: &Value) -> Value {
    let site = s(&ctx["site_name"]);
    let product = json!({ "kind": "product", "name": site });
    let mut entities = vec![product.clone()];
    let mut edges = Vec::new();
    let mut about = Vec::new();
    let topics: Vec<String> = arr(&ctx["topics"]).iter().map(|t| s(&t["title"])).collect();
    for e in arr(&ctx["entries"]) {
        let num = e["num"].as_i64().unwrap_or(0);
        let body = s(&e["body"]);
        let entity = match s(&e["kind"]).as_str() {
            "competitor" => proper_noun(&body, &site).map(|n| (json!({ "kind": "competitor", "name": n }), "competes_with")),
            "feature" => Some((json!({ "kind": "feature", "name": short_name(&body) }), "has_feature")),
            "audience" => Some((json!({ "kind": "persona", "name": short_name(&body) }), "serves")),
            _ => None,
        };
        match entity {
            Some((ent, edge_kind)) => {
                if !entities.contains(&ent) {
                    entities.push(ent.clone());
                }
                edges.push(json!({ "from": product, "to": ent, "kind": edge_kind }));
                about.push(json!({ "entry_num": num, "entity": ent }));
                for t in &topics {
                    if overlap(t, &body) > 0.0 {
                        edges.push(json!({ "from": ent, "to": { "kind": "topic", "name": t }, "kind": "part_of_topic" }));
                    }
                }
            }
            None => about.push(json!({ "entry_num": num, "entity": product })),
        }
    }
    json!({ "entities": entities, "edges": edges, "about": about })
}

fn numbers(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '$' || c == '%'))
        .filter(|t| t.chars().any(|c| c.is_ascii_digit()))
        .map(|t| t.trim_matches('.').to_string())
        .collect()
}

fn detect_conflicts(ctx: &Value) -> Value {
    let entry = &ctx["entry"];
    let body = s(&entry["body"]);
    let kind = s(&entry["kind"]);
    let mut links = Vec::new();
    for c in arr(&ctx["candidates"]) {
        if s(&c["kind"]) != kind {
            continue;
        }
        let other = s(&c["body"]);
        let shared = stems(&body).intersection(&stems(&other)).count();
        if shared < 2 {
            continue;
        }
        let (na, nb) = (numbers(&body), numbers(&other));
        if !na.is_empty() && !nb.is_empty() && na != nb {
            links.push(json!({ "to_num": c["num"], "kind": "contradicts", "note": "The two entries give different figures." }));
        } else if overlap(&other, &body) >= 0.8 && body.len() > other.len() {
            links.push(json!({ "to_num": c["num"], "kind": "refines", "note": "Adds detail to an existing entry." }));
        }
    }
    json!({ "links": links })
}

fn learn_from_edit(ctx: &Value) -> Value {
    let before = crate::text::strip_citations(&s(&ctx["before"]));
    let after = crate::text::strip_citations(&s(&ctx["after"]));
    let mut entries = Vec::new();
    for para in after.split("\n\n") {
        let p = para.trim();
        if p.starts_with('#') || p.starts_with('|') || p.split_whitespace().count() < 8 {
            continue;
        }
        if before.contains(p) {
            continue;
        }
        let lower = p.to_lowercase();
        let kind = if OPINION_MARKERS.iter().any(|m| lower.contains(m)) { "opinion" } else { "fact" };
        entries.push(json!({ "kind": kind, "body": truncate_words(p, 80), "quote": p }));
    }
    json!({ "entries": entries })
}

fn tag_topics(ctx: &Value) -> Value {
    let topics = arr(&ctx["topics"]);
    let mut tags = Vec::new();
    for e in arr(&ctx["entries"]) {
        let body = s(&e["body"]);
        for t in &topics {
            let label = format!("{} {}", s(&t["title"]), s(&t["description"]));
            let shared = stems(&s(&t["title"])).intersection(&stems(&body)).count();
            if shared > 0 || overlap(&label, &body) >= 0.3 {
                tags.push(json!({ "entry_num": e["num"], "topic_id": t["id"] }));
            }
        }
    }
    json!({ "tags": tags })
}
