//! Turning websites, documents and transcripts into passages that entries can quote.

use crate::library::create_source;
use crate::models::Site;
use crate::state::AppState;
use anyhow::{anyhow, Context, Result};
use scraper::{ElementRef, Html, Selector};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::Read;
use url::Url;

pub const MAX_CRAWL_PAGES: usize = 200;

pub fn hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Groups paragraphs into passages of roughly 40 to 150 words.
pub fn passages_from_paragraphs(paragraphs: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for p in paragraphs {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        if !current.is_empty() && crate::text::word_count(&current) + crate::text::word_count(p) > 150 {
            out.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(p);
        if crate::text::word_count(&current) >= 40 {
            out.push(std::mem::take(&mut current));
        }
    }
    if crate::text::word_count(&current) >= 5 {
        out.push(current);
    }
    out
}

pub fn split_paragraphs(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n")
        .split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect()
}

// ---------------------------------------------------------------- documents

pub fn docx_text(bytes: &[u8]) -> Result<String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("not a DOCX file")?;
    let mut xml = String::new();
    archive.by_name("word/document.xml").context("DOCX has no document body")?.read_to_string(&mut xml)?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut out = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event()? {
            quick_xml::events::Event::Start(e) if e.name().as_ref() == "w:t" => in_text = true,
            quick_xml::events::Event::End(e) if e.name().as_ref() == "w:t" => in_text = false,
            quick_xml::events::Event::End(e) if e.name().as_ref() == "w:p" => out.push_str("\n\n"),
            quick_xml::events::Event::Text(t) if in_text => {
                out.push_str(t.as_ref());
            }
            quick_xml::events::Event::GeneralRef(r) if in_text => {
                let name: String = r.as_ref().to_string();
                out.push_str(match name.as_str() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                });
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// Strips cue numbers, timestamps and headers from VTT and SRT files.
pub fn transcript_text(raw: &str) -> String {
    let mut lines = Vec::new();
    for line in raw.lines() {
        let l = line.trim();
        if l.is_empty() {
            lines.push(String::new());
            continue;
        }
        if l == "WEBVTT" || l.contains("-->") || l.chars().all(|c| c.is_ascii_digit()) || l.starts_with("NOTE") {
            continue;
        }
        let l = regex::Regex::new(r"<[^>]+>").unwrap().replace_all(l, "").to_string();
        lines.push(l);
    }
    let joined = lines.join("\n");
    // Merge cue fragments into paragraphs per speaker turn.
    let mut paragraphs: Vec<String> = Vec::new();
    for block in joined.split("\n\n") {
        let text = block.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        let new_speaker = text.split_once(':').map(|(s, _)| s.split_whitespace().count() <= 3).unwrap_or(false);
        match paragraphs.last_mut() {
            Some(last) if !new_speaker && crate::text::word_count(last) < 120 => {
                last.push(' ');
                last.push_str(&text);
            }
            _ => paragraphs.push(text),
        }
    }
    paragraphs.join("\n\n")
}

pub fn document_text(filename: &str, bytes: &[u8]) -> Result<(String, bool)> {
    let lower = filename.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    let is_transcript = matches!(ext, "vtt" | "srt") || lower.contains("transcript");
    let text = match ext {
        "pdf" => pdf_extract::extract_text_from_mem(bytes).map_err(|e| anyhow!("could not read PDF: {e}"))?,
        "docx" => docx_text(bytes)?,
        "md" | "markdown" | "txt" | "text" => String::from_utf8_lossy(bytes).to_string(),
        "vtt" | "srt" => transcript_text(&String::from_utf8_lossy(bytes)),
        other => return Err(anyhow!("unsupported file type .{other}; use PDF, DOCX, Markdown, text, VTT or SRT")),
    };
    let text = if is_transcript && !matches!(ext, "vtt" | "srt") { transcript_text(&text) } else { text };
    Ok((text, is_transcript))
}

pub async fn process_upload(state: &AppState, site: &Site, upload_id: uuid::Uuid) -> Result<serde_json::Value> {
    let (filename, key): (String, String) = sqlx::query_as("select filename, storage_key from uploads where id = $1")
        .bind(upload_id)
        .fetch_one(&state.db)
        .await?;
    let bytes = state.storage.get(&key).await?;
    let parsed = document_text(&filename, &bytes);
    let (text, is_transcript) = match parsed {
        Ok(v) => v,
        Err(e) => {
            sqlx::query("update uploads set status = 'failed', error = $2 where id = $1").bind(upload_id).bind(e.to_string()).execute(&state.db).await?;
            return Ok(json!({ "error": e.to_string() }));
        }
    };
    let passages = passages_from_paragraphs(&split_paragraphs(&text));
    let kind = if is_transcript { "transcript" } else { "upload" };
    let (source_id, _) = create_source(state, site, kind, &filename, &filename, &passages).await?;
    sqlx::query("update uploads set status = 'processed' where id = $1").bind(upload_id).execute(&state.db).await?;
    crate::jobs::enqueue(state, Some(site.org_id), Some(site.id), "extract_source", json!({ "source_id": source_id })).await?;
    Ok(json!({ "source_id": source_id, "passages": passages.len() }))
}

// ---------------------------------------------------------------- crawling

#[derive(Debug, Default)]
pub struct Robots {
    disallow: Vec<String>,
    allow: Vec<String>,
    pub sitemaps: Vec<String>,
}

impl Robots {
    pub fn parse(text: &str) -> Robots {
        let mut robots = Robots::default();
        let mut applies = false;
        let mut seen_rules_for_group = false;
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((key, value)) = line.split_once(':') else { continue };
            let key = key.trim().to_lowercase();
            let value = value.trim();
            match key.as_str() {
                "user-agent" => {
                    if seen_rules_for_group {
                        applies = false;
                        seen_rules_for_group = false;
                    }
                    let agent = value.to_lowercase();
                    if agent == "*" || agent.contains("kwilbot") {
                        applies = true;
                    }
                }
                "disallow" if applies => {
                    seen_rules_for_group = true;
                    if !value.is_empty() {
                        robots.disallow.push(value.to_string());
                    }
                }
                "allow" if applies => {
                    seen_rules_for_group = true;
                    robots.allow.push(value.to_string());
                }
                "disallow" | "allow" => seen_rules_for_group = true,
                "sitemap" => robots.sitemaps.push(value.to_string()),
                _ => {}
            }
        }
        robots
    }

    /// Longest matching rule wins; allow wins ties.
    pub fn allowed(&self, path: &str) -> bool {
        let best = |rules: &[String]| rules.iter().filter(|r| path.starts_with(r.as_str())).map(|r| r.len()).max();
        match (best(&self.allow), best(&self.disallow)) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some(a), Some(d)) => a >= d,
        }
    }
}

fn locs(xml: &str) -> Vec<String> {
    let re = regex::Regex::new(r"(?s)<loc>\s*(.*?)\s*</loc>").unwrap();
    re.captures_iter(xml).map(|c| c[1].replace("&amp;", "&")).collect()
}

/// The main readable text of a page, as paragraphs, plus its title.
pub fn page_text(html: &str) -> (String, Vec<String>) {
    let doc = Html::parse_document(html);
    let title = doc
        .select(&Selector::parse("title").unwrap())
        .next()
        .map(|t| t.text().collect::<String>().trim().to_string())
        .unwrap_or_default();
    let root_sel = ["main", "article", "[role=main]", "body"];
    let root: Option<ElementRef> = root_sel.iter().find_map(|s| doc.select(&Selector::parse(s).unwrap()).next());
    let Some(root) = root else { return (title, Vec::new()) };
    let blocks = Selector::parse("h1, h2, h3, p, li, blockquote, td").unwrap();
    let skip = ["nav", "header", "footer", "script", "style", "noscript", "form", "aside"];
    let mut paragraphs = Vec::new();
    for el in root.select(&blocks) {
        let inside_skipped = el.ancestors().filter_map(ElementRef::wrap).any(|a| skip.contains(&a.value().name()));
        if inside_skipped {
            continue;
        }
        let text = el.text().collect::<Vec<_>>().join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
        let min_words = if matches!(el.value().name(), "h1" | "h2" | "h3") { 3 } else { 6 };
        if crate::text::word_count(&text) >= min_words && !paragraphs.contains(&text) {
            paragraphs.push(text);
        }
    }
    (title, paragraphs)
}

pub async fn crawl_site(state: &AppState, site: &Site) -> Result<serde_json::Value> {
    let base = site.website_url.clone().ok_or_else(|| anyhow!("the site has no website URL to crawl"))?;
    let base = Url::parse(&base).context("the website URL is not valid")?;
    let origin = format!("{}://{}", base.scheme(), base.host_str().unwrap_or_default());
    let robots_page = state.fetcher.get(&format!("{origin}/robots.txt")).await?;
    let robots = if robots_page.status == 200 { Robots::parse(&robots_page.body) } else { Robots::default() };

    let mut sitemap_urls = robots.sitemaps.clone();
    if sitemap_urls.is_empty() {
        sitemap_urls.push(format!("{origin}/sitemap.xml"));
    }
    let mut pages: Vec<String> = Vec::new();
    let mut nested = 0;
    while let Some(sm) = sitemap_urls.pop() {
        let resp = state.fetcher.get(&sm).await?;
        if resp.status != 200 {
            continue;
        }
        let found = locs(&resp.body);
        if resp.body.contains("<sitemapindex") {
            nested += 1;
            if nested <= 20 {
                sitemap_urls.extend(found);
            }
            continue;
        }
        pages.extend(found);
    }
    if pages.is_empty() {
        pages.push(base.to_string());
    }
    let host = base.host_str().unwrap_or_default().to_string();
    let mut seen = Vec::new();
    for p in pages {
        if let Ok(u) = Url::parse(&p) {
            if u.host_str() == Some(host.as_str()) && robots.allowed(u.path()) && !seen.contains(&p) {
                seen.push(p);
            }
        }
        if seen.len() >= MAX_CRAWL_PAGES {
            break;
        }
    }

    let mut changed = 0;
    let mut skipped = 0;
    for url in &seen {
        let page = state.fetcher.get(url).await?;
        if page.status != 200 {
            skipped += 1;
            continue;
        }
        let (title, paragraphs) = page_text(&page.body);
        let text = paragraphs.join("\n\n");
        let h = hash(&text);
        let previous: Option<String> = sqlx::query_scalar("select content_hash from crawl_pages where site_id = $1 and url = $2")
            .bind(site.id)
            .bind(url)
            .fetch_optional(&state.db)
            .await?;
        if previous.as_deref() == Some(h.as_str()) {
            skipped += 1;
            continue;
        }
        sqlx::query(
            "insert into crawl_pages (site_id, url, content_hash, fetched_at) values ($1, $2, $3, $4)
             on conflict (site_id, url) do update set content_hash = excluded.content_hash, fetched_at = excluded.fetched_at",
        )
        .bind(site.id)
        .bind(url)
        .bind(&h)
        .bind(state.clock.now())
        .execute(&state.db)
        .await?;
        let passages = passages_from_paragraphs(&paragraphs);
        if passages.is_empty() {
            continue;
        }
        let (source_id, _) = create_source(state, site, "crawl", url, &title, &passages).await?;
        crate::jobs::enqueue(state, Some(site.org_id), Some(site.id), "extract_source", json!({ "source_id": source_id })).await?;
        changed += 1;
    }
    sqlx::query("update sites set last_crawled_at = $2 where id = $1").bind(site.id).bind(state.clock.now()).execute(&state.db).await?;
    Ok(json!({ "pages": seen.len(), "changed": changed, "unchanged_or_skipped": skipped }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_rules() {
        let r = Robots::parse("User-agent: *\nDisallow: /admin\nAllow: /admin/public\nSitemap: https://x.test/sm.xml\n\nUser-agent: Other\nDisallow: /\n");
        assert!(r.allowed("/pricing"));
        assert!(!r.allowed("/admin/secret"));
        assert!(r.allowed("/admin/public/page"));
        assert_eq!(r.sitemaps, vec!["https://x.test/sm.xml"]);
    }

    #[test]
    fn extracts_main_text_without_navigation() {
        let html = "<html><head><title>Pricing</title></head><body><nav><p>Home About Pricing Blog Contact Login Signup</p></nav>\
<main><h1>Simple pricing for small teams</h1><p>The Pro plan costs $49 per month and includes unlimited projects.</p></main>\
<footer><p>Copyright 2026 Example Inc all rights reserved here</p></footer></body></html>";
        let (title, paras) = page_text(html);
        assert_eq!(title, "Pricing");
        assert_eq!(paras, vec!["Simple pricing for small teams".to_string(), "The Pro plan costs $49 per month and includes unlimited projects.".to_string()]);
    }

    #[test]
    fn transcripts_lose_timestamps() {
        let vtt = "WEBVTT\n\n1\n00:00:01.000 --> 00:00:04.000\nCustomer: We tried three tools before this one.\n\n2\n00:00:04.000 --> 00:00:06.000\nand none of them handled exports.\n";
        let text = transcript_text(vtt);
        assert_eq!(text, "Customer: We tried three tools before this one. and none of them handled exports.");
    }
}
