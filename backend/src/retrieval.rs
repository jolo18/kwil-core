//! Choosing the library entries a piece should be written from: topic links, a two-hop walk
//! of the knowledge graph, then Postgres full-text search.

use crate::models::{Entry, Piece};
use crate::state::AppState;
use crate::text::significant_words;
use anyhow::Result;
use uuid::Uuid;

pub const MAX_ENTRIES: usize = 40;
/// The coverage check needs at least this many relevant approved entries.
pub const MIN_RELEVANT: usize = 3;

/// Topic itself, its parent and its children.
async fn nearby_topics(state: &AppState, topic_id: Uuid) -> Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "select id from topics where id = $1
         union select parent_id from topics where id = $1 and parent_id is not null
         union select id from topics where parent_id = $1",
    )
    .bind(topic_id)
    .fetch_all(&state.db)
    .await?)
}

/// Entities within two hops of the starting entities, over approved edges in either direction.
async fn graph_entities(state: &AppState, site_id: Uuid, start: &[Uuid]) -> Result<Vec<Uuid>> {
    if start.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_scalar(
        "with recursive walk(id, depth) as (
             select unnest($2::uuid[]), 0
             union
             select case when e.from_entity = w.id then e.to_entity else e.from_entity end, w.depth + 1
             from walk w
             join entity_edges e on (e.from_entity = w.id or e.to_entity = w.id)
             where w.depth < 2 and e.status = 'approved' and e.site_id = $1
         )
         select distinct w.id from walk w join entities n on n.id = w.id where n.status = 'approved'",
    )
    .bind(site_id)
    .bind(start)
    .fetch_all(&state.db)
    .await?)
}

/// Entities the walk starts from: the piece's topic and any competitor it names.
async fn start_entities(state: &AppState, piece: &Piece, topics: &[Uuid]) -> Result<Vec<Uuid>> {
    let mut start: Vec<Uuid> = sqlx::query_scalar("select id from entities where site_id = $1 and topic_id = any($2) and status = 'approved'")
        .bind(piece.site_id)
        .bind(topics)
        .fetch_all(&state.db)
        .await?;
    let text = format!("{} {} {}", piece.title, piece.angle, piece.target_keyword.clone().unwrap_or_default()).to_lowercase();
    let competitors: Vec<(Uuid, String)> = sqlx::query_as("select id, name from entities where site_id = $1 and kind = 'competitor' and status = 'approved'")
        .bind(piece.site_id)
        .fetch_all(&state.db)
        .await?;
    for (id, name) in competitors {
        if text.contains(&name.to_lowercase()) {
            start.push(id);
        }
    }
    Ok(start)
}

pub async fn entries_for_piece(state: &AppState, piece: &Piece) -> Result<Vec<Entry>> {
    let mut ordered: Vec<Entry> = Vec::new();
    let push = |list: Vec<Entry>, ordered: &mut Vec<Entry>| {
        for e in list {
            if !ordered.iter().any(|o| o.id == e.id) {
                ordered.push(e);
            }
        }
    };

    let topics = match piece.topic_id {
        Some(t) => nearby_topics(state, t).await?,
        None => Vec::new(),
    };

    // 1. Entries tagged with the topic, its parent or its children.
    let tagged = sqlx::query_as::<_, Entry>(
        "select distinct e.* from entries e join entry_topics t on t.entry_id = e.id
         where e.site_id = $1 and e.status = 'approved' and t.topic_id = any($2) order by e.num",
    )
    .bind(piece.site_id)
    .bind(&topics)
    .fetch_all(&state.db)
    .await?;
    push(tagged, &mut ordered);

    // 2. Entries about entities within two hops of the topic (or a named competitor).
    let start = start_entities(state, piece, &topics).await?;
    let entities = graph_entities(state, piece.site_id, &start).await?;
    let about = sqlx::query_as::<_, Entry>(
        "select distinct e.* from entries e join entry_entities x on x.entry_id = e.id
         where e.site_id = $1 and e.status = 'approved' and x.entity_id = any($2) order by e.num",
    )
    .bind(piece.site_id)
    .bind(&entities)
    .fetch_all(&state.db)
    .await?;
    push(about, &mut ordered);

    // 3. Full-text search on the piece's title, angle and keyword.
    let words: Vec<String> = significant_words(&format!("{} {} {}", piece.title, piece.angle, piece.target_keyword.clone().unwrap_or_default()))
        .into_iter()
        .filter(|w| w.chars().all(|c| c.is_alphanumeric()))
        .collect();
    if !words.is_empty() {
        let query = words.join(" | ");
        let found = sqlx::query_as::<_, Entry>(
            "select * from entries where site_id = $1 and status = 'approved' and search @@ to_tsquery('english', $2)
             order by ts_rank(search, to_tsquery('english', $2)) desc, num limit 15",
        )
        .bind(piece.site_id)
        .bind(&query)
        .fetch_all(&state.db)
        .await?;
        push(found, &mut ordered);
    }

    ordered.truncate(MAX_ENTRIES);
    Ok(ordered)
}

/// What the library is missing for a piece, if anything.
pub fn coverage_gaps(relevant: &[Entry]) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !relevant.iter().any(|e| e.kind == "opinion" || e.kind == "positioning") {
        missing.push("opinion");
    }
    if relevant.len() < MIN_RELEVANT {
        missing.push("depth");
    }
    missing
}
