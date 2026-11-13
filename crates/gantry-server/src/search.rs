//! Search across work items and pages. Results are filtered inside the engine to the projects
//! and spaces the caller may view; nothing is fetched and then hidden.
//!
//! Modes: `keyword` (full-text), `semantic` (vector similarity; needs an embedding provider)
//! and `hybrid` (both, merged by reciprocal rank fusion; the default when embeddings are on).

use crate::auth::Ctx;
use crate::authz;
use crate::error::{bad, AppError, AppResult};
use crate::http::Api;
use crate::storage::{chunk_text, SearchHit, SearchScope, VecMeta, HL_END, HL_START};
use crate::St;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashMap;
use uuid::Uuid;

/// Merge ranked lists with reciprocal rank fusion (k = 60).
pub fn rrf(lists: Vec<Vec<SearchHit>>, limit: usize) -> Vec<SearchHit> {
    let mut scores: HashMap<(String, Uuid), (f64, SearchHit)> = HashMap::new();
    for list in lists {
        for (rank, hit) in list.into_iter().enumerate() {
            let entry = scores.entry((hit.kind.clone(), hit.ref_id)).or_insert((0.0, hit));
            entry.0 += 1.0 / (60.0 + rank as f64 + 1.0);
        }
    }
    let mut out: Vec<SearchHit> = scores.into_values().map(|(s, mut h)| {
        h.score = s;
        h
    }).collect();
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.label.cmp(&b.label)));
    out.truncate(limit);
    out
}

/// Split a snippet on highlight markers into `[{text, hit}]` so clients never handle markup.
pub fn snippet_parts(snippet: &str) -> Vec<Value> {
    let mut parts = vec![];
    let mut hit = false;
    let mut cur = String::new();
    for c in snippet.chars() {
        if c == HL_START || c == HL_END {
            if !cur.is_empty() {
                parts.push(json!({"text": std::mem::take(&mut cur), "hit": hit}));
            }
            hit = c == HL_START;
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        parts.push(json!({"text": cur, "hit": hit}));
    }
    parts
}

pub fn hit_json(h: &SearchHit) -> Value {
    let plain: String = h.snippet.chars().filter(|c| *c != HL_START && *c != HL_END).collect();
    json!({"kind": h.kind, "id": h.ref_id, "label": h.label, "title": h.title, "url": h.url, "snippet": plain, "snippet_parts": snippet_parts(&h.snippet), "score": h.score})
}

pub async fn search(st: &St, ctx: &Ctx, q: &str, kinds: Vec<String>, mode: Option<&str>, limit: i64) -> AppResult<Vec<SearchHit>> {
    let q = q.trim();
    if q.chars().count() > 500 {
        return Err(bad("query is limited to 500 characters"));
    }
    if kinds.iter().any(|k| !["item", "page"].contains(&k.as_str())) {
        return Err(bad("kind must be item or page"));
    }
    let limit = limit.clamp(1, 50);
    let scope = SearchScope { projects: authz::visible_project_ids(st, ctx).await?, spaces: authz::visible_space_ids(st, ctx).await?, kinds, customer_visible_only: false };
    let mode = match (mode, &st.stores.embedder) {
        (None, Some(_)) => "hybrid",
        (None, None) => "keyword",
        (Some(m @ ("keyword" | "semantic" | "hybrid")), _) => m,
        (Some(_), _) => return Err(bad("mode must be keyword, semantic or hybrid")),
    };
    let keyword = if mode != "semantic" { st.stores.search.search(q, &scope, limit).await.map_err(AppError::Other)? } else { vec![] };
    if mode == "keyword" {
        return Ok(keyword);
    }
    let Some(embedder) = &st.stores.embedder else {
        return Err(bad("semantic search needs an embedding provider; ask an administrator to configure ai.embeddings"));
    };
    if q.is_empty() {
        return Ok(vec![]);
    }
    let qv = embedder.embed(&[q.to_string()]).await.map_err(|e| AppError::Upstream(e.to_string()))?.pop().unwrap_or_default();
    let floor = st.cfg.ai.embeddings.min_similarity;
    let semantic: Vec<SearchHit> = st.stores.vector.search(&qv, &scope, limit).await.map_err(AppError::Other)?.into_iter().filter(|h| h.score >= floor).collect();
    Ok(if mode == "semantic" { semantic } else { rrf(vec![keyword, semantic], limit as usize) })
}

/// Job handler: recompute embeddings for one item or page.
pub async fn embed_doc(st: &St, payload: &Value) -> anyhow::Result<()> {
    let Some(embedder) = &st.stores.embedder else { return Ok(()) };
    let kind = payload["kind"].as_str().unwrap_or_default().to_string();
    let id: Uuid = payload["id"].as_str().and_then(|s| s.parse().ok()).ok_or_else(|| anyhow::anyhow!("embed job without id"))?;
    let (meta, text) = match kind.as_str() {
        "item" => {
            let Some(r) = sqlx::query("SELECT i.project_id, p.key || '-' || i.number AS key, i.title, i.description, i.archived FROM items i JOIN projects p ON p.id = i.project_id WHERE i.id = $1").bind(id).fetch_optional(&st.db).await? else { return Ok(()) };
            if r.get::<bool, _>("archived") {
                return st.stores.vector.remove("item", id).await;
            }
            let key: String = r.get("key");
            let title: String = r.get("title");
            (VecMeta { kind, ref_id: id, project_id: Some(r.get("project_id")), space_id: None, url: format!("/i/{key}"), label: key, title: title.clone(), model: embedder.model() }, format!("{title}\n\n{}", r.get::<String, _>("description")))
        }
        "page" => {
            let Some(r) = sqlx::query("SELECT pg.space_id, pg.title, pg.content, pg.status, pg.archived, s.key FROM pages pg JOIN spaces s ON s.id = pg.space_id WHERE pg.id = $1").bind(id).fetch_optional(&st.db).await? else { return Ok(()) };
            if r.get::<bool, _>("archived") || r.get::<String, _>("status") != "published" {
                return st.stores.vector.remove("page", id).await;
            }
            let key: String = r.get("key");
            let title: String = r.get("title");
            (VecMeta { kind, ref_id: id, project_id: None, space_id: Some(r.get("space_id")), url: format!("/docs/{key}/{id}"), label: key, title: title.clone(), model: embedder.model() }, format!("{title}\n\n{}", r.get::<String, _>("content")))
        }
        other => anyhow::bail!("cannot embed kind {other:?}"),
    };
    let chunks: Vec<String> = chunk_text(&text, 1200).into_iter().take(64).collect();
    if chunks.is_empty() {
        return st.stores.vector.remove(&meta.kind, id).await;
    }
    let vectors = embedder.embed(&chunks).await?;
    st.stores.vector.upsert(&meta, &chunks.into_iter().zip(vectors).collect::<Vec<_>>()).await
}

/// Rebuild the search index (and queue embeddings) for everything. `gantry reindex`.
pub async fn reindex_all(st: &St) -> anyhow::Result<(usize, usize)> {
    let items: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM items").fetch_all(&st.db).await?;
    for id in &items {
        crate::items::index_item(st, *id).await;
    }
    let pages: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM pages").fetch_all(&st.db).await?;
    for id in &pages {
        crate::wiki::index_page(st, *id).await;
    }
    Ok((items.len(), pages.len()))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    kind: Option<String>,
    mode: Option<String>,
    limit: Option<i64>,
}

async fn h_search(State(st): State<St>, ctx: Ctx, Query(p): Query<SearchQuery>) -> AppResult<Json<Value>> {
    let kinds = p.kind.map(|k| k.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    let hits = search(&st, &ctx, &p.q, kinds, p.mode.as_deref(), p.limit.unwrap_or(20)).await?;
    Ok(Json(json!({"results": hits.iter().map(hit_json).collect::<Vec<_>>()})))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/search", "Search", "Search work items and pages the caller may see", get(h_search))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(label: &str) -> SearchHit {
        SearchHit { kind: "item".into(), ref_id: Uuid::from_u128(label.bytes().fold(7u128, |a, b| a * 131 + b as u128)), label: label.into(), url: String::new(), title: String::new(), snippet: String::new(), score: 0.0 }
    }

    #[test]
    fn fusion_rewards_agreement_between_lists() {
        let merged = rrf(vec![vec![hit("A"), hit("B"), hit("C")], vec![hit("C"), hit("B"), hit("D")]], 10);
        let order: Vec<&str> = merged.iter().map(|h| h.label.as_str()).collect();
        assert_eq!(order.len(), 4, "duplicates are merged");
        assert!(["B", "C"].contains(&order[0]) && ["B", "C"].contains(&order[1]), "items in both lists rank first: {order:?}");
        assert_eq!(rrf(vec![vec![hit("A"), hit("B")]], 1).len(), 1);
    }

    #[test]
    fn snippets_split_on_markers() {
        let s = format!("reset the {HL_START}VPN{HL_END} password");
        let parts = snippet_parts(&s);
        assert_eq!(parts, vec![json!({"text": "reset the ", "hit": false}), json!({"text": "VPN", "hit": true}), json!({"text": " password", "hit": false})]);
        assert!(snippet_parts("").is_empty());
        let h = SearchHit { snippet: s, ..hit("A") };
        assert_eq!(hit_json(&h)["snippet"], "reset the VPN password");
    }
}
