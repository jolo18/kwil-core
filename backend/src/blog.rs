//! The public blog service. Read-only: it resolves the site from the request's host name and
//! renders published content only.

use crate::models::{FixedPage, Post, Site, Topic};
use crate::state::AppState;
use crate::text::{slugify, strip_citations};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use chrono::{DateTime, Utc};
use maud::{html, Markup, PreEscaped, DOCTYPE};
use pulldown_cmark::{Event, Options, Parser};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/robots.txt", get(robots))
        .route("/sitemap.xml", get(sitemap))
        .route("/rss.xml", get(rss))
        .route("/feed", get(rss))
        .route("/llms.txt", get(llms))
        .route("/topic/{slug}", get(topic_page))
        .route("/author/{slug}", get(author_page))
        .route("/{slug}", get(post_or_page))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
}

struct Ctx {
    site: Site,
    base_url: String,
    pages: Vec<FixedPage>,
}

fn host_of(headers: &HeaderMap) -> String {
    let raw = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    raw.split(',').next().unwrap_or("").trim().split(':').next().unwrap_or("").to_lowercase()
}

fn not_found() -> Response {
    let page = html! {
        (DOCTYPE)
        html lang="en" { head { meta charset="utf-8"; title { "Not found" } } body style="font-family: system-ui; padding: 4rem; text-align: center" { h1 { "Page not found" } } }
    };
    (StatusCode::NOT_FOUND, page).into_response()
}

async fn resolve(state: &AppState, headers: &HeaderMap) -> Option<Ctx> {
    let host = host_of(headers);
    let base = &state.config.blog_base_domain;
    let site = if let Some(slug) = host.strip_suffix(&format!(".{base}")) {
        sqlx::query_as::<_, Site>("select * from sites where slug = $1").bind(slug).fetch_optional(&state.db).await.ok()?
    } else {
        sqlx::query_as::<_, Site>("select s.* from sites s join domains d on d.site_id = s.id where d.hostname = $1 and d.status = 'active'")
            .bind(&host)
            .fetch_optional(&state.db)
            .await
            .ok()?
    }?;
    // Canonical host: an active custom domain if there is one, else the subdomain.
    let custom: Option<String> = sqlx::query_scalar("select hostname from domains where site_id = $1 and status = 'active' order by created_at limit 1")
        .bind(site.id)
        .fetch_optional(&state.db)
        .await
        .ok()?;
    let base_url = match custom {
        Some(h) => format!("https://{h}"),
        None if base == "localhost" => format!("http://{}.localhost:{}", site.slug, state.config.blog_port),
        None => format!("https://{}.{}", site.slug, base),
    };
    let pages = sqlx::query_as::<_, FixedPage>("select * from fixed_pages where site_id = $1 order by nav_position, title")
        .bind(site.id)
        .fetch_all(&state.db)
        .await
        .ok()?;
    Some(Ctx { site, base_url, pages })
}

const PUBLISHED: &str = "status = 'published' and published_at <= $2";

async fn published_posts(state: &AppState, site: &Site, limit: i64, offset: i64) -> Vec<Post> {
    sqlx::query_as::<_, Post>(sqlx::AssertSqlSafe(format!("select * from posts where site_id = $1 and {PUBLISHED} order by published_at desc limit $3 offset $4")))
        .bind(site.id)
        .bind(state.clock.now())
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default()
}

async fn published_post(state: &AppState, site: &Site, slug: &str) -> Option<Post> {
    sqlx::query_as::<_, Post>(sqlx::AssertSqlSafe(format!("select * from posts where site_id = $1 and {PUBLISHED} and slug = $3")))
        .bind(site.id)
        .bind(state.clock.now())
        .bind(slug)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
}

/// Markdown to HTML with citations removed and raw HTML shown as text.
pub fn render_markdown(md: &str) -> String {
    let clean = strip_citations(md);
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(&clean, opts).map(|e| match e {
        Event::Html(h) | Event::InlineHtml(h) => Event::Text(h),
        other => other,
    });
    let mut out = String::new();
    pulldown_cmark::html::push_html(&mut out, parser);
    out
}

/// Question headings followed by an answer paragraph, for FAQ structured data.
pub fn faq_pairs(md: &str) -> Vec<(String, String)> {
    let clean = strip_citations(md);
    let blocks: Vec<&str> = clean.split("\n\n").map(|b| b.trim()).filter(|b| !b.is_empty()).collect();
    let mut out = Vec::new();
    for w in blocks.windows(2) {
        if let Some(q) = w[0].strip_prefix("## ").or_else(|| w[0].strip_prefix("### ")) {
            if q.trim().ends_with('?') && !w[1].starts_with('#') && !w[1].starts_with('|') {
                out.push((q.trim().to_string(), w[1].to_string()));
            }
        }
    }
    out
}

fn brand(site: &Site, key: &str) -> String {
    site.branding.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn safe_color(c: &str) -> String {
    let ok = c.starts_with('#') && (c.len() == 4 || c.len() == 7) && c[1..].chars().all(|ch| ch.is_ascii_hexdigit());
    if ok { c.to_string() } else { "#2563eb".into() }
}

fn font_stack(font: &str) -> &'static str {
    match font {
        "serif" => "Georgia, 'Times New Roman', serif",
        "mono" => "ui-monospace, SFMono-Regular, Menlo, monospace",
        "humanist" => "'Segoe UI', Candara, 'Gill Sans', Optima, sans-serif",
        _ => "system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif",
    }
}

fn css(site: &Site) -> String {
    let primary = safe_color(&brand(site, "primary_color"));
    let font = font_stack(&brand(site, "font"));
    format!(
        ":root{{--p:{primary};--fg:#111827;--muted:#6b7280;--bg:#ffffff;--line:#e5e7eb;--soft:#f9fafb}}
@media (prefers-color-scheme:dark){{:root{{--fg:#f3f4f6;--muted:#9ca3af;--bg:#0b0f17;--line:#1f2937;--soft:#111827}}}}
*{{box-sizing:border-box}}body{{margin:0;font-family:{font};color:var(--fg);background:var(--bg);line-height:1.65;font-size:18px}}
a{{color:var(--p)}}.wrap{{max-width:760px;margin:0 auto;padding:0 20px}}
header.site{{border-bottom:1px solid var(--line)}}header.site .wrap{{display:flex;align-items:center;justify-content:space-between;gap:16px;padding:18px 20px;flex-wrap:wrap}}
.logo{{font-weight:700;font-size:20px;color:var(--fg);text-decoration:none;display:flex;align-items:center;gap:10px}}.logo img{{height:32px}}
nav a{{margin-left:18px;color:var(--muted);text-decoration:none;font-size:16px}}nav a:hover{{color:var(--p)}}
main.wrap{{padding:40px 20px 64px}}h1{{font-size:40px;line-height:1.15;margin:0 0 12px}}h2{{font-size:26px;margin:40px 0 12px}}h3{{font-size:21px}}
.meta{{color:var(--muted);font-size:15px;margin-bottom:28px}}.lede{{font-size:20px;color:var(--muted)}}
article table{{border-collapse:collapse;width:100%;margin:20px 0;font-size:16px;display:block;overflow-x:auto}}
article th,article td{{border:1px solid var(--line);padding:10px 12px;text-align:left}}article th{{background:var(--soft)}}
article img{{max-width:100%}}blockquote{{border-left:4px solid var(--p);margin:0;padding:4px 20px;color:var(--muted)}}
.card{{display:block;padding:22px 0;border-bottom:1px solid var(--line);text-decoration:none;color:var(--fg)}}.card h2{{margin:0 0 6px;font-size:23px}}.card p{{margin:0;color:var(--muted)}}
.tag{{display:inline-block;font-size:13px;padding:2px 10px;border-radius:999px;background:var(--soft);border:1px solid var(--line);color:var(--muted);text-decoration:none;margin-right:6px}}
footer.site{{border-top:1px solid var(--line);color:var(--muted);font-size:15px;padding:28px 0}}.pager{{display:flex;justify-content:space-between;margin-top:28px}}
@media (max-width:600px){{body{{font-size:17px}}h1{{font-size:31px}}nav a{{margin:0 14px 0 0}}}}"
    )
}

struct Head {
    title: String,
    description: String,
    canonical: String,
    og_type: &'static str,
    json_ld: Vec<Value>,
    alternate_md: Option<String>,
}

fn layout(c: &Ctx, head: Head, body: Markup) -> Markup {
    let logo = brand(&c.site, "logo_url");
    let nav: Vec<(String, String)> = c
        .site
        .branding
        .get("nav")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|n| Some((n.get("label")?.as_str()?.to_string(), n.get("url")?.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    let footer = brand(&c.site, "footer");
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (head.title) }
                meta name="description" content=(head.description);
                link rel="canonical" href=(head.canonical);
                link rel="alternate" type="application/rss+xml" title=(c.site.name) href=(format!("{}/rss.xml", c.base_url));
                @if let Some(md) = &head.alternate_md { link rel="alternate" type="text/markdown" href=(md); }
                meta property="og:title" content=(head.title);
                meta property="og:description" content=(head.description);
                meta property="og:type" content=(head.og_type);
                meta property="og:url" content=(head.canonical);
                meta property="og:site_name" content=(c.site.name);
                meta name="twitter:card" content="summary";
                meta name="twitter:title" content=(head.title);
                meta name="twitter:description" content=(head.description);
                @for ld in &head.json_ld {
                    script type="application/ld+json" { (PreEscaped(ld.to_string().replace("</", "<\\/"))) }
                }
                style { (PreEscaped(css(&c.site))) }
            }
            body {
                header.site { div.wrap {
                    a.logo href="/" {
                        @if !logo.is_empty() { img src=(logo) alt=(c.site.name); }
                        span { (c.site.name) }
                    }
                    nav {
                        @for p in &c.pages { a href=(format!("/{}", p.slug)) { (p.title) } }
                        @for (label, url) in &nav { a href=(url) { (label) } }
                    }
                } }
                main.wrap { (body) }
                footer.site { div.wrap {
                    (if footer.is_empty() { format!("© {}", c.site.name) } else { footer })
                    " · " a href="/rss.xml" { "RSS" }
                } }
            }
        }
    }
}

fn cached(markup: Markup) -> Response {
    ([(header::CACHE_CONTROL, "public, max-age=60, s-maxage=86400")], markup).into_response()
}

fn text(content_type: &'static str, body: String) -> Response {
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "public, max-age=300, s-maxage=86400")], body).into_response()
}

fn date(d: Option<DateTime<Utc>>) -> String {
    d.map(|d| d.format("%B %-d, %Y").to_string()).unwrap_or_default()
}

fn post_card(p: &Post) -> Markup {
    html! {
        a.card href=(format!("/{}", p.slug)) {
            h2 { (p.title) }
            p { (p.meta_description) }
            p.meta style="margin:6px 0 0" { (date(p.published_at)) }
        }
    }
}

#[derive(Deserialize)]
struct PageQ {
    page: Option<i64>,
}

const PER_PAGE: i64 = 20;

async fn home(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<PageQ>) -> Response {
    let Some(c) = resolve(&state, &headers).await else { return not_found() };
    let page = q.page.unwrap_or(1).max(1);
    let posts = published_posts(&state, &c.site, PER_PAGE + 1, (page - 1) * PER_PAGE).await;
    let more = posts.len() as i64 > PER_PAGE;
    let topics = sqlx::query_as::<_, Topic>(
        "select t.* from topics t where t.site_id = $1 and t.status = 'approved' and t.parent_id is null
         and exists(select 1 from posts p where p.topic_id in (select id from topics where id = t.id or parent_id = t.id) and p.status = 'published')
         order by t.position",
    )
    .bind(c.site.id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let description = if c.site.product_description.is_empty() { format!("Articles from {}", c.site.name) } else { crate::text::truncate_words(&c.site.product_description, 30) };
    let ld = json!({ "@context": "https://schema.org", "@type": "Blog", "name": c.site.name, "url": c.base_url, "description": description });
    let body = html! {
        h1 { (c.site.name) }
        p.lede { (description) }
        @if !topics.is_empty() {
            p { @for t in &topics { a.tag href=(format!("/topic/{}", t.slug)) { (t.title) } } }
        }
        @if posts.is_empty() { p { "No posts yet." } }
        @for p in posts.iter().take(PER_PAGE as usize) { (post_card(p)) }
        div.pager {
            @if page > 1 { a href=(format!("/?page={}", page - 1)) { "← Newer" } } @else { span {} }
            @if more { a href=(format!("/?page={}", page + 1)) { "Older →" } }
        }
    };
    let head = Head {
        title: c.site.name.clone(),
        description: description.clone(),
        canonical: if page > 1 { format!("{}/?page={page}", c.base_url) } else { format!("{}/", c.base_url) },
        og_type: "website",
        json_ld: vec![ld],
        alternate_md: None,
    };
    cached(layout(&c, head, body))
}

async fn post_or_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let Some(c) = resolve(&state, &headers).await else { return not_found() };
    if let Some(md_slug) = slug.strip_suffix(".md") {
        let Some(post) = published_post(&state, &c.site, md_slug).await else { return not_found() };
        let body = format!(
            "# {}\n\n> {}\n\nPublished {} · Updated {} · {}/{}\n\n{}",
            post.title,
            post.meta_description,
            post.published_at.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default(),
            post.last_updated_at.format("%Y-%m-%d"),
            c.base_url,
            post.slug,
            strip_citations(&post.body_md)
        );
        return text("text/markdown; charset=utf-8", body);
    }
    if let Some(post) = published_post(&state, &c.site, &slug).await {
        return render_post(&state, &c, &post).await;
    }
    if let Some(page) = c.pages.iter().find(|p| p.slug == slug) {
        let head = Head {
            title: format!("{} · {}", page.title, c.site.name),
            description: crate::text::truncate_words(&strip_citations(&page.body_md).replace('\n', " "), 30),
            canonical: format!("{}/{}", c.base_url, page.slug),
            og_type: "website",
            json_ld: vec![],
            alternate_md: None,
        };
        let body = html! { article { h1 { (page.title) } (PreEscaped(render_markdown(&page.body_md))) } };
        return cached(layout(&c, head, body));
    }
    not_found()
}

async fn render_post(state: &AppState, c: &Ctx, post: &Post) -> Response {
    let topic: Option<Topic> = match post.topic_id {
        Some(t) => sqlx::query_as::<_, Topic>("select * from topics where id = $1").bind(t).fetch_optional(&state.db).await.ok().flatten(),
        None => None,
    };
    let author = if post.author_name.is_empty() { c.site.author_name.clone() } else { post.author_name.clone() };
    let url = format!("{}/{}", c.base_url, post.slug);
    let mut ld = vec![json!({
        "@context": "https://schema.org",
        "@type": "Article",
        "headline": post.title,
        "description": post.meta_description,
        "datePublished": post.published_at.map(|d| d.to_rfc3339()),
        "dateModified": post.last_updated_at.to_rfc3339(),
        "author": { "@type": "Organization", "name": author },
        "publisher": { "@type": "Organization", "name": c.site.name },
        "mainEntityOfPage": url,
        "url": url,
    })];
    let faqs = faq_pairs(&post.body_md);
    if !faqs.is_empty() {
        ld.push(json!({
            "@context": "https://schema.org",
            "@type": "FAQPage",
            "mainEntity": faqs.iter().map(|(q, a)| json!({ "@type": "Question", "name": q, "acceptedAnswer": { "@type": "Answer", "text": a } })).collect::<Vec<_>>(),
        }));
    }
    let mut crumbs = vec![json!({ "@type": "ListItem", "position": 1, "name": c.site.name, "item": format!("{}/", c.base_url) })];
    if let Some(t) = &topic {
        crumbs.push(json!({ "@type": "ListItem", "position": 2, "name": t.title, "item": format!("{}/topic/{}", c.base_url, t.slug) }));
    }
    crumbs.push(json!({ "@type": "ListItem", "position": crumbs.len() + 1, "name": post.title, "item": url }));
    ld.push(json!({ "@context": "https://schema.org", "@type": "BreadcrumbList", "itemListElement": crumbs }));

    let updated = post.published_at.map(|p| post.last_updated_at.date_naive() > p.date_naive()).unwrap_or(false);
    let body = html! {
        article {
            h1 { (post.title) }
            p.meta {
                a href=(format!("/author/{}", slugify(&author))) { (author) }
                " · " (date(post.published_at))
                @if updated { " · Updated " (date(Some(post.last_updated_at))) }
                @if let Some(t) = &topic { " · " a href=(format!("/topic/{}", t.slug)) { (t.title) } }
            }
            (PreEscaped(render_markdown(&post.body_md)))
        }
    };
    let head = Head {
        title: format!("{} · {}", post.title, c.site.name),
        description: post.meta_description.clone(),
        canonical: url.clone(),
        og_type: "article",
        json_ld: ld,
        alternate_md: Some(format!("{url}.md")),
    };
    cached(layout(c, head, body))
}

async fn topic_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let Some(c) = resolve(&state, &headers).await else { return not_found() };
    let Some(topic) = sqlx::query_as::<_, Topic>("select * from topics where site_id = $1 and slug = $2 and status = 'approved'")
        .bind(c.site.id)
        .bind(&slug)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
    else {
        return not_found();
    };
    let posts = sqlx::query_as::<_, Post>(sqlx::AssertSqlSafe(format!(
        "select * from posts where site_id = $1 and {PUBLISHED} and topic_id in (select id from topics where id = $3 or parent_id = $3) order by published_at desc"
    )))
    .bind(c.site.id)
    .bind(state.clock.now())
    .bind(topic.id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let children = sqlx::query_as::<_, Topic>("select * from topics where parent_id = $1 and status = 'approved' order by position").bind(topic.id).fetch_all(&state.db).await.unwrap_or_default();
    let head = Head {
        title: format!("{} · {}", topic.title, c.site.name),
        description: if topic.description.is_empty() { format!("Articles about {}", topic.title) } else { topic.description.clone() },
        canonical: format!("{}/topic/{}", c.base_url, topic.slug),
        og_type: "website",
        json_ld: vec![json!({ "@context": "https://schema.org", "@type": "CollectionPage", "name": topic.title, "url": format!("{}/topic/{}", c.base_url, topic.slug) })],
        alternate_md: None,
    };
    let body = html! {
        h1 { (topic.title) }
        @if !topic.description.is_empty() { p.lede { (topic.description) } }
        @if !children.is_empty() { p { @for t in &children { a.tag href=(format!("/topic/{}", t.slug)) { (t.title) } } } }
        @if posts.is_empty() { p { "No posts on this topic yet." } }
        @for p in &posts { (post_card(p)) }
    };
    cached(layout(&c, head, body))
}

async fn author_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let Some(c) = resolve(&state, &headers).await else { return not_found() };
    let posts: Vec<Post> = published_posts(&state, &c.site, 500, 0)
        .await
        .into_iter()
        .filter(|p| slugify(if p.author_name.is_empty() { &c.site.author_name } else { &p.author_name }) == slug)
        .collect();
    let Some(first) = posts.first() else { return not_found() };
    let name = if first.author_name.is_empty() { c.site.author_name.clone() } else { first.author_name.clone() };
    let head = Head {
        title: format!("{} · {}", name, c.site.name),
        description: format!("Articles by {} on {}", name, c.site.name),
        canonical: format!("{}/author/{}", c.base_url, slug),
        og_type: "profile",
        json_ld: vec![json!({ "@context": "https://schema.org", "@type": "ProfilePage", "name": name })],
        alternate_md: None,
    };
    let body = html! { h1 { (name) } @for p in &posts { (post_card(p)) } };
    cached(layout(&c, head, body))
}

async fn robots(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(c) = resolve(&state, &headers).await else { return not_found() };
    text("text/plain; charset=utf-8", format!("User-agent: *\nAllow: /\n\nSitemap: {}/sitemap.xml\n", c.base_url))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

async fn sitemap(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(c) = resolve(&state, &headers).await else { return not_found() };
    let posts = published_posts(&state, &c.site, 50_000, 0).await;
    let topics = sqlx::query_as::<_, Topic>("select * from topics where site_id = $1 and status = 'approved'").bind(c.site.id).fetch_all(&state.db).await.unwrap_or_default();
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    let latest = posts.first().map(|p| p.last_updated_at.format("%Y-%m-%d").to_string());
    xml.push_str(&format!("  <url><loc>{}/</loc>{}</url>\n", c.base_url, latest.map(|d| format!("<lastmod>{d}</lastmod>")).unwrap_or_default()));
    for p in &posts {
        xml.push_str(&format!("  <url><loc>{}/{}</loc><lastmod>{}</lastmod></url>\n", c.base_url, xml_escape(&p.slug), p.last_updated_at.format("%Y-%m-%d")));
    }
    for t in &topics {
        xml.push_str(&format!("  <url><loc>{}/topic/{}</loc></url>\n", c.base_url, xml_escape(&t.slug)));
    }
    for page in &c.pages {
        xml.push_str(&format!("  <url><loc>{}/{}</loc></url>\n", c.base_url, xml_escape(&page.slug)));
    }
    xml.push_str("</urlset>\n");
    text("application/xml; charset=utf-8", xml)
}

async fn rss(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(c) = resolve(&state, &headers).await else { return not_found() };
    let posts = published_posts(&state, &c.site, 50, 0).await;
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rss version=\"2.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\"><channel>\n<title>{}</title><link>{}/</link><description>{}</description>\n<atom:link href=\"{}/rss.xml\" rel=\"self\" type=\"application/rss+xml\"/>\n",
        xml_escape(&c.site.name),
        c.base_url,
        xml_escape(&c.site.product_description),
        c.base_url
    );
    for p in &posts {
        xml.push_str(&format!(
            "<item><title>{}</title><link>{}/{}</link><guid isPermaLink=\"true\">{}/{}</guid><pubDate>{}</pubDate><description>{}</description></item>\n",
            xml_escape(&p.title),
            c.base_url,
            p.slug,
            c.base_url,
            p.slug,
            p.published_at.map(|d| d.to_rfc2822()).unwrap_or_default(),
            xml_escape(&p.meta_description)
        ));
    }
    xml.push_str("</channel></rss>\n");
    text("application/rss+xml; charset=utf-8", xml)
}

async fn llms(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(c) = resolve(&state, &headers).await else { return not_found() };
    let posts = published_posts(&state, &c.site, 1000, 0).await;
    let mut out = format!("# {}\n\n", c.site.name);
    if !c.site.product_description.is_empty() {
        out.push_str(&format!("> {}\n\n", c.site.product_description.replace('\n', " ")));
    }
    out.push_str("Every post is also available as plain Markdown by adding .md to its URL.\n\n## Posts\n\n");
    for p in &posts {
        out.push_str(&format!("- [{}]({}/{}.md): {}\n", p.title, c.base_url, p.slug, p.meta_description));
    }
    if !c.pages.is_empty() {
        out.push_str("\n## Pages\n\n");
        for page in &c.pages {
            out.push_str(&format!("- [{}]({}/{})\n", page.title, c.base_url, page.slug));
        }
    }
    text("text/plain; charset=utf-8", out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_hides_citations_and_raw_html() {
        let html = render_markdown("Fast setup [[E2]].\n\n<script>alert(1)</script>\n");
        assert!(html.contains("<p>Fast setup.</p>"));
        assert!(!html.contains("<script>"));
    }

    #[test]
    fn faq_from_question_headings() {
        let pairs = faq_pairs("Intro.\n\n## What is a CRM?\n\nA CRM keeps notes [[E1]].\n\n## Setup\n\nText.\n");
        assert_eq!(pairs, vec![("What is a CRM?".to_string(), "A CRM keeps notes.".to_string())]);
    }
}
