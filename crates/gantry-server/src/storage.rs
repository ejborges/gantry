//! Storage ports. Each kind of data sits behind a small trait so the default
//! PostgreSQL implementation can be swapped for a dedicated engine by configuration:
//!
//! | Port          | Default (core)                  | Dedicated option            |
//! |---------------|---------------------------------|-----------------------------|
//! | `BlobStore`   | filesystem, content-addressed   | any S3-compatible store     |
//! | `SearchIndex` | PostgreSQL full-text + trigram  | OpenSearch                  |
//! | `VectorStore` | PostgreSQL `real[]`, exact      | pgvector HNSW               |
//! | `Kv`          | PostgreSQL unlogged table       | Valkey (Redis protocol)     |
//! | `GraphStore`  | PostgreSQL recursive CTE        | (port only; see the manual) |
//!
//! To add an engine: implement the trait, add a backend name in `config.rs`, and
//! construct it in `Stores::connect`.

use crate::config::Config;
use crate::crypto;
use async_trait::async_trait;
use bytes::Bytes;
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use uuid::Uuid;

pub const HL_START: char = '\u{E000}';
pub const HL_END: char = '\u{E001}';

// ───────────────────────── blobs ─────────────────────────

#[async_trait]
pub trait BlobStore: Send + Sync {
    fn name(&self) -> &'static str;
    async fn put(&self, key: &str, data: Bytes, content_type: &str) -> anyhow::Result<()>;
    async fn get(&self, key: &str) -> anyhow::Result<Option<Bytes>>;
    async fn delete(&self, key: &str) -> anyhow::Result<()>;
    async fn health(&self) -> anyhow::Result<()>;
}

/// Blob keys are SHA-256 hex digests. Anything else is refused, which rules out path traversal.
fn valid_blob_key(key: &str) -> anyhow::Result<()> {
    anyhow::ensure!(key.len() == 64 && key.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')), "invalid blob key");
    Ok(())
}

pub struct FsBlob {
    root: PathBuf,
}

impl FsBlob {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    fn path(&self, key: &str) -> PathBuf {
        self.root.join(&key[0..2]).join(&key[2..4]).join(key)
    }
}

#[async_trait]
impl BlobStore for FsBlob {
    fn name(&self) -> &'static str {
        "fs"
    }
    async fn put(&self, key: &str, data: Bytes, _content_type: &str) -> anyhow::Result<()> {
        valid_blob_key(key)?;
        let path = self.path(key);
        if tokio::fs::try_exists(&path).await? {
            return Ok(());
        }
        tokio::fs::create_dir_all(path.parent().expect("blob path has a parent")).await?;
        let tmp = path.with_extension(format!("tmp-{}", crypto::random_hex(6)));
        tokio::fs::write(&tmp, &data).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }
    async fn get(&self, key: &str) -> anyhow::Result<Option<Bytes>> {
        valid_blob_key(key)?;
        match tokio::fs::read(self.path(key)).await {
            Ok(b) => Ok(Some(Bytes::from(b))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        valid_blob_key(key)?;
        match tokio::fs::remove_file(self.path(key)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
    async fn health(&self) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(&self.root).await?;
        let probe = self.root.join(format!(".probe-{}", crypto::random_hex(4)));
        tokio::fs::write(&probe, b"ok").await?;
        tokio::fs::remove_file(&probe).await?;
        Ok(())
    }
}

/// S3-compatible object storage (SeaweedFS, Garage, Ceph RGW, AWS S3), path-style, SigV4.
pub struct S3Blob {
    endpoint: url::Url,
    bucket: String,
    region: String,
    access_key: String,
    secret_key: String,
    http: reqwest::Client,
}

/// AWS Signature Version 4 `Authorization` header value for an S3 request.
/// `headers` must already include `host`, `x-amz-date` and `x-amz-content-sha256`.
pub fn sigv4_authorization(method: &str, path: &str, query: &str, headers: &[(String, String)], payload_sha256: &str, region: &str, access_key: &str, secret_key: &str) -> String {
    let mut hs: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string())).collect();
    hs.sort();
    let amz_date = hs.iter().find(|(k, _)| k == "x-amz-date").map(|(_, v)| v.clone()).unwrap_or_default();
    let date = &amz_date[..amz_date.len().min(8)];
    let canonical_headers: String = hs.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = hs.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let canonical = format!("{method}\n{path}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_sha256}");
    let scope = format!("{date}/{region}/s3/aws4_request");
    let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", crypto::sha256_hex(canonical.as_bytes()));
    let mut key = crypto::hmac_sha256(format!("AWS4{secret_key}").as_bytes(), date.as_bytes());
    for part in [region, "s3", "aws4_request"] {
        key = crypto::hmac_sha256(&key, part.as_bytes());
    }
    let signature = hex::encode(crypto::hmac_sha256(&key, to_sign.as_bytes()));
    format!("AWS4-HMAC-SHA256 Credential={access_key}/{scope},SignedHeaders={signed_headers},Signature={signature}")
}

impl S3Blob {
    pub fn new(cfg: &crate::config::Blob, http: reqwest::Client) -> anyhow::Result<Self> {
        let endpoint = url::Url::parse(&cfg.endpoint).map_err(|e| anyhow::anyhow!("storage.blob.endpoint: {e}"))?;
        anyhow::ensure!(!cfg.access_key.is_empty() && !cfg.secret_key.is_empty(), "storage.blob.access_key and secret_key are required for the s3 backend");
        Ok(Self { endpoint, bucket: cfg.bucket.clone(), region: cfg.region.clone(), access_key: cfg.access_key.clone(), secret_key: cfg.secret_key.clone(), http })
    }

    async fn send(&self, method: reqwest::Method, key: &str, body: Option<Bytes>, content_type: Option<&str>) -> anyhow::Result<reqwest::Response> {
        let base = self.endpoint.path().trim_end_matches('/');
        let path = if key.is_empty() { format!("{base}/{}", self.bucket) } else { format!("{base}/{}/{key}", self.bucket) };
        let host = match self.endpoint.port() {
            Some(p) => format!("{}:{p}", self.endpoint.host_str().unwrap_or_default()),
            None => self.endpoint.host_str().unwrap_or_default().to_string(),
        };
        let payload_hash = crypto::sha256_hex(body.as_deref().unwrap_or(&[]));
        let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let headers = vec![("host".to_string(), host), ("x-amz-content-sha256".to_string(), payload_hash.clone()), ("x-amz-date".to_string(), amz_date.clone())];
        let auth = sigv4_authorization(method.as_str(), &path, "", &headers, &payload_hash, &self.region, &self.access_key, &self.secret_key);
        let mut url = self.endpoint.clone();
        url.set_path(&path);
        let mut req = self.http.request(method, url).header("x-amz-content-sha256", payload_hash).header("x-amz-date", amz_date).header("authorization", auth);
        if let Some(ct) = content_type {
            req = req.header("content-type", ct);
        }
        if let Some(b) = body {
            req = req.body(b);
        }
        Ok(req.send().await?)
    }
}

#[async_trait]
impl BlobStore for S3Blob {
    fn name(&self) -> &'static str {
        "s3"
    }
    async fn put(&self, key: &str, data: Bytes, content_type: &str) -> anyhow::Result<()> {
        valid_blob_key(key)?;
        let r = self.send(reqwest::Method::PUT, key, Some(data), Some(content_type)).await?;
        anyhow::ensure!(r.status().is_success(), "object store PUT failed: {}", r.status());
        Ok(())
    }
    async fn get(&self, key: &str) -> anyhow::Result<Option<Bytes>> {
        valid_blob_key(key)?;
        let r = self.send(reqwest::Method::GET, key, None, None).await?;
        if r.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        anyhow::ensure!(r.status().is_success(), "object store GET failed: {}", r.status());
        Ok(Some(r.bytes().await?))
    }
    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        valid_blob_key(key)?;
        let r = self.send(reqwest::Method::DELETE, key, None, None).await?;
        anyhow::ensure!(r.status().is_success() || r.status() == reqwest::StatusCode::NOT_FOUND, "object store DELETE failed: {}", r.status());
        Ok(())
    }
    async fn health(&self) -> anyhow::Result<()> {
        let r = self.send(reqwest::Method::HEAD, "", None, None).await?;
        anyhow::ensure!(r.status().is_success(), "bucket check failed: {}", r.status());
        Ok(())
    }
}

// ───────────────────────── search ─────────────────────────

#[derive(Debug, Clone, Default)]
pub struct SearchDoc {
    pub kind: String,
    pub ref_id: Uuid,
    pub project_id: Option<Uuid>,
    pub space_id: Option<Uuid>,
    pub label: String,
    pub url: String,
    pub title: String,
    pub body: String,
    pub customer_visible: bool,
}

/// What the caller is allowed to see. Every search is filtered by it inside the engine.
#[derive(Debug, Clone, Default)]
pub struct SearchScope {
    pub projects: Vec<Uuid>,
    pub spaces: Vec<Uuid>,
    pub kinds: Vec<String>,
    pub customer_visible_only: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub kind: String,
    pub ref_id: Uuid,
    pub label: String,
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
}

#[async_trait]
pub trait SearchIndex: Send + Sync {
    fn name(&self) -> &'static str;
    async fn index(&self, doc: &SearchDoc) -> anyhow::Result<()>;
    async fn remove(&self, kind: &str, ref_id: Uuid) -> anyhow::Result<()>;
    async fn search(&self, query: &str, scope: &SearchScope, limit: i64) -> anyhow::Result<Vec<SearchHit>>;
    async fn health(&self) -> anyhow::Result<()>;
}

pub struct PgSearch {
    db: PgPool,
}

impl PgSearch {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

/// Escape `%`, `_` and `\` so user text can be used inside a LIKE pattern.
pub fn like_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[async_trait]
impl SearchIndex for PgSearch {
    fn name(&self) -> &'static str {
        "pg"
    }
    async fn index(&self, d: &SearchDoc) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO search_docs (kind, ref_id, project_id, space_id, label, url, title, body, customer_visible, updated_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9, now())
             ON CONFLICT (kind, ref_id) DO UPDATE SET project_id = EXCLUDED.project_id, space_id = EXCLUDED.space_id,
               label = EXCLUDED.label, url = EXCLUDED.url, title = EXCLUDED.title, body = EXCLUDED.body,
               customer_visible = EXCLUDED.customer_visible, updated_at = now()",
        )
        .bind(&d.kind)
        .bind(d.ref_id)
        .bind(d.project_id)
        .bind(d.space_id)
        .bind(&d.label)
        .bind(&d.url)
        .bind(&d.title)
        .bind(&d.body)
        .bind(d.customer_visible)
        .execute(&self.db)
        .await?;
        Ok(())
    }
    async fn remove(&self, kind: &str, ref_id: Uuid) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM search_docs WHERE kind = $1 AND ref_id = $2").bind(kind).bind(ref_id).execute(&self.db).await?;
        Ok(())
    }
    async fn search(&self, query: &str, scope: &SearchScope, limit: i64) -> anyhow::Result<Vec<SearchHit>> {
        // PostgreSQL text cannot hold NUL; control characters carry no search meaning.
        let cleaned: String = query.chars().filter(|c| !c.is_control()).collect();
        let q = cleaned.trim();
        if q.is_empty() {
            return Ok(vec![]);
        }
        let headline = format!("StartSel={HL_START}, StopSel={HL_END}, MaxFragments=1, MaxWords=24, MinWords=8");
        let rows = sqlx::query(
            "SELECT d.kind, d.ref_id, d.label, d.url, d.title,
                    ts_headline('english', left(d.body, 20000), q.tsq, $6) AS snippet,
                    (ts_rank_cd(d.tsv, q.tsq) + CASE WHEN d.label ILIKE $2 THEN 2 WHEN d.title ILIKE $3 THEN 0.5 ELSE 0 END)::float8 AS score
             FROM search_docs d, (SELECT websearch_to_tsquery('english', $1) AS tsq) q
             WHERE (d.tsv @@ q.tsq OR d.title ILIKE $3 OR d.label ILIKE $2)
               AND (d.project_id = ANY($4) OR d.space_id = ANY($5))
               AND (cardinality($7::text[]) = 0 OR d.kind = ANY($7))
               AND (NOT $8 OR d.customer_visible)
             ORDER BY score DESC, d.updated_at DESC
             LIMIT $9",
        )
        .bind(q)
        .bind(format!("{}%", like_escape(q)))
        .bind(format!("%{}%", like_escape(q)))
        .bind(&scope.projects)
        .bind(&scope.spaces)
        .bind(headline)
        .bind(&scope.kinds)
        .bind(scope.customer_visible_only)
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| SearchHit { kind: r.get("kind"), ref_id: r.get("ref_id"), label: r.get("label"), url: r.get("url"), title: r.get("title"), snippet: r.get("snippet"), score: r.get("score") })
            .collect())
    }
    async fn health(&self) -> anyhow::Result<()> {
        sqlx::query("SELECT 1 FROM search_docs LIMIT 1").fetch_optional(&self.db).await?;
        Ok(())
    }
}

/// OpenSearch adapter. Documents carry the same scope fields, and the scope filter is part of every query.
pub struct OpenSearch {
    base: String,
    index: String,
    username: String,
    password: String,
    http: reqwest::Client,
}

impl OpenSearch {
    pub fn new(cfg: &crate::config::Search, http: reqwest::Client) -> anyhow::Result<Self> {
        anyhow::ensure!(!cfg.url.is_empty(), "storage.search.url is required for the opensearch backend");
        Ok(Self { base: cfg.url.trim_end_matches('/').to_string(), index: cfg.index.clone(), username: cfg.username.clone(), password: cfg.password.clone(), http })
    }
    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let r = self.http.request(method, format!("{}/{}", self.base, path));
        if self.username.is_empty() {
            r
        } else {
            r.basic_auth(&self.username, Some(&self.password))
        }
    }
    /// The query body, exposed so tests can check the permission filter without a live cluster.
    pub fn query_body(query: &str, scope: &SearchScope, limit: i64) -> Value {
        let ids = |v: &[Uuid]| v.iter().map(|u| u.to_string()).collect::<Vec<_>>();
        let mut filter = vec![json!({"bool": {"should": [
            {"terms": {"project_id": ids(&scope.projects)}},
            {"terms": {"space_id": ids(&scope.spaces)}}
        ], "minimum_should_match": 1}})];
        if !scope.kinds.is_empty() {
            filter.push(json!({"terms": {"kind": scope.kinds}}));
        }
        if scope.customer_visible_only {
            filter.push(json!({"term": {"customer_visible": true}}));
        }
        json!({
            "size": limit.clamp(1, 100),
            "query": {"bool": {
                "must": [{"multi_match": {"query": query, "fields": ["label^5", "title^3", "body"], "type": "best_fields", "operator": "and"}}],
                "filter": filter
            }},
            "highlight": {"pre_tags": [HL_START.to_string()], "post_tags": [HL_END.to_string()], "fields": {"body": {"number_of_fragments": 1, "fragment_size": 180}}}
        })
    }
}

#[async_trait]
impl SearchIndex for OpenSearch {
    fn name(&self) -> &'static str {
        "opensearch"
    }
    async fn index(&self, d: &SearchDoc) -> anyhow::Result<()> {
        let body = json!({"kind": d.kind, "ref_id": d.ref_id, "project_id": d.project_id, "space_id": d.space_id, "label": d.label,
            "url": d.url, "title": d.title, "body": d.body, "customer_visible": d.customer_visible, "updated_at": chrono::Utc::now()});
        let r = self.req(reqwest::Method::PUT, &format!("{}/_doc/{}:{}", self.index, d.kind, d.ref_id)).json(&body).send().await?;
        anyhow::ensure!(r.status().is_success(), "opensearch index failed: {}", r.status());
        Ok(())
    }
    async fn remove(&self, kind: &str, ref_id: Uuid) -> anyhow::Result<()> {
        let r = self.req(reqwest::Method::DELETE, &format!("{}/_doc/{kind}:{ref_id}", self.index)).send().await?;
        anyhow::ensure!(r.status().is_success() || r.status() == reqwest::StatusCode::NOT_FOUND, "opensearch delete failed: {}", r.status());
        Ok(())
    }
    async fn search(&self, query: &str, scope: &SearchScope, limit: i64) -> anyhow::Result<Vec<SearchHit>> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }
        let r = self.req(reqwest::Method::POST, &format!("{}/_search", self.index)).json(&Self::query_body(query, scope, limit)).send().await?;
        anyhow::ensure!(r.status().is_success(), "opensearch search failed: {}", r.status());
        let v: Value = r.json().await?;
        let hits = v["hits"]["hits"].as_array().cloned().unwrap_or_default();
        Ok(hits
            .iter()
            .filter_map(|h| {
                let s = &h["_source"];
                Some(SearchHit {
                    kind: s["kind"].as_str()?.to_string(),
                    ref_id: s["ref_id"].as_str()?.parse().ok()?,
                    label: s["label"].as_str().unwrap_or_default().to_string(),
                    url: s["url"].as_str().unwrap_or_default().to_string(),
                    title: s["title"].as_str().unwrap_or_default().to_string(),
                    snippet: h["highlight"]["body"][0].as_str().unwrap_or_default().to_string(),
                    score: h["_score"].as_f64().unwrap_or(0.0),
                })
            })
            .collect())
    }
    async fn health(&self) -> anyhow::Result<()> {
        let r = self.req(reqwest::Method::GET, "_cluster/health").send().await?;
        anyhow::ensure!(r.status().is_success(), "opensearch health check failed: {}", r.status());
        Ok(())
    }
}

// ───────────────────────── vectors and embeddings ─────────────────────────

#[async_trait]
pub trait Embedder: Send + Sync {
    fn model(&self) -> String;
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>>;
}

fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x = (*x as f64 / norm) as f32;
        }
    }
}

/// Offline feature-hashing embedder. It captures word and word-pair overlap, not meaning;
/// it exists so vector search works with no model server and so the vector path is testable.
pub struct HashEmbedder {
    pub dims: usize,
}

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

impl HashEmbedder {
    pub fn embed_one(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.dims];
        let lower = text.to_lowercase();
        let tokens: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|t| t.len() > 1).collect();
        let mut add = |feature: &str, weight: f32| {
            let h = fnv1a(feature);
            let idx = (h % self.dims as u64) as usize;
            v[idx] += if (h >> 63) & 1 == 1 { -weight } else { weight };
        };
        for (i, t) in tokens.iter().enumerate() {
            add(t, 1.0);
            if let Some(next) = tokens.get(i + 1) {
                add(&format!("{t} {next}"), 0.5);
            }
        }
        normalize(&mut v);
        v
    }
}

#[async_trait]
impl Embedder for HashEmbedder {
    fn model(&self) -> String {
        format!("hash-{}", self.dims)
    }
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.embed_one(t)).collect())
    }
}

/// Any server with an OpenAI-style `POST {base_url}/embeddings` endpoint.
pub struct OpenAiCompatEmbedder {
    base_url: String,
    model: String,
    api_key: String,
    http: reqwest::Client,
}

#[async_trait]
impl Embedder for OpenAiCompatEmbedder {
    fn model(&self) -> String {
        self.model.clone()
    }
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut req = self.http.post(format!("{}/embeddings", self.base_url.trim_end_matches('/'))).json(&json!({"model": self.model, "input": texts}));
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let r = req.send().await?;
        anyhow::ensure!(r.status().is_success(), "embedding request failed: {}", r.status());
        let v: Value = r.json().await?;
        let data = v["data"].as_array().ok_or_else(|| anyhow::anyhow!("embedding response has no data array"))?;
        anyhow::ensure!(data.len() == texts.len(), "embedding response count mismatch");
        let mut out = Vec::with_capacity(data.len());
        for d in data {
            let mut e: Vec<f32> = d["embedding"].as_array().ok_or_else(|| anyhow::anyhow!("missing embedding"))?.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect();
            normalize(&mut e);
            out.push(e);
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Default)]
pub struct VecMeta {
    pub kind: String,
    pub ref_id: Uuid,
    pub project_id: Option<Uuid>,
    pub space_id: Option<Uuid>,
    pub label: String,
    pub url: String,
    pub title: String,
    pub model: String,
}

#[async_trait]
pub trait VectorStore: Send + Sync {
    fn name(&self) -> &'static str;
    async fn upsert(&self, meta: &VecMeta, chunks: &[(String, Vec<f32>)]) -> anyhow::Result<()>;
    async fn remove(&self, kind: &str, ref_id: Uuid) -> anyhow::Result<()>;
    async fn search(&self, query: &[f32], scope: &SearchScope, limit: i64) -> anyhow::Result<Vec<SearchHit>>;
}

fn vec_literal(v: &[f32]) -> String {
    let mut s = String::with_capacity(v.len() * 8 + 2);
    s.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("{x}"));
    }
    s.push(']');
    s
}

/// Keep the best-scoring chunk per document.
fn best_per_doc(rows: Vec<SearchHit>, limit: i64) -> Vec<SearchHit> {
    let mut seen = std::collections::HashSet::new();
    rows.into_iter().filter(|h| seen.insert((h.kind.clone(), h.ref_id))).take(limit.max(1) as usize).collect()
}

/// Exact nearest-neighbor search over `real[]` columns. Fine up to roughly 10^5 chunks.
pub struct PgArrayVector {
    db: PgPool,
}

/// Two jobs can index the same object at the same moment (it was changed twice in a row).
/// Each replaces the object's rows, so they take turns: without this, both find nothing to
/// delete and the second insert collides with the first.
async fn one_writer(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, m: &VecMeta) -> anyhow::Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1 || ':' || $2::text, 0))").bind(&m.kind).bind(m.ref_id).execute(&mut **tx).await?;
    Ok(())
}

#[async_trait]
impl VectorStore for PgArrayVector {
    fn name(&self) -> &'static str {
        "pg_array"
    }
    async fn upsert(&self, m: &VecMeta, chunks: &[(String, Vec<f32>)]) -> anyhow::Result<()> {
        let mut tx = self.db.begin().await?;
        one_writer(&mut tx, m).await?;
        sqlx::query("DELETE FROM embeddings WHERE kind = $1 AND ref_id = $2").bind(&m.kind).bind(m.ref_id).execute(&mut *tx).await?;
        for (i, (content, emb)) in chunks.iter().enumerate() {
            sqlx::query("INSERT INTO embeddings (kind, ref_id, chunk, project_id, space_id, label, url, title, content, model, embedding) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
                .bind(&m.kind)
                .bind(m.ref_id)
                .bind(i as i32)
                .bind(m.project_id)
                .bind(m.space_id)
                .bind(&m.label)
                .bind(&m.url)
                .bind(&m.title)
                .bind(content)
                .bind(&m.model)
                .bind(emb)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    async fn remove(&self, kind: &str, ref_id: Uuid) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM embeddings WHERE kind = $1 AND ref_id = $2").bind(kind).bind(ref_id).execute(&self.db).await?;
        Ok(())
    }
    async fn search(&self, query: &[f32], scope: &SearchScope, limit: i64) -> anyhow::Result<Vec<SearchHit>> {
        let rows = sqlx::query(
            "SELECT kind, ref_id, label, url, title, left(content, 240) AS snippet, vec_dot(embedding, $1) AS score
             FROM embeddings
             WHERE (project_id = ANY($2) OR space_id = ANY($3)) AND cardinality(embedding) = cardinality($1)
               AND (cardinality($4::text[]) = 0 OR kind = ANY($4))
             ORDER BY score DESC LIMIT $5",
        )
        .bind(query)
        .bind(&scope.projects)
        .bind(&scope.spaces)
        .bind(&scope.kinds)
        .bind(limit.clamp(1, 100) * 4)
        .fetch_all(&self.db)
        .await?;
        let hits = rows.into_iter().map(|r| SearchHit { kind: r.get("kind"), ref_id: r.get("ref_id"), label: r.get("label"), url: r.get("url"), title: r.get("title"), snippet: r.get("snippet"), score: r.get("score") }).collect();
        Ok(best_per_doc(hits, limit))
    }
}

/// Approximate nearest-neighbor search with the pgvector extension (HNSW, cosine distance).
pub struct PgVector {
    db: PgPool,
    dims: usize,
}

impl PgVector {
    pub async fn connect(db: PgPool, dims: usize) -> anyhow::Result<Self> {
        sqlx::query("CREATE EXTENSION IF NOT EXISTS vector").execute(&db).await.map_err(|e| anyhow::anyhow!("pgvector extension is not available: {e}"))?;
        sqlx::query(&format!(
            "CREATE TABLE IF NOT EXISTS embeddings_pgv (kind text NOT NULL, ref_id uuid NOT NULL, chunk integer NOT NULL, project_id uuid, space_id uuid,
               label text NOT NULL DEFAULT '', url text NOT NULL DEFAULT '', title text NOT NULL DEFAULT '', content text NOT NULL, model text NOT NULL,
               embedding vector({dims}) NOT NULL, PRIMARY KEY (kind, ref_id, chunk))"
        ))
        .execute(&db)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS embeddings_pgv_hnsw ON embeddings_pgv USING hnsw (embedding vector_cosine_ops)").execute(&db).await?;
        Ok(Self { db, dims })
    }
}

#[async_trait]
impl VectorStore for PgVector {
    fn name(&self) -> &'static str {
        "pgvector"
    }
    async fn upsert(&self, m: &VecMeta, chunks: &[(String, Vec<f32>)]) -> anyhow::Result<()> {
        let mut tx = self.db.begin().await?;
        one_writer(&mut tx, m).await?;
        sqlx::query("DELETE FROM embeddings_pgv WHERE kind = $1 AND ref_id = $2").bind(&m.kind).bind(m.ref_id).execute(&mut *tx).await?;
        for (i, (content, emb)) in chunks.iter().enumerate() {
            anyhow::ensure!(emb.len() == self.dims, "embedding has {} dimensions, the index expects {}", emb.len(), self.dims);
            sqlx::query("INSERT INTO embeddings_pgv (kind, ref_id, chunk, project_id, space_id, label, url, title, content, model, embedding) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11::text::vector)")
                .bind(&m.kind)
                .bind(m.ref_id)
                .bind(i as i32)
                .bind(m.project_id)
                .bind(m.space_id)
                .bind(&m.label)
                .bind(&m.url)
                .bind(&m.title)
                .bind(content)
                .bind(&m.model)
                .bind(vec_literal(emb))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    async fn remove(&self, kind: &str, ref_id: Uuid) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM embeddings_pgv WHERE kind = $1 AND ref_id = $2").bind(kind).bind(ref_id).execute(&self.db).await?;
        Ok(())
    }
    async fn search(&self, query: &[f32], scope: &SearchScope, limit: i64) -> anyhow::Result<Vec<SearchHit>> {
        let rows = sqlx::query(
            "SELECT kind, ref_id, label, url, title, left(content, 240) AS snippet, (1 - (embedding <=> $1::text::vector))::float8 AS score
             FROM embeddings_pgv
             WHERE (project_id = ANY($2) OR space_id = ANY($3)) AND (cardinality($4::text[]) = 0 OR kind = ANY($4))
             ORDER BY embedding <=> $1::text::vector LIMIT $5",
        )
        .bind(vec_literal(query))
        .bind(&scope.projects)
        .bind(&scope.spaces)
        .bind(&scope.kinds)
        .bind(limit.clamp(1, 100) * 4)
        .fetch_all(&self.db)
        .await?;
        let hits = rows.into_iter().map(|r| SearchHit { kind: r.get("kind"), ref_id: r.get("ref_id"), label: r.get("label"), url: r.get("url"), title: r.get("title"), snippet: r.get("snippet"), score: r.get("score") }).collect();
        Ok(best_per_doc(hits, limit))
    }
}

/// Split text into overlapping chunks on paragraph boundaries for embedding.
pub fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut cur = String::new();
    for para in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        if !cur.is_empty() && cur.chars().count() + para.chars().count() + 2 > max_chars {
            chunks.push(std::mem::take(&mut cur));
        }
        if para.chars().count() > max_chars {
            let chars: Vec<char> = para.chars().collect();
            for piece in chars.chunks(max_chars) {
                chunks.push(piece.iter().collect());
            }
            continue;
        }
        if !cur.is_empty() {
            cur.push_str("\n\n");
        }
        cur.push_str(para);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

// ───────────────────────── key-value ─────────────────────────

#[async_trait]
pub trait Kv: Send + Sync {
    fn name(&self) -> &'static str;
    /// Increment a counter that expires `ttl_secs` after its first increment. Returns the new value.
    async fn incr(&self, key: &str, ttl_secs: i64) -> anyhow::Result<i64>;
    async fn get(&self, key: &str) -> anyhow::Result<Option<String>>;
    async fn set(&self, key: &str, value: &str, ttl_secs: i64) -> anyhow::Result<()>;
    async fn del(&self, key: &str) -> anyhow::Result<()>;
}

pub struct PgKv {
    db: PgPool,
}

#[async_trait]
impl Kv for PgKv {
    fn name(&self) -> &'static str {
        "pg"
    }
    async fn incr(&self, key: &str, ttl_secs: i64) -> anyhow::Result<i64> {
        let n: i64 = sqlx::query_scalar(
            "INSERT INTO kv (key, value, expires_at) VALUES ($1, '1', now() + make_interval(secs => $2))
             ON CONFLICT (key) DO UPDATE SET
               value = CASE WHEN kv.expires_at < now() THEN '1' ELSE (kv.value::bigint + 1)::text END,
               expires_at = CASE WHEN kv.expires_at < now() THEN EXCLUDED.expires_at ELSE kv.expires_at END
             RETURNING value::bigint",
        )
        .bind(key)
        .bind(ttl_secs as f64)
        .fetch_one(&self.db)
        .await?;
        Ok(n)
    }
    async fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT value FROM kv WHERE key = $1 AND expires_at > now()").bind(key).fetch_optional(&self.db).await?)
    }
    async fn set(&self, key: &str, value: &str, ttl_secs: i64) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO kv (key, value, expires_at) VALUES ($1, $2, now() + make_interval(secs => $3)) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, expires_at = EXCLUDED.expires_at")
            .bind(key)
            .bind(value)
            .bind(ttl_secs as f64)
            .execute(&self.db)
            .await?;
        Ok(())
    }
    async fn del(&self, key: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM kv WHERE key = $1").bind(key).execute(&self.db).await?;
        Ok(())
    }
}

/// Minimal Valkey / Redis client (RESP2) covering exactly the commands the `Kv` port needs.
pub struct ValkeyKv {
    addr: String,
    password: Option<String>,
    database: Option<String>,
    conn: tokio::sync::Mutex<Option<BufReader<TcpStream>>>,
}

#[derive(Debug, PartialEq)]
enum Resp {
    Simple(String),
    Int(i64),
    Bulk(Option<Vec<u8>>),
}

impl ValkeyKv {
    pub fn new(url: &str) -> anyhow::Result<Self> {
        let u = url::Url::parse(url).map_err(|e| anyhow::anyhow!("storage.kv.url: {e}"))?;
        anyhow::ensure!(matches!(u.scheme(), "valkey" | "redis"), "storage.kv.url must start with valkey:// or redis://");
        let host = u.host_str().ok_or_else(|| anyhow::anyhow!("storage.kv.url has no host"))?;
        let db = u.path().trim_start_matches('/');
        Ok(Self {
            addr: format!("{host}:{}", u.port().unwrap_or(6379)),
            password: u.password().map(str::to_string),
            database: (!db.is_empty()).then(|| db.to_string()),
            conn: tokio::sync::Mutex::new(None),
        })
    }

    async fn exchange(conn: &mut BufReader<TcpStream>, args: &[&str]) -> anyhow::Result<Resp> {
        let mut out = format!("*{}\r\n", args.len()).into_bytes();
        for a in args {
            out.extend(format!("${}\r\n", a.len()).as_bytes());
            out.extend(a.as_bytes());
            out.extend(b"\r\n");
        }
        conn.get_mut().write_all(&out).await?;
        let mut line = String::new();
        conn.read_line(&mut line).await?;
        let line = line.trim_end_matches(['\r', '\n']);
        anyhow::ensure!(!line.is_empty(), "connection closed by key-value server");
        let (tag, rest) = line.split_at(1);
        match tag {
            "+" => Ok(Resp::Simple(rest.to_string())),
            "-" => anyhow::bail!("key-value server error: {rest}"),
            ":" => Ok(Resp::Int(rest.parse()?)),
            "$" => {
                let len: i64 = rest.parse()?;
                if len < 0 {
                    return Ok(Resp::Bulk(None));
                }
                let mut buf = vec![0u8; len as usize + 2];
                conn.read_exact(&mut buf).await?;
                buf.truncate(len as usize);
                Ok(Resp::Bulk(Some(buf)))
            }
            other => anyhow::bail!("unexpected reply type {other:?} from key-value server"),
        }
    }

    async fn cmd(&self, args: &[&str]) -> anyhow::Result<Resp> {
        let mut guard = self.conn.lock().await;
        for attempt in 0..2 {
            if guard.is_none() {
                let mut c = BufReader::new(TcpStream::connect(&self.addr).await?);
                if let Some(pw) = &self.password {
                    Self::exchange(&mut c, &["AUTH", pw]).await?;
                }
                if let Some(db) = &self.database {
                    Self::exchange(&mut c, &["SELECT", db]).await?;
                }
                *guard = Some(c);
            }
            match Self::exchange(guard.as_mut().expect("connection present"), args).await {
                Ok(r) => return Ok(r),
                Err(e) if attempt == 0 && !e.to_string().starts_with("key-value server error") => {
                    *guard = None;
                }
                Err(e) => return Err(e),
            }
        }
        anyhow::bail!("key-value server unavailable")
    }
}

#[async_trait]
impl Kv for ValkeyKv {
    fn name(&self) -> &'static str {
        "valkey"
    }
    async fn incr(&self, key: &str, ttl_secs: i64) -> anyhow::Result<i64> {
        let ttl = ttl_secs.max(1).to_string();
        self.cmd(&["SET", key, "0", "EX", &ttl, "NX"]).await?;
        match self.cmd(&["INCR", key]).await? {
            Resp::Int(n) => Ok(n),
            other => anyhow::bail!("unexpected INCR reply {other:?}"),
        }
    }
    async fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        match self.cmd(&["GET", key]).await? {
            Resp::Bulk(b) => Ok(b.map(|v| String::from_utf8_lossy(&v).into_owned())),
            other => anyhow::bail!("unexpected GET reply {other:?}"),
        }
    }
    async fn set(&self, key: &str, value: &str, ttl_secs: i64) -> anyhow::Result<()> {
        let ttl = ttl_secs.max(1).to_string();
        self.cmd(&["SET", key, value, "EX", &ttl]).await?;
        Ok(())
    }
    async fn del(&self, key: &str) -> anyhow::Result<()> {
        self.cmd(&["DEL", key]).await?;
        Ok(())
    }
}

// ───────────────────────── graph ─────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Edge {
    pub id: Uuid,
    pub src_type: String,
    pub src_id: String,
    pub rel: String,
    pub dst_type: String,
    pub dst_id: String,
    pub meta: Value,
}

#[async_trait]
pub trait GraphStore: Send + Sync {
    fn name(&self) -> &'static str;
    async fn add_edge(&self, src: (&str, &str), rel: &str, dst: (&str, &str), meta: Value, by: Option<Uuid>) -> anyhow::Result<Uuid>;
    async fn remove_edge(&self, id: Uuid) -> anyhow::Result<bool>;
    async fn get_edge(&self, id: Uuid) -> anyhow::Result<Option<Edge>>;
    /// Edges touching `node` in either direction.
    async fn edges_of(&self, node: (&str, &str)) -> anyhow::Result<Vec<Edge>>;
    /// Every edge reachable from `node` within `depth` hops, ignoring direction.
    async fn neighborhood(&self, node: (&str, &str), depth: i32) -> anyhow::Result<Vec<Edge>>;
}

pub struct PgGraph {
    db: PgPool,
}

#[async_trait]
impl GraphStore for PgGraph {
    fn name(&self) -> &'static str {
        "pg"
    }
    async fn add_edge(&self, src: (&str, &str), rel: &str, dst: (&str, &str), meta: Value, by: Option<Uuid>) -> anyhow::Result<Uuid> {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO links (src_type, src_id, rel, dst_type, dst_id, meta, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT (src_type, src_id, rel, dst_type, dst_id) DO UPDATE SET meta = EXCLUDED.meta RETURNING id",
        )
        .bind(src.0)
        .bind(src.1)
        .bind(rel)
        .bind(dst.0)
        .bind(dst.1)
        .bind(meta)
        .bind(by)
        .fetch_one(&self.db)
        .await?;
        Ok(id)
    }
    async fn remove_edge(&self, id: Uuid) -> anyhow::Result<bool> {
        Ok(sqlx::query("DELETE FROM links WHERE id = $1").bind(id).execute(&self.db).await?.rows_affected() > 0)
    }
    async fn get_edge(&self, id: Uuid) -> anyhow::Result<Option<Edge>> {
        Ok(sqlx::query_as("SELECT id, src_type, src_id, rel, dst_type, dst_id, meta FROM links WHERE id = $1").bind(id).fetch_optional(&self.db).await?)
    }
    async fn edges_of(&self, node: (&str, &str)) -> anyhow::Result<Vec<Edge>> {
        Ok(sqlx::query_as(
            "SELECT id, src_type, src_id, rel, dst_type, dst_id, meta FROM links
             WHERE (src_type = $1 AND src_id = $2) OR (dst_type = $1 AND dst_id = $2) ORDER BY created_at",
        )
        .bind(node.0)
        .bind(node.1)
        .fetch_all(&self.db)
        .await?)
    }
    async fn neighborhood(&self, node: (&str, &str), depth: i32) -> anyhow::Result<Vec<Edge>> {
        Ok(sqlx::query_as(
            "WITH RECURSIVE frontier (ntype, nid, depth) AS (
                 SELECT $1::text, $2::text, 0
               UNION
                 SELECT CASE WHEN l.src_type = f.ntype AND l.src_id = f.nid THEN l.dst_type ELSE l.src_type END,
                        CASE WHEN l.src_type = f.ntype AND l.src_id = f.nid THEN l.dst_id ELSE l.src_id END,
                        f.depth + 1
                 FROM frontier f
                 JOIN links l ON (l.src_type = f.ntype AND l.src_id = f.nid) OR (l.dst_type = f.ntype AND l.dst_id = f.nid)
                 WHERE f.depth < $3
             )
             SELECT DISTINCT l.id, l.src_type, l.src_id, l.rel, l.dst_type, l.dst_id, l.meta
             FROM links l
             JOIN (SELECT DISTINCT ntype, nid FROM frontier WHERE depth < $3) f
               ON (l.src_type = f.ntype AND l.src_id = f.nid) OR (l.dst_type = f.ntype AND l.dst_id = f.nid)
             LIMIT 1000",
        )
        .bind(node.0)
        .bind(node.1)
        .bind(depth.clamp(1, 6))
        .fetch_all(&self.db)
        .await?)
    }
}

// ───────────────────────── wiring ─────────────────────────

pub struct Stores {
    pub blob: Arc<dyn BlobStore>,
    pub search: Arc<dyn SearchIndex>,
    pub vector: Arc<dyn VectorStore>,
    pub kv: Arc<dyn Kv>,
    pub graph: Arc<dyn GraphStore>,
    pub embedder: Option<Arc<dyn Embedder>>,
}

impl Stores {
    pub async fn connect(cfg: &Config, db: &PgPool, http: &reqwest::Client) -> anyhow::Result<Stores> {
        let blob: Arc<dyn BlobStore> = match cfg.storage.blob.backend.as_str() {
            "s3" => Arc::new(S3Blob::new(&cfg.storage.blob, http.clone())?),
            _ => Arc::new(FsBlob::new(cfg.storage.blob.path.clone())),
        };
        let search: Arc<dyn SearchIndex> = match cfg.storage.search.backend.as_str() {
            "opensearch" => Arc::new(OpenSearch::new(&cfg.storage.search, http.clone())?),
            _ => Arc::new(PgSearch::new(db.clone())),
        };
        let e = &cfg.ai.embeddings;
        let embedder: Option<Arc<dyn Embedder>> = match e.provider.as_str() {
            "hash" => Some(Arc::new(HashEmbedder { dims: e.dimensions })),
            "openai_compatible" => {
                anyhow::ensure!(!e.base_url.is_empty() && !e.model.is_empty(), "ai.embeddings.base_url and model are required");
                Some(Arc::new(OpenAiCompatEmbedder { base_url: e.base_url.clone(), model: e.model.clone(), api_key: e.api_key.clone(), http: http.clone() }))
            }
            _ => None,
        };
        let vector: Arc<dyn VectorStore> = match cfg.storage.vector.backend.as_str() {
            "pgvector" => Arc::new(PgVector::connect(db.clone(), e.dimensions).await?),
            _ => Arc::new(PgArrayVector { db: db.clone() }),
        };
        let kv: Arc<dyn Kv> = match cfg.storage.kv.backend.as_str() {
            "valkey" => Arc::new(ValkeyKv::new(&cfg.storage.kv.url)?),
            _ => Arc::new(PgKv { db: db.clone() }),
        };
        Ok(Stores { blob, search, vector, kv, graph: Arc::new(PgGraph { db: db.clone() }), embedder })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigv4_matches_aws_documented_example() {
        // "GET Object" example from the Amazon S3 Signature Version 4 documentation.
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let headers = vec![
            ("host".to_string(), "examplebucket.s3.amazonaws.com".to_string()),
            ("range".to_string(), "bytes=0-9".to_string()),
            ("x-amz-content-sha256".to_string(), empty.to_string()),
            ("x-amz-date".to_string(), "20130524T000000Z".to_string()),
        ];
        let auth = sigv4_authorization("GET", "/test.txt", "", &headers, empty, "us-east-1", "AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn blob_keys_must_be_sha256_hex() {
        assert!(valid_blob_key(&"a".repeat(64)).is_ok());
        assert!(valid_blob_key("../../etc/passwd").is_err());
        assert!(valid_blob_key(&"A".repeat(64)).is_err());
        assert!(valid_blob_key(&"a".repeat(63)).is_err());
    }

    #[tokio::test]
    async fn fs_blob_round_trip() {
        let dir = std::env::temp_dir().join(format!("gantry-blob-{}", crypto::random_hex(6)));
        let store = FsBlob::new(dir.clone());
        store.health().await.unwrap();
        let data = Bytes::from_static(b"hello");
        let key = crypto::sha256_hex(&data);
        store.put(&key, data.clone(), "text/plain").await.unwrap();
        store.put(&key, data.clone(), "text/plain").await.unwrap();
        assert_eq!(store.get(&key).await.unwrap(), Some(data));
        store.delete(&key).await.unwrap();
        assert_eq!(store.get(&key).await.unwrap(), None);
        assert!(store.get("../x").await.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn like_escape_neutralises_wildcards() {
        assert_eq!(like_escape("50%_off\\"), "50\\%\\_off\\\\");
    }

    #[test]
    fn hash_embedder_is_deterministic_and_unit_length() {
        let e = HashEmbedder { dims: 128 };
        let a = e.embed_one("Reset the VPN password for a contractor");
        let b = e.embed_one("Reset the VPN password for a contractor");
        assert_eq!(a, b);
        let norm: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4);
        let dot = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(p, q)| p * q).sum::<f32>();
        let related = e.embed_one("How do I reset my VPN password?");
        let unrelated = e.embed_one("Quarterly procurement budget forecast");
        assert!(dot(&a, &related) > dot(&a, &unrelated));
    }

    #[test]
    fn chunking_respects_limit_and_keeps_all_text() {
        let text = format!("{}\n\n{}\n\n{}", "a".repeat(300), "b".repeat(300), "c".repeat(1200));
        let chunks = chunk_text(&text, 500);
        assert!(chunks.iter().all(|c| c.chars().count() <= 500));
        assert_eq!(chunks.iter().map(|c| c.chars().filter(|ch| ch.is_alphabetic()).count()).sum::<usize>(), 1800);
        assert!(chunk_text("  \n\n ", 100).is_empty());
    }

    #[test]
    fn opensearch_query_always_carries_scope_filter() {
        let scope = SearchScope { projects: vec![Uuid::nil()], spaces: vec![], kinds: vec!["page".into()], customer_visible_only: true };
        let body = OpenSearch::query_body("vpn", &scope, 10);
        let filter = body["query"]["bool"]["filter"].as_array().unwrap();
        assert_eq!(filter.len(), 3);
        assert_eq!(filter[0]["bool"]["minimum_should_match"], 1);
        assert_eq!(filter[0]["bool"]["should"][0]["terms"]["project_id"][0], Uuid::nil().to_string());
        assert_eq!(filter[2]["term"]["customer_visible"], true);
    }

    #[test]
    fn valkey_url_parsing() {
        let v = ValkeyKv::new("valkey://:s3cret@cache.internal:6380/2").unwrap();
        assert_eq!(v.addr, "cache.internal:6380");
        assert_eq!(v.password.as_deref(), Some("s3cret"));
        assert_eq!(v.database.as_deref(), Some("2"));
        assert!(ValkeyKv::new("http://x").is_err());
    }
}
