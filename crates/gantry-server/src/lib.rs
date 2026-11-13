//! Gantry: project management, issue tracking, service desk, knowledge base and wiki.
//!
//! One binary (`gantry`) provides the HTTP API (`serve`), the background worker (`worker`),
//! schema migration (`migrate`) and administration commands. See `docs/manual` for operations.

pub mod about;
pub mod accounts;
pub mod admin;
pub mod analytics;
pub mod audit;
pub mod auth;
pub mod authz;
pub mod config;
pub mod crypto;
pub mod desk;
pub mod error;
pub mod events;
pub mod http;
pub mod integrations;
pub mod items;
pub mod jobs;
pub mod mcp;
pub mod notify;
pub mod oidc;
pub mod packs;
pub mod pages;
pub mod planning;
pub mod projects;
pub mod realtime;
pub mod reference;
pub mod refs;
pub mod saml;
pub mod scim;
pub mod search;
pub mod settings;
pub mod storage;
pub mod teams;
pub mod wiki;

use std::sync::Arc;
use std::time::{Duration, Instant};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct AppState {
    pub cfg: config::Config,
    pub db: sqlx::PgPool,
    pub stores: storage::Stores,
    pub secrets: crypto::SecretBox,
    pub http: reqwest::Client,
    pub metrics: http::Metrics,
    pub limiter: http::RateLimiter,
    pub started: Instant,
    pub node_id: String,
    /// Open real-time connections and the rooms they follow.
    pub hub: realtime::Hub,
    /// Content-Security-Policy for the application's own pages.
    pub csp: String,
}

pub type St = Arc<AppState>;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Connect to PostgreSQL.
pub async fn connect_db(cfg: &config::Config) -> anyhow::Result<sqlx::PgPool> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(cfg.database.max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&cfg.database.url)
        .await
        .map_err(|e| anyhow::anyhow!("cannot connect to PostgreSQL: {e}"))?;
    Ok(pool)
}

/// Apply schema migrations, then (re)install built-in roles and the core pack. Safe to run repeatedly
/// and from several nodes at once: the migrator holds an advisory lock.
pub async fn migrate(db: &sqlx::PgPool) -> anyhow::Result<()> {
    MIGRATOR.run(db).await?;
    authz::seed_roles(db).await?;
    packs::install_core(db).await?;
    Ok(())
}

/// True when every migration compiled into this binary has been applied.
pub async fn schema_is_current(db: &sqlx::PgPool) -> bool {
    let latest = MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0);
    sqlx::query_scalar::<_, i64>("SELECT coalesce(max(version), 0) FROM _sqlx_migrations WHERE success").fetch_one(db).await.map(|v| v >= latest).unwrap_or(false)
}

impl AppState {
    pub async fn init(cfg: config::Config, db: sqlx::PgPool) -> anyhow::Result<St> {
        anyhow::ensure!(!cfg.security.secret_key.is_empty(), "security.secret_key is not set; generate one with `gantry config generate-key`");
        let secrets = crypto::SecretBox::new(&cfg.security.secret_key)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.integrations.timeout_seconds))
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(format!("Gantry/{VERSION}"))
            .build()?;
        let stores = storage::Stores::connect(&cfg, &db, &http).await?;
        let csp = http::content_security_policy(&cfg);
        let host = std::env::var("HOSTNAME").ok().or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string())).unwrap_or_else(|| "node".into());
        Ok(Arc::new(AppState {
            cfg,
            db,
            stores,
            secrets,
            http,
            metrics: http::Metrics::default(),
            limiter: http::RateLimiter::default(),
            started: Instant::now(),
            node_id: format!("{host}-{}", crypto::random_hex(3)),
            hub: realtime::Hub::default(),
            csp,
        }))
    }
}
